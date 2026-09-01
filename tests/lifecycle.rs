//! Cucumber-проверка публичного lifecycle API и process-контрактов lifecycle CLI.

use std::collections::VecDeque;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Barrier, Mutex};
use std::time::{Duration, Instant};

use cucumber::{World, given, then, when};
use orchestrator::{
    AgentExit, AgentInput, AgentRegistry, AgentRunRequest, AttemptControl, LifecycleCommand,
    LifecycleReporter, ProcessEnvironment, execute_lifecycle,
};
use tempfile::TempDir;

#[derive(Debug, Default, World)]
struct LifecycleWorld {
    root: Option<TempDir>,
    run_id: Option<String>,
    observed: Option<Observed>,
    calls: Vec<Call>,
    process_agent: Option<PathBuf>,
    process_gate: Option<PathBuf>,
    process_ready: Option<PathBuf>,
    control_code: Option<PathBuf>,
    max_concurrency: usize,
}

#[derive(Debug)]
struct Observed {
    exit_code: u8,
    lines: Vec<String>,
    error: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Call {
    step_id: String,
    attempt: u64,
    resume_session: Option<String>,
    inputs: Vec<AgentInput>,
    prompt: String,
}

#[derive(Debug)]
enum Behavior {
    ReturnWithoutCompletion,
    Activate(Vec<String>),
    Complete { input_id: String, bytes: Vec<u8> },
    CompleteTwice,
    CompleteThenInvalid,
    CompleteWithExit(i32),
    CompleteEmpty,
}

#[derive(Debug)]
struct FakeAgentRegistry {
    root: PathBuf,
    behaviors: Mutex<VecDeque<Behavior>>,
    calls: Mutex<Vec<Call>>,
}

impl FakeAgentRegistry {
    fn new(root: PathBuf, behaviors: impl IntoIterator<Item = Behavior>) -> Self {
        Self {
            root,
            behaviors: Mutex::new(behaviors.into_iter().collect()),
            calls: Mutex::new(Vec::new()),
        }
    }
}

impl AgentRegistry for FakeAgentRegistry {
    fn validate(&self, type_id: &str, model: &str, reasoning: &str) -> Result<(), String> {
        if type_id == "codex" && !model.is_empty() && !reasoning.is_empty() {
            Ok(())
        } else {
            Err("невалидный fake Agent".to_owned())
        }
    }

    fn run(
        &self,
        request: &AgentRunRequest<'_>,
        control: &mut dyn AttemptControl,
    ) -> Result<AgentExit, String> {
        let run_directory = self.root.join("run").join(request.run_id);
        if !run_directory.join("spec.yaml").is_file()
            || !run_directory
                .join(format!(
                    "{}.{}.attempt.yaml",
                    request.attempt, request.step_id
                ))
                .is_file()
        {
            return Err("Agent вызван до durable-публикации run".to_owned());
        }
        self.calls
            .lock()
            .expect("call log must be available")
            .push(Call {
                step_id: request.step_id.to_owned(),
                attempt: request.attempt,
                resume_session: request.resume_session.map(str::to_owned),
                inputs: request.inputs.to_vec(),
                prompt: request.prompt.to_owned(),
            });
        let behavior = self
            .behaviors
            .lock()
            .expect("behavior queue must be available")
            .pop_front()
            .unwrap_or(Behavior::ReturnWithoutCompletion);
        match behavior {
            Behavior::ReturnWithoutCompletion => {}
            Behavior::Activate(sessions) => {
                for session in sessions {
                    control.activate_session(&session)?;
                }
            }
            Behavior::Complete { input_id, bytes } => {
                let source = self
                    .root
                    .join(format!("source-artifact-{}", request.attempt));
                fs::write(&source, bytes).map_err(|error| error.to_string())?;
                control.complete(&[(input_id, source)])?;
            }
            Behavior::CompleteTwice => {
                let source = self.root.join("source-artifact");
                fs::write(&source, b"draft").map_err(|error| error.to_string())?;
                control.complete(&[("result".to_owned(), source.clone())])?;
                fs::write(&source, b"final").map_err(|error| error.to_string())?;
                control.complete(&[("result".to_owned(), source)])?;
            }
            Behavior::CompleteThenInvalid => {
                let source = self.root.join("source-artifact");
                fs::write(&source, b"good").map_err(|error| error.to_string())?;
                control.complete(&[("result".to_owned(), source.clone())])?;
                if control.complete(&[("extra".to_owned(), source)]).is_ok() {
                    return Err("невалидный completion был принят".to_owned());
                }
            }
            Behavior::CompleteWithExit(code) => {
                let source = self.root.join("source-artifact");
                fs::write(&source, b"final").map_err(|error| error.to_string())?;
                control.complete(&[("result".to_owned(), source)])?;
                return Ok(AgentExit { code });
            }
            Behavior::CompleteEmpty => control.complete(&[])?,
        }
        Ok(AgentExit { code: 0 })
    }
}

#[derive(Debug)]
struct ParallelAgentRegistry {
    calls: Mutex<Vec<Call>>,
    branch_barrier: Barrier,
    active: AtomicUsize,
    max_active: AtomicUsize,
    fail_left: bool,
}

impl ParallelAgentRegistry {
    fn new(fail_left: bool) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            branch_barrier: Barrier::new(2),
            active: AtomicUsize::new(0),
            max_active: AtomicUsize::new(0),
            fail_left,
        }
    }
}

impl AgentRegistry for ParallelAgentRegistry {
    fn validate(&self, type_id: &str, model: &str, reasoning: &str) -> Result<(), String> {
        if type_id == "codex" && !model.is_empty() && !reasoning.is_empty() {
            Ok(())
        } else {
            Err("невалидный parallel fake Agent".to_owned())
        }
    }

    fn run(
        &self,
        request: &AgentRunRequest<'_>,
        control: &mut dyn AttemptControl,
    ) -> Result<AgentExit, String> {
        self.calls
            .lock()
            .expect("parallel call log must be available")
            .push(Call {
                step_id: request.step_id.to_owned(),
                attempt: request.attempt,
                resume_session: request.resume_session.map(str::to_owned),
                inputs: request.inputs.to_vec(),
                prompt: request.prompt.to_owned(),
            });
        if matches!(request.step_id, "left" | "right") {
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_active.fetch_max(active, Ordering::SeqCst);
            control.activate_session(request.step_id)?;
            self.branch_barrier.wait();
            if self.fail_left && request.step_id == "left" {
                self.active.fetch_sub(1, Ordering::SeqCst);
                return Ok(AgentExit { code: 7 });
            }
            control.complete(&[])?;
            self.active.fetch_sub(1, Ordering::SeqCst);
        } else {
            control.complete(&[])?;
        }
        Ok(AgentExit { code: 0 })
    }
}

#[derive(Debug, Default)]
struct VecReporter(Vec<String>);

impl LifecycleReporter for VecReporter {
    fn line(&mut self, value: &str) -> Result<(), std::io::Error> {
        self.0.push(value.to_owned());
        Ok(())
    }
}

#[given("подготовлен single-step workflow без outputs")]
fn workflow_without_outputs(world: &mut LifecycleWorld) {
    prepare_workflow(world, &[]);
}

#[given("подготовлен single-step workflow с output result")]
fn workflow_with_output(world: &mut LifecycleWorld) {
    prepare_workflow(world, &["result"]);
}

#[given("подготовлен линейный workflow source → target")]
fn linear_workflow(world: &mut LifecycleWorld) {
    prepare_graph(
        world,
        "steps:\n  - id: source\n    agent: main\n    prompt: null\n    human: false\n    depends-on: []\n    outputs: [result]\n  - id: target\n    agent: main\n    prompt: target\n    human: false\n    depends-on: [source]\n    outputs: []\n",
        Some((
            "target",
            "at {{path:source:result}} says {{content:source:result}}",
        )),
    );
}

#[given("подготовлен diamond workflow")]
fn diamond_workflow(world: &mut LifecycleWorld) {
    prepare_graph(
        world,
        "steps:\n  - id: root\n    agent: main\n    prompt: null\n    human: false\n    depends-on: []\n    outputs: []\n  - id: left\n    agent: main\n    prompt: null\n    human: false\n    depends-on: [root]\n    outputs: []\n  - id: right\n    agent: main\n    prompt: null\n    human: false\n    depends-on: [root]\n    outputs: []\n  - id: join\n    agent: main\n    prompt: null\n    human: false\n    depends-on: [left, right]\n    outputs: []\n",
        None,
    );
}

#[given("подготовлен diamond workflow с общим лимитом 2")]
fn parallel_diamond_workflow(world: &mut LifecycleWorld) {
    diamond_workflow(world);
    let root = world.root.as_ref().expect("scenario must define root");
    fs::write(
        root.path().join("config.yaml"),
        "default-agent: main\nmax-parallel-agents: 2\nagents:\n  main:\n    type: codex\n    model: model\n    reasoning: high\n",
    )
    .expect("parallel config must be written");
}

#[given("process branch Agents настроены для fail-fast")]
fn process_branches_for_fail_fast(world: &mut LifecycleWorld) {
    let root = world.root.as_ref().expect("scenario must define root");
    for name in ["left.gate", "right.gate"] {
        let status = Command::new("mkfifo")
            .arg(root.path().join(name))
            .status()
            .expect("mkfifo must run");
        assert!(status.success());
    }
    prepare_process_agent(
        world,
        "#!/bin/sh\ncase \"$ORC_STEP_ID\" in\n  root|join) \"$ORC_TEST_ORCHESTRATOR\" attempt complete ;;\n  left) printf %s \"$ORC_CONTROL_ENDPOINT\" > \"$ORC_HOME/left.endpoint\"; : > \"$ORC_HOME/left.ready\"; IFS= read -r ignored < \"$ORC_HOME/left.gate\"; exit 7 ;;\n  right) trap ': > \"$ORC_HOME/right.terminated\"; exit 143' TERM; printf %s \"$ORC_CONTROL_ENDPOINT\" > \"$ORC_HOME/right.endpoint\"; : > \"$ORC_HOME/right.ready\"; IFS= read -r ignored < \"$ORC_HOME/right.gate\"; \"$ORC_TEST_ORCHESTRATOR\" attempt complete ;;\nesac\n",
    );
}

#[given("подготовлен fan-in workflow с одинаковым output shared")]
fn shared_output_workflow(world: &mut LifecycleWorld) {
    prepare_graph(
        world,
        "steps:\n  - id: root\n    agent: main\n    prompt: null\n    human: false\n    depends-on: []\n    outputs: [seed]\n  - id: left\n    agent: main\n    prompt: null\n    human: false\n    depends-on: [root]\n    outputs: [shared]\n  - id: right\n    agent: main\n    prompt: null\n    human: false\n    depends-on: [root]\n    outputs: [shared]\n  - id: join\n    agent: main\n    prompt: null\n    human: false\n    depends-on: [left, right]\n    outputs: []\n",
        None,
    );
}

#[given("подготовлен корень без runs")]
fn root_without_runs(world: &mut LifecycleWorld) {
    world.root = Some(TempDir::new().expect("test root must be created"));
}

#[given("process Agent возвращает управление без completion")]
fn process_agent_without_completion(world: &mut LifecycleWorld) {
    prepare_process_agent(world, "#!/bin/sh\nexit 0\n");
}

#[given("process Agent ожидает явного разрешения")]
fn process_agent_waits_for_release(world: &mut LifecycleWorld) {
    let root = world.root.as_ref().expect("scenario must define root");
    let gate = root.path().join("agent-gate");
    let ready = root.path().join("agent-ready");
    let status = Command::new("mkfifo")
        .arg(&gate)
        .status()
        .expect("mkfifo must run");
    assert!(status.success());
    world.process_gate = Some(gate);
    world.process_ready = Some(ready);
    prepare_process_agent(
        world,
        "#!/bin/sh\n: > \"$ORC_TEST_READY\"\nIFS= read -r ignored < \"$ORC_TEST_GATE\"\n",
    );
}

#[given("process Agent публикует artifact final через attempt complete")]
fn process_agent_with_completion(world: &mut LifecycleWorld) {
    prepare_process_agent(
        world,
        "#!/bin/sh\nprintf final > \"$ORC_TEST_ARTIFACT\"\n\"$ORC_TEST_ORCHESTRATOR\" attempt complete --artifact result \"$ORC_TEST_ARTIFACT\"\n",
    );
}

#[given(expr = "process Agent отправляет невалидный completion {string}")]
#[allow(clippy::needless_pass_by_value)]
fn process_agent_with_invalid_completion(world: &mut LifecycleWorld, case: String) {
    let root = world.root.as_ref().expect("scenario must define root");
    world.control_code = Some(root.path().join("control-code"));
    let arguments = match case.as_str() {
        "с отсутствующим output" => "",
        "с дополнительным output" => "--artifact extra \"$ORC_TEST_ARTIFACT\"",
        "с повторяющимся output" => {
            "--artifact result \"$ORC_TEST_ARTIFACT\" --artifact result \"$ORC_TEST_ARTIFACT\""
        }
        "с относительным path" => "--artifact result relative",
        "с отсутствующим path" => "--artifact result \"$ORC_TEST_MISSING\"",
        "с path на directory" => "--artifact result \"$ORC_HOME\"",
        other => panic!("unknown invalid completion case: {other}"),
    };
    prepare_process_agent(
        world,
        &format!(
            "#!/bin/sh\nprintf source > \"$ORC_TEST_ARTIFACT\"\n\"$ORC_TEST_ORCHESTRATOR\" attempt complete {arguments}\nprintf %s $? > \"$ORC_TEST_CONTROL_CODE\"\n"
        ),
    );
}

#[given("process Agent активирует session process-session")]
fn process_agent_activates_session(world: &mut LifecycleWorld) {
    prepare_process_agent(
        world,
        "#!/bin/sh\n\"$ORC_TEST_ORCHESTRATOR\" session activate process-session\n",
    );
}

#[when("process Agent активирует session и следующий resume её продолжает")]
fn process_session_round_trip(world: &mut LifecycleWorld) {
    process_agent_activates_session(world);
    start_through_process(world);
    assert_eq!(
        world.observed().exit_code,
        1,
        "{:?}",
        world.observed().error
    );
    prepare_process_agent(
        world,
        "#!/bin/sh\ntest \"$ORC_RESUME_SESSION\" = process-session || exit 9\n\"$ORC_TEST_ORCHESTRATOR\" attempt complete\n",
    );
    let root = world.root.as_ref().expect("scenario must define root");
    let agent = world
        .process_agent
        .as_ref()
        .expect("scenario must define Agent");
    let run_id = world.run_id.as_ref().expect("scenario must define run ID");
    let output = Command::new(env!("CARGO_BIN_EXE_orchestrator"))
        .args(["resume", run_id])
        .env("ORC_HOME", root.path())
        .env("ORC_AGENT_COMMAND", agent)
        .env("ORC_TEST_ORCHESTRATOR", env!("CARGO_BIN_EXE_orchestrator"))
        .output()
        .expect("orchestrator resume must run");
    world.observed = Some(Observed {
        exit_code: u8::try_from(output.status.code().expect("process must exit normally"))
            .expect("fixture exit code must fit u8"),
        lines: String::from_utf8(output.stdout)
            .expect("stdout must be UTF-8")
            .lines()
            .map(str::to_owned)
            .collect(),
        error: Some(String::from_utf8_lossy(&output.stderr).into_owned()),
    });
}

#[given("подготовлен завершённый single-step run")]
fn completed_run(world: &mut LifecycleWorld) {
    prepare_workflow(world, &["result"]);
    run_start(
        world,
        [Behavior::Complete {
            input_id: "result".to_owned(),
            bytes: b"initial".to_vec(),
        }],
    );
    world.calls.clear();
    world.observed = None;
}

#[when("workflow запускается через lifecycle API с возвратом без completion")]
fn start_without_completion(world: &mut LifecycleWorld) {
    run_start(world, [Behavior::ReturnWithoutCompletion]);
}

#[when("Agent активирует sessions a, b, a, a и возвращается без completion")]
fn activate_sessions(world: &mut LifecycleWorld) {
    run_start(
        world,
        [Behavior::Activate(vec![
            "a".to_owned(),
            "b".to_owned(),
            "a".to_owned(),
            "a".to_owned(),
        ])],
    );
}

#[when("Agent передаёт completion с bytes final и возвращает управление")]
fn complete_with_artifact(world: &mut LifecycleWorld) {
    run_start(
        world,
        [Behavior::Complete {
            input_id: "result".to_owned(),
            bytes: b"final".to_vec(),
        }],
    );
}

#[when("Agent передаёт completion сначала с bytes draft, затем final и возвращает управление")]
fn replace_completion_candidate(world: &mut LifecycleWorld) {
    run_start(world, [Behavior::CompleteTwice]);
}

#[when("Agent передаёт валидный completion good, затем невалидный extra и возвращает управление")]
fn invalid_completion_keeps_candidate(world: &mut LifecycleWorld) {
    run_start(world, [Behavior::CompleteThenInvalid]);
}

#[when("Agent передаёт completion final и завершается с кодом 7")]
fn completion_before_nonzero_exit(world: &mut LifecycleWorld) {
    run_start(world, [Behavior::CompleteWithExit(7)]);
}

#[when("source публикует result hello, а target завершается")]
fn complete_linear_workflow(world: &mut LifecycleWorld) {
    run_start(
        world,
        [
            Behavior::Complete {
                input_id: "result".to_owned(),
                bytes: b"hello".to_vec(),
            },
            Behavior::CompleteEmpty,
        ],
    );
}

#[when("source публикует невалидный UTF-8 result")]
fn invalid_utf8_source(world: &mut LifecycleWorld) {
    run_start(
        world,
        [Behavior::Complete {
            input_id: "result".to_owned(),
            bytes: vec![0xff],
        }],
    );
}

#[when("все четыре Steps успешно завершаются")]
fn complete_diamond(world: &mut LifecycleWorld) {
    run_start(
        world,
        std::iter::repeat_with(|| Behavior::CompleteEmpty).take(4),
    );
}

#[when("ветви выполняются через синхронизируемые fake Agents")]
fn complete_parallel_diamond(world: &mut LifecycleWorld) {
    run_parallel_diamond(world, false);
}

#[when("левая ветвь fail-fast завершается с ошибкой")]
fn fail_parallel_diamond(world: &mut LifecycleWorld) {
    run_parallel_diamond(world, true);
}

#[when("process fail-fast освобождает заблокированную соседнюю ветвь")]
fn process_fail_fast_releases_sibling(world: &mut LifecycleWorld) {
    let root = world.root.as_ref().expect("scenario must define root");
    let agent = world
        .process_agent
        .as_ref()
        .expect("scenario must define process Agent");
    let child = Command::new(env!("CARGO_BIN_EXE_orchestrator"))
        .args(["start", "delivery"])
        .env("ORC_HOME", root.path())
        .env("ORC_AGENT_COMMAND", agent)
        .env("ORC_TEST_ORCHESTRATOR", env!("CARGO_BIN_EXE_orchestrator"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("orchestrator must start");
    wait_for_path(&root.path().join("left.ready"));
    wait_for_path(&root.path().join("right.ready"));
    fs::write(root.path().join("left.gate"), b"fail\n").expect("left Agent must be released");
    let output = child.wait_with_output().expect("orchestrator must exit");
    let lines: Vec<String> = String::from_utf8(output.stdout)
        .expect("stdout must be UTF-8")
        .lines()
        .map(str::to_owned)
        .collect();
    world.run_id = lines
        .iter()
        .find_map(|line| line.strip_prefix("Run "))
        .filter(|value| !value.ends_with(" exited"))
        .map(str::to_owned);
    world.observed = Some(Observed {
        exit_code: u8::try_from(output.status.code().expect("process must exit normally"))
            .expect("fixture exit code must fit u8"),
        lines,
        error: Some(String::from_utf8_lossy(&output.stderr).into_owned()),
    });
}

fn run_parallel_diamond(world: &mut LifecycleWorld, fail_left: bool) {
    let root = world.root.as_ref().expect("scenario must define root");
    let registry = ParallelAgentRegistry::new(fail_left);
    let mut reporter = VecReporter::default();
    let result = execute_lifecycle(
        &LifecycleCommand::start_explicit("delivery").expect("workflow ID must be valid"),
        &environment(root),
        &registry,
        &mut reporter,
    );
    world.run_id = reporter
        .0
        .iter()
        .find_map(|line| line.strip_prefix("Run "))
        .filter(|value| !value.ends_with(" exited"))
        .map(str::to_owned);
    world.calls = registry
        .calls
        .into_inner()
        .expect("parallel call log must be available");
    world.max_concurrency = registry.max_active.load(Ordering::SeqCst);
    world.observed = Some(observe(result, reporter));
}

#[when("все четыре Steps публикуют свои outputs и завершаются")]
fn complete_shared_outputs(world: &mut LifecycleWorld) {
    run_start(
        world,
        [
            Behavior::Complete {
                input_id: "seed".to_owned(),
                bytes: b"seed".to_vec(),
            },
            Behavior::Complete {
                input_id: "shared".to_owned(),
                bytes: b"left".to_vec(),
            },
            Behavior::Complete {
                input_id: "shared".to_owned(),
                bytes: b"right".to_vec(),
            },
            Behavior::CompleteEmpty,
        ],
    );
}

#[when(expr = "неизвестный run {word} продолжается через lifecycle API")]
#[allow(clippy::needless_pass_by_value)]
fn resume_unknown_run(world: &mut LifecycleWorld, run_id: String) {
    let root = world.root.as_ref().expect("scenario must define root");
    let registry = FakeAgentRegistry::new(root.path().to_owned(), []);
    let command = LifecycleCommand::Resume(
        orchestrator::RunId::parse(&run_id).expect("run ID must be valid"),
    );
    let mut reporter = VecReporter::default();
    let result = execute_lifecycle(&command, &environment(root), &registry, &mut reporter);
    world.observed = Some(observe(result, reporter));
    world.run_id = Some(run_id);
}

#[when("запускается orchestrator start delivery")]
fn start_through_process(world: &mut LifecycleWorld) {
    let root = world.root.as_ref().expect("scenario must define root");
    let agent = world
        .process_agent
        .as_ref()
        .expect("scenario must define process Agent");
    let output = Command::new(env!("CARGO_BIN_EXE_orchestrator"))
        .args(["start", "delivery"])
        .env("ORC_HOME", root.path())
        .env("ORC_AGENT_COMMAND", agent)
        .env("ORC_TEST_ORCHESTRATOR", env!("CARGO_BIN_EXE_orchestrator"))
        .env("ORC_TEST_ARTIFACT", root.path().join("process-source"))
        .env("ORC_TEST_MISSING", root.path().join("missing-source"))
        .env(
            "ORC_TEST_CONTROL_CODE",
            world.control_code.as_deref().unwrap_or_else(|| root.path()),
        )
        .output()
        .expect("orchestrator must run");
    let stdout = String::from_utf8(output.stdout).expect("stdout must be UTF-8");
    let lines: Vec<String> = stdout.lines().map(str::to_owned).collect();
    world.run_id = lines
        .iter()
        .find_map(|line| line.strip_prefix("Run "))
        .filter(|value| !value.ends_with(" exited"))
        .map(str::to_owned);
    world.observed = Some(Observed {
        exit_code: u8::try_from(output.status.code().expect("process must exit normally"))
            .expect("fixture exit code must fit u8"),
        lines,
        error: Some(String::from_utf8_lossy(&output.stderr).into_owned()),
    });
}

#[when("во время первого start запускается competing resume")]
fn competing_resume(world: &mut LifecycleWorld) {
    let root = world.root.as_ref().expect("scenario must define root");
    let agent = world
        .process_agent
        .as_ref()
        .expect("scenario must define Agent");
    let gate = world
        .process_gate
        .as_ref()
        .expect("scenario must define gate");
    let ready = world
        .process_ready
        .as_ref()
        .expect("scenario must define ready marker");
    let first = Command::new(env!("CARGO_BIN_EXE_orchestrator"))
        .args(["start", "delivery"])
        .env("ORC_HOME", root.path())
        .env("ORC_AGENT_COMMAND", agent)
        .env("ORC_TEST_GATE", gate)
        .env("ORC_TEST_READY", ready)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("first orchestrator must start");
    wait_for_path(ready);
    let run_id = fs::read_dir(root.path().join("run"))
        .expect("run root must be readable")
        .filter_map(Result::ok)
        .find(|entry| entry.path().is_dir())
        .expect("run must be reserved")
        .file_name()
        .to_string_lossy()
        .into_owned();
    let competing = Command::new(env!("CARGO_BIN_EXE_orchestrator"))
        .args(["resume", &run_id])
        .env("ORC_HOME", root.path())
        .env("ORC_AGENT_COMMAND", agent)
        .output()
        .expect("competing orchestrator must run");
    fs::write(gate, b"release\n").expect("Agent must be released");
    let first_output = first
        .wait_with_output()
        .expect("first orchestrator must exit");
    assert_eq!(first_output.status.code(), Some(1));
    world.run_id = Some(run_id);
    world.observed = Some(Observed {
        exit_code: u8::try_from(competing.status.code().expect("process must exit normally"))
            .expect("fixture exit code must fit u8"),
        lines: String::from_utf8(competing.stdout)
            .expect("stdout must be UTF-8")
            .lines()
            .map(str::to_owned)
            .collect(),
        error: Some(String::from_utf8_lossy(&competing.stderr).into_owned()),
    });
}

#[when("run продолжается через lifecycle API")]
fn resume_run(world: &mut LifecycleWorld) {
    let root = world.root.as_ref().expect("scenario must define root");
    let registry =
        FakeAgentRegistry::new(root.path().to_owned(), [Behavior::ReturnWithoutCompletion]);
    let run_id = world.run_id.as_ref().expect("scenario must define run ID");
    let command =
        LifecycleCommand::Resume(orchestrator::RunId::parse(run_id).expect("run ID must be valid"));
    let mut reporter = VecReporter::default();
    let result = execute_lifecycle(&command, &environment(root), &registry, &mut reporter);
    world.calls = registry
        .calls
        .into_inner()
        .expect("call log must be available");
    world.observed = Some(observe(result, reporter));
}

#[then(expr = "lifecycle завершается с кодом {int}")]
fn lifecycle_exit_code(world: &mut LifecycleWorld, expected: u8) {
    assert_eq!(
        world.observed().exit_code,
        expected,
        "{:?}",
        world.observed().error
    );
}

#[then("до вызова Agent опубликованы spec и initial attempt 0")]
fn durable_before_agent(world: &mut LifecycleWorld) {
    let directory = run_directory(world);
    assert!(directory.join("spec.yaml").is_file());
    assert!(directory.join("0.first.attempt.yaml").is_file());
    let spec = fs::read_to_string(directory.join("spec.yaml"))
        .expect("materialized workflow must be readable");
    assert!(spec.contains("workflow-id: delivery"));
    assert!(spec.contains("max-parallel-agents: 5"));
    assert!(spec.contains("agent:\n    type: codex"));
    assert!(spec.contains("prompt: null"));
    assert!(!spec.contains("agent: main"));
    assert_eq!(
        world.calls,
        vec![Call {
            step_id: "first".to_owned(),
            attempt: 0,
            resume_session: None,
            inputs: Vec::new(),
            prompt: String::new(),
        }],
        "{:?}",
        world.observed().error
    );
}

#[then("initial attempt остаётся незавершённым")]
fn attempt_unfinished(world: &mut LifecycleWorld) {
    let text = fs::read_to_string(run_directory(world).join("0.first.attempt.yaml"))
        .expect("attempt record must be readable");
    assert!(!text.contains("completed"));
}

#[then("competing resume не изменяет initial attempt")]
fn competing_resume_does_not_mutate(world: &mut LifecycleWorld) {
    attempt_unfinished(world);
}

#[then("stdout содержит workflow, RunId и финальную строку exited в стабильном порядке")]
fn stable_start_output(world: &mut LifecycleWorld) {
    let lines = &world.observed().lines;
    assert_eq!(
        lines.first().map(String::as_str),
        Some("workflow: delivery")
    );
    assert!(lines.get(1).is_some_and(|line| line.starts_with("Run ")));
    assert!(lines.last().is_some_and(|line| line.ends_with(" exited")));
}

#[then("resume запускает тот же attempt 0 с session a")]
fn resumed_same_attempt(world: &mut LifecycleWorld) {
    assert_eq!(
        world.calls,
        vec![Call {
            step_id: "first".to_owned(),
            attempt: 0,
            resume_session: Some("a".to_owned()),
            inputs: Vec::new(),
            prompt: String::new(),
        }],
        "{:?}",
        world.observed().error
    );
}

#[then("durable activations равны a, b, a")]
fn durable_activations(world: &mut LifecycleWorld) {
    let text = fs::read_to_string(run_directory(world).join("0.first.attempt.yaml"))
        .expect("attempt record must be readable");
    let sessions: Vec<&str> = text
        .lines()
        .filter_map(|line| line.strip_prefix("  session-id: "))
        .collect();
    assert_eq!(sessions, ["a", "b", "a"]);
}

#[then("durable activations равны process-session")]
fn durable_process_activation(world: &mut LifecycleWorld) {
    let text = fs::read_to_string(run_directory(world).join("0.first.attempt.yaml"))
        .expect("attempt record must be readable");
    assert!(text.contains("session-id: process-session"));
}

#[then("каталог неизвестного run не создан")]
fn unknown_run_not_created(world: &mut LifecycleWorld) {
    assert!(!run_directory(world).exists());
}

#[then("durable artifact result содержит bytes final")]
fn artifact_has_final_bytes(world: &mut LifecycleWorld) {
    assert_eq!(
        fs::read(run_directory(world).join("0.first.result.artifact"))
            .expect("artifact must be readable"),
        b"final"
    );
}

#[then("durable artifact result содержит bytes good")]
fn artifact_has_good_bytes(world: &mut LifecycleWorld) {
    assert_eq!(
        fs::read(run_directory(world).join("0.first.result.artifact"))
            .expect("artifact must be readable"),
        b"good"
    );
}

#[then("дочерний attempt complete завершился с кодом 3")]
fn child_completion_exit_code(world: &mut LifecycleWorld) {
    let path = world
        .control_code
        .as_ref()
        .expect("scenario must define control code path");
    assert_eq!(
        fs::read_to_string(path).expect("control code must be readable"),
        "3"
    );
}

#[then("target attempt 1 имеет input 0")]
fn target_input_is_source(world: &mut LifecycleWorld) {
    let record = fs::read_to_string(run_directory(world).join("1.target.attempt.yaml"))
        .expect("target attempt must be readable");
    assert!(record.contains("input:\n- 0"));
}

#[then("target Agent получает artifact source:result и prompt с path и content hello")]
fn target_receives_input_and_prompt(world: &mut LifecycleWorld) {
    let call = world.calls.get(1).expect("target Agent must be called");
    assert_eq!(call.step_id, "target");
    assert_eq!(call.inputs.len(), 1);
    assert_eq!(call.inputs[0].step_id, "source");
    assert_eq!(call.inputs[0].input_id, "result");
    assert!(
        call.prompt
            .contains(call.inputs[0].path.to_string_lossy().as_ref())
    );
    assert!(call.prompt.ends_with("says hello"));
}

#[then("target attempt 1 не создан")]
fn target_attempt_absent(world: &mut LifecycleWorld) {
    assert!(!run_directory(world).join("1.target.attempt.yaml").exists());
}

#[then("attempts созданы как 0 root, 1 left, 2 right, 3 join")]
fn diamond_attempt_order(world: &mut LifecycleWorld) {
    let directory = run_directory(world);
    for name in [
        "0.root.attempt.yaml",
        "1.left.attempt.yaml",
        "2.right.attempt.yaml",
        "3.join.attempt.yaml",
    ] {
        assert!(directory.join(name).is_file(), "missing {name}");
    }
    let mut calls: Vec<(&str, u64)> = world
        .calls
        .iter()
        .map(|call| (call.step_id.as_str(), call.attempt))
        .collect();
    calls.sort_by_key(|call| call.1);
    assert_eq!(calls, [("root", 0), ("left", 1), ("right", 2), ("join", 3)]);
}

#[then("join attempt имеет input 1, 2")]
fn join_input_versions(world: &mut LifecycleWorld) {
    let record = fs::read_to_string(run_directory(world).join("3.join.attempt.yaml"))
        .expect("join attempt must be readable");
    assert!(record.contains("input:\n- 1\n- 2"));
}

#[then("join Agent получает inputs left:shared и right:shared")]
fn join_receives_distinct_inputs(world: &mut LifecycleWorld) {
    let call = world.calls.last().expect("join Agent must be called");
    let keys: Vec<(&str, &str)> = call
        .inputs
        .iter()
        .map(|input| (input.step_id.as_str(), input.input_id.as_str()))
        .collect();
    assert_eq!(keys, [("left", "shared"), ("right", "shared")]);
}

#[then("одновременно работали ровно 2 branch Agents")]
fn exactly_two_parallel_agents(world: &mut LifecycleWorld) {
    assert_eq!(world.max_concurrency, 2);
}

#[then("parallel attempts сохранили независимые session contexts")]
fn parallel_sessions_are_independent(world: &mut LifecycleWorld) {
    let directory = run_directory(world);
    let left = fs::read_to_string(directory.join("1.left.attempt.yaml"))
        .expect("left attempt must be readable");
    let right = fs::read_to_string(directory.join("2.right.attempt.yaml"))
        .expect("right attempt must be readable");
    assert!(left.contains("session-id: left"));
    assert!(!left.contains("session-id: right"));
    assert!(right.contains("session-id: right"));
    assert!(!right.contains("session-id: left"));
}

#[then("успешная правая ветвь durable завершена, а join не создан")]
fn successful_sibling_is_durable_without_join(world: &mut LifecycleWorld) {
    let directory = run_directory(world);
    let right = fs::read_to_string(directory.join("2.right.attempt.yaml"))
        .expect("right attempt must be readable");
    assert!(right.trim_end().ends_with("type: completed"));
    assert!(!directory.join("3.join.attempt.yaml").exists());
}

#[then("обе process ветви использовали один endpoint и правая получила SIGTERM")]
fn process_branches_share_endpoint_and_cancel(world: &mut LifecycleWorld) {
    let root = world.root.as_ref().expect("scenario must define root");
    let left = fs::read_to_string(root.path().join("left.endpoint"))
        .expect("left endpoint must be readable");
    let right = fs::read_to_string(root.path().join("right.endpoint"))
        .expect("right endpoint must be readable");
    assert!(!left.is_empty());
    assert_eq!(left, right);
    assert!(root.path().join("right.terminated").is_file());
}

#[then("attempt завершён terminal event completed")]
fn attempt_completed(world: &mut LifecycleWorld) {
    let text = fs::read_to_string(run_directory(world).join("0.first.attempt.yaml"))
        .expect("attempt record must be readable");
    assert!(text.trim_end().ends_with("type: completed"));
}

#[then("lifecycle сообщает о завершении run")]
fn reports_completed(world: &mut LifecycleWorld) {
    assert!(
        world
            .observed()
            .lines
            .iter()
            .any(|line| line.contains(": completed"))
    );
}

#[then("Agent не запускается повторно")]
fn agent_not_restarted(world: &mut LifecycleWorld) {
    assert!(world.calls.is_empty());
}

#[then("lifecycle сообщает already completed")]
fn reports_already_completed(world: &mut LifecycleWorld) {
    assert!(
        world
            .observed()
            .lines
            .iter()
            .any(|line| line.contains("already completed"))
    );
}

fn prepare_workflow(world: &mut LifecycleWorld, outputs: &[&str]) {
    let root = TempDir::new().expect("test root must be created");
    fs::create_dir_all(root.path().join("workflow")).expect("workflow root must be created");
    fs::write(
        root.path().join("config.yaml"),
        "default-agent: main\nagents:\n  main:\n    type: codex\n    model: model\n    reasoning: high\n",
    )
    .expect("config must be written");
    let outputs = if outputs.is_empty() {
        "[]".to_owned()
    } else {
        format!("[{}]", outputs.join(", "))
    };
    fs::write(
        root.path().join("workflow/delivery.yaml"),
        format!("steps:\n  - id: first\n    agent: main\n    prompt: null\n    human: false\n    depends-on: []\n    outputs: {outputs}\n"),
    )
    .expect("workflow must be written");
    world.root = Some(root);
}

fn prepare_graph(world: &mut LifecycleWorld, workflow: &str, prompt: Option<(&str, &str)>) {
    let root = TempDir::new().expect("test root must be created");
    fs::create_dir_all(root.path().join("workflow")).expect("workflow root must be created");
    fs::create_dir_all(root.path().join("prompt")).expect("prompt root must be created");
    fs::write(
        root.path().join("config.yaml"),
        "default-agent: main\nagents:\n  main:\n    type: codex\n    model: model\n    reasoning: high\n",
    )
    .expect("config must be written");
    fs::write(root.path().join("workflow/delivery.yaml"), workflow)
        .expect("workflow must be written");
    if let Some((id, content)) = prompt {
        fs::write(root.path().join("prompt").join(format!("{id}.md")), content)
            .expect("prompt must be written");
    }
    world.root = Some(root);
}

fn prepare_process_agent(world: &mut LifecycleWorld, script: &str) {
    use std::os::unix::fs::PermissionsExt;

    let root = world.root.as_ref().expect("scenario must define root");
    let path = root.path().join("fake-agent.sh");
    fs::write(&path, script).expect("process Agent must be written");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
        .expect("process Agent must be executable");
    world.process_agent = Some(path);
}

fn wait_for_path(path: &std::path::Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "timeout waiting for {}",
            path.display()
        );
        std::thread::yield_now();
    }
}

fn run_start(world: &mut LifecycleWorld, behaviors: impl IntoIterator<Item = Behavior>) {
    let root = world.root.as_ref().expect("scenario must define root");
    let registry = FakeAgentRegistry::new(root.path().to_owned(), behaviors);
    let mut reporter = VecReporter::default();
    let result = execute_lifecycle(
        &LifecycleCommand::start_explicit("delivery").expect("workflow ID must be valid"),
        &environment(root),
        &registry,
        &mut reporter,
    );
    world.run_id = reporter
        .0
        .iter()
        .find_map(|line| line.strip_prefix("Run "))
        .filter(|value| !value.ends_with(" exited"))
        .map(str::to_owned);
    world.calls = registry
        .calls
        .into_inner()
        .expect("call log must be available");
    world.observed = Some(observe(result, reporter));
}

fn observe(result: Result<(), orchestrator::CommandError>, reporter: VecReporter) -> Observed {
    let (exit_code, error) = result.map_or_else(
        |error| (error.exit_code(), Some(error.to_string())),
        |()| (0, None),
    );
    Observed {
        exit_code,
        lines: reporter.0,
        error,
    }
}

fn environment(root: &TempDir) -> ProcessEnvironment {
    ProcessEnvironment {
        home: None,
        orc_home: Some(root.path().as_os_str().to_owned()),
    }
}

fn run_directory(world: &LifecycleWorld) -> PathBuf {
    world
        .root
        .as_ref()
        .expect("scenario must define root")
        .path()
        .join("run")
        .join(world.run_id.as_ref().expect("scenario must define run ID"))
}

impl LifecycleWorld {
    fn observed(&self) -> &Observed {
        self.observed
            .as_ref()
            .expect("scenario must execute lifecycle")
    }
}

#[tokio::main]
async fn main() {
    LifecycleWorld::run("features/lifecycle.feature").await;
    LifecycleWorld::run("features/graph_execution.feature").await;
}
