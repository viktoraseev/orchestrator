//! Cucumber-проверка публичного lifecycle API и process-контрактов lifecycle CLI.

use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Barrier, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use cucumber::{World, given, then, when};
use orchestrator::{
    AgentExit, AgentInput, AgentRegistry, AgentRunRequest, AttemptControl, LifecycleCommand,
    LifecycleReporter, LifecycleSignals, ProcessEnvironment, TerminalMode, execute_lifecycle,
};
use tempfile::TempDir;

#[derive(Debug, Default, World)]
struct LifecycleWorld {
    root: Option<TempDir>,
    run_id: Option<String>,
    observed: Option<Observed>,
    prior_observed: Option<Observed>,
    calls: Vec<Call>,
    durable_snapshot: Vec<(String, Vec<u8>)>,
    process_agent: Option<PathBuf>,
    default_agent_path: Option<PathBuf>,
    process_gate: Option<PathBuf>,
    process_ready: Option<PathBuf>,
    control_code: Option<PathBuf>,
    max_concurrency: usize,
    sent_signal: Option<String>,
    shutdown_elapsed: Option<Duration>,
    agent_type: Option<String>,
    protocol_case: Option<String>,
    start_window_ms: Option<(u128, u128)>,
    control_endpoint_observation: Option<(PathBuf, bool, u32)>,
    stale_control_endpoint: Option<PathBuf>,
    new_control_endpoint: Option<PathBuf>,
    stale_endpoint_existed: bool,
    competing_durable_unchanged: bool,
    concurrent_observed: Vec<Observed>,
    concurrent_run_ids: Vec<String>,
}

struct ProcessTreeGuard {
    child: Option<Child>,
    agent_pid_path: PathBuf,
}

impl ProcessTreeGuard {
    fn new(child: Child, agent_pid_path: PathBuf) -> Self {
        Self {
            child: Some(child),
            agent_pid_path,
        }
    }

    fn id(&self) -> u32 {
        self.child.as_ref().expect("child must be running").id()
    }

    fn wait_with_output(&mut self) -> std::process::Output {
        self.child
            .take()
            .expect("child must be running")
            .wait_with_output()
            .expect("orchestrator must exit")
    }
}

impl Drop for ProcessTreeGuard {
    fn drop(&mut self) {
        if let Ok(agent_pid) = fs::read_to_string(&self.agent_pid_path) {
            let _ = Command::new("/bin/kill")
                .args(["-KILL", agent_pid.trim()])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        if let Some(child) = &mut self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[given(expr = "подготовлен single-step workflow для Agent type {word}")]
#[allow(clippy::needless_pass_by_value)]
fn workflow_for_agent_type(world: &mut LifecycleWorld, agent_type: String) {
    prepare_workflow(world, &[]);
    let root = world.root.as_ref().expect("scenario must define root");
    fs::write(
        root.path().join("config.yaml"),
        format!(
            "default-agent: main\nagents:\n  main:\n    type: {agent_type}\n    model: model\n    reasoning: high\n"
        ),
    )
    .expect("config must be written");
    world.agent_type = Some(agent_type);
}

#[given(expr = "config содержит Agent type {word} с model {word} и reasoning {word}")]
#[allow(clippy::needless_pass_by_value)]
fn config_for_agent_type(
    world: &mut LifecycleWorld,
    agent_type: String,
    model: String,
    reasoning: String,
) {
    let root = TempDir::new().expect("test root must be created");
    let model = if model == "empty" { "" } else { &model };
    let reasoning = if reasoning == "empty" { "" } else { &reasoning };
    fs::write(
        root.path().join("config.yaml"),
        format!(
            "agents:\n  main:\n    type: {agent_type}\n    model: '{model}'\n    reasoning: '{reasoning}'\n"
        ),
    )
    .expect("config must be written");
    world.root = Some(root);
}

#[when("config проверяется встроенным Agent registry")]
fn validate_config_with_builtin_registry(world: &mut LifecycleWorld) {
    let root = world.root.as_ref().expect("scenario must define root");
    let output = Command::new(env!("CARGO_BIN_EXE_orchestrator"))
        .args(["config", "list"])
        .env("ORC_HOME", root.path())
        .output()
        .expect("orchestrator config list must run");
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

#[then(expr = "проверка config завершается с кодом {int}")]
fn config_validation_exits_with(world: &mut LifecycleWorld, code: u8) {
    assert_eq!(
        world.observed().exit_code,
        code,
        "{:?}",
        world.observed().error
    );
}

#[given(expr = "в изолированном PATH доступен fake executable {word}")]
#[allow(clippy::needless_pass_by_value)]
fn default_agent_executable(world: &mut LifecycleWorld, agent_type: String) {
    use std::os::unix::fs::PermissionsExt;

    let root = world.root.as_ref().expect("scenario must define root");
    let bin = root.path().join("bin");
    fs::create_dir(&bin).expect("isolated PATH directory must be created");
    let path = bin.join(&agent_type);
    let session = match agent_type.as_str() {
        "codex" => "{\"type\":\"thread.started\",\"thread_id\":\"default-session\"}",
        "claude" => "{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"default-session\"}",
        other => panic!("unknown Agent type: {other}"),
    };
    fs::write(
        &path,
        format!(
            "#!/bin/sh\nprintf '%s\\n' '{session}'\n: > \"$ORC_HOME/default-{agent_type}-started\"\n\"$ORC_TEST_ORCHESTRATOR\" attempt complete\n"
        ),
    )
    .expect("default Agent executable must be written");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
        .expect("default Agent executable must be executable");
    world.default_agent_path = Some(bin);
}

#[when("workflow запускается без ORC_AGENT_COMMAND")]
fn start_with_default_agent_executable(world: &mut LifecycleWorld) {
    let root = world.root.as_ref().expect("scenario must define root");
    let path = world
        .default_agent_path
        .as_ref()
        .expect("scenario must define isolated PATH");
    let output = Command::new(env!("CARGO_BIN_EXE_orchestrator"))
        .args(["start", "delivery"])
        .env("ORC_HOME", root.path())
        .env("ORC_TEST_ORCHESTRATOR", env!("CARGO_BIN_EXE_orchestrator"))
        .env("PATH", path)
        .env_remove("ORC_AGENT_COMMAND")
        .output()
        .expect("orchestrator start must run");
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

#[then(expr = "был запущен default executable {word}")]
#[allow(clippy::needless_pass_by_value)]
fn default_agent_executable_started(world: &mut LifecycleWorld, agent_type: String) {
    let root = world.root.as_ref().expect("scenario must define root");
    assert!(
        root.path()
            .join(format!("default-{agent_type}-started"))
            .is_file()
    );
}

#[given(expr = "подготовлен single-step human workflow для Agent type {word}")]
#[allow(clippy::needless_pass_by_value)]
fn human_workflow_for_agent_type(world: &mut LifecycleWorld, agent_type: String) {
    workflow_for_agent_type(world, agent_type);
    let root = world.root.as_ref().expect("scenario must define root");
    fs::write(
        root.path().join("workflow/delivery.yaml"),
        "steps:\n  - id: first\n    agent: main\n    prompt: null\n    human: true\n    depends-on: []\n    outputs: []\n",
    )
    .expect("human workflow must be written");
}

#[given("process human Agent записывает args, активирует session и на resume завершает attempt")]
fn process_human_agent_records_args(world: &mut LifecycleWorld) {
    prepare_process_agent(
        world,
        "#!/bin/sh\ntest -t 0 && test -t 1 || exit 9\nif [ -n \"${ORC_RESUME_SESSION:-}\" ]; then\n  printf '%s\\n' \"$@\" > \"$ORC_HOME/human.resume.args\"\n  : > \"$ORC_HOME/human.resume.tty\"\n  \"$ORC_TEST_ORCHESTRATOR\" attempt complete\nelse\n  printf '%s\\n' \"$@\" > \"$ORC_HOME/human.start.args\"\n  : > \"$ORC_HOME/human.start.tty\"\n  \"$ORC_TEST_ORCHESTRATOR\" session activate human-session\nfi\n",
    );
}

#[when("human workflow запускается и продолжается через системный pseudo-terminal")]
fn start_and_resume_human_through_terminal(world: &mut LifecycleWorld) {
    start_human_through_terminal(world);
    assert_eq!(
        world.observed().exit_code,
        1,
        "{:?}",
        world.observed().error
    );
    resume_human_through_process_terminal(world);
}

#[then("обе human команды получили прямой TTY")]
fn both_human_commands_received_tty(world: &mut LifecycleWorld) {
    let root = world.root.as_ref().expect("scenario must define root");
    assert!(root.path().join("human.start.tty").is_file());
    assert!(root.path().join("human.resume.tty").is_file());
}

#[then(expr = "process human Agent получил точные start и resume args для {word}")]
#[allow(clippy::needless_pass_by_value)]
fn exact_human_agent_args(world: &mut LifecycleWorld, agent_type: String) {
    let root = world.root.as_ref().expect("scenario must define root");
    let start = fs::read_to_string(root.path().join("human.start.args"))
        .expect("human start args must be readable");
    let resume = fs::read_to_string(root.path().join("human.resume.args"))
        .expect("human resume args must be readable");
    let actual_start: Vec<&str> = start.lines().collect();
    let actual_resume: Vec<&str> = resume.lines().collect();
    let (expected_start, expected_resume) = match agent_type.as_str() {
        "codex" => (
            vec![
                "--model",
                "model",
                "--config",
                "model_reasoning_effort=\"high\"",
                "",
            ],
            vec![
                "resume",
                "--model",
                "model",
                "--config",
                "model_reasoning_effort=\"high\"",
                "human-session",
                "",
            ],
        ),
        "claude" => (
            vec!["--model", "model", "--effort", "high", ""],
            vec![
                "--model",
                "model",
                "--effort",
                "high",
                "--resume",
                "human-session",
                "",
            ],
        ),
        other => panic!("unknown Agent type: {other}"),
    };
    assert_eq!(actual_start, expected_start);
    assert_eq!(actual_resume, expected_resume);
}

#[given("process Agent записывает args и environment и завершает attempt")]
fn process_agent_records_contract(world: &mut LifecycleWorld) {
    prepare_process_agent(
        world,
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$ORC_HOME/agent.args\"\nprintf '%s\\n%s\\n%s\\n%s\\n' \"$ORC_STEP_ID\" \"$ORC_RUN_ID\" \"$ORC_ATTEMPT\" \"$ORC_CONTROL_ENDPOINT\" > \"$ORC_HOME/agent.env\"\nprintf '%s' \"$ORC_INPUT\" > \"$ORC_HOME/agent.input\"\n\"$ORC_TEST_ORCHESTRATOR\" attempt complete\n",
    );
}

#[given(regex = r"^ORC_AGENT_COMMAND является (.+)$")]
#[allow(clippy::needless_pass_by_value)]
fn invalid_process_agent(world: &mut LifecycleWorld, case: String) {
    let root = world.root.as_ref().expect("scenario must define root");
    let path = match case.as_str() {
        "relative path" => PathBuf::from("fake-agent"),
        "отсутствующий absolute path" => root.path().join("missing-agent"),
        "directory" => root.path().to_owned(),
        "non-executable regular file" => {
            let path = root.path().join("non-executable-agent");
            fs::write(&path, "#!/bin/sh\nexit 0\n").expect("fixture must be written");
            path
        }
        other => panic!("unknown ORC_AGENT_COMMAND case: {other}"),
    };
    world.process_agent = Some(path);
}

#[then(expr = "process Agent получил точные args для {word}")]
#[allow(clippy::needless_pass_by_value)]
fn exact_agent_args(world: &mut LifecycleWorld, agent_type: String) {
    let root = world.root.as_ref().expect("scenario must define root");
    let args =
        fs::read_to_string(root.path().join("agent.args")).expect("Agent args must be readable");
    let actual: Vec<&str> = args.lines().collect();
    let expected = match agent_type.as_str() {
        "codex" => vec![
            "exec",
            "--json",
            "--model",
            "model",
            "--config",
            "model_reasoning_effort=\"high\"",
            "",
        ],
        "claude" => vec![
            "--print",
            "--output-format",
            "stream-json",
            "--verbose",
            "--model",
            "model",
            "--effort",
            "high",
            "",
        ],
        other => panic!("unknown Agent type: {other}"),
    };
    assert_eq!(actual, expected);
}

#[given(
    "process Agent сначала активирует session, а на resume записывает args и завершает attempt"
)]
fn process_agent_records_resume_args(world: &mut LifecycleWorld) {
    prepare_process_agent(
        world,
        "#!/bin/sh\nif [ -n \"${ORC_RESUME_SESSION:-}\" ]; then\n  printf '%s\\n' \"$@\" > \"$ORC_HOME/agent.resume.args\"\n  \"$ORC_TEST_ORCHESTRATOR\" attempt complete\nfi\n",
    );
}

#[when("запускаются start и resume через process Agent")]
fn start_and_resume_through_process(world: &mut LifecycleWorld) {
    start_through_process(world);
    assert_eq!(
        world.observed().exit_code,
        1,
        "{:?}",
        world.observed().error
    );
    resume_through_process(world);
}

#[when("start материализует Agent, config изменяется и run продолжается")]
fn resume_after_agent_config_change(world: &mut LifecycleWorld) {
    start_through_process(world);
    assert_eq!(
        world.observed().exit_code,
        1,
        "{:?}",
        world.observed().error
    );
    let root = world.root.as_ref().expect("scenario must define root");
    let replacement_type = match world.agent_type.as_deref() {
        Some("codex") => "claude",
        Some("claude") => "codex",
        other => panic!("unknown materialized Agent type: {other:?}"),
    };
    fs::write(
        root.path().join("config.yaml"),
        format!(
            "default-agent: main\nagents:\n  main:\n    type: {replacement_type}\n    model: replacement\n    reasoning: low\n"
        ),
    )
    .expect("replacement config must be written");
    resume_through_process(world);
}

#[then(expr = "process Agent получил точные resume args для {word}")]
#[allow(clippy::needless_pass_by_value)]
fn exact_resume_agent_args(world: &mut LifecycleWorld, agent_type: String) {
    let root = world.root.as_ref().expect("scenario must define root");
    let args = fs::read_to_string(root.path().join("agent.resume.args"))
        .expect("resume Agent args must be readable");
    let actual: Vec<&str> = args.lines().collect();
    let expected = match agent_type.as_str() {
        "codex" => vec![
            "exec",
            "resume",
            "--json",
            "--model",
            "model",
            "--config",
            "model_reasoning_effort=\"high\"",
            "fake-session",
            "",
        ],
        "claude" => vec![
            "--print",
            "--output-format",
            "stream-json",
            "--verbose",
            "--model",
            "model",
            "--effort",
            "high",
            "--resume",
            "fake-session",
            "",
        ],
        other => panic!("unknown Agent type: {other}"),
    };
    assert_eq!(actual, expected);
}

#[then("process Agent получил полный control environment")]
fn full_agent_environment(world: &mut LifecycleWorld) {
    let root = world.root.as_ref().expect("scenario must define root");
    let environment = fs::read_to_string(root.path().join("agent.env"))
        .expect("Agent environment must be readable");
    let values: Vec<&str> = environment.lines().collect();
    assert_eq!(values[0], "first");
    assert_eq!(
        values[1],
        world.run_id.as_deref().expect("run ID must exist")
    );
    assert_eq!(values[2], "0");
    assert!(PathBuf::from(values[3]).is_absolute());
    assert_eq!(
        fs::read_to_string(root.path().join("agent.input")).expect("Agent input must be readable"),
        "[]\n"
    );
}

#[then("сырой Agent protocol отсутствует в выводе")]
fn raw_protocol_is_hidden(world: &mut LifecycleWorld) {
    let observed = world.observed();
    assert!(
        observed
            .lines
            .iter()
            .all(|line| !line.contains("thread.started"))
    );
    assert!(
        observed
            .error
            .as_deref()
            .is_none_or(|error| !error.contains("session_id"))
    );
}

#[then("initial attempt не создан")]
fn initial_attempt_not_created(world: &mut LifecycleWorld) {
    let root = world.root.as_ref().expect("scenario must define root");
    let run_root = root.path().join("run");
    assert!(
        !run_root.exists()
            || fs::read_dir(run_root)
                .expect("run root must be readable")
                .next()
                .is_none()
    );
}

#[given(
    expr = "process Agent игнорирует неизвестный event, публикует два сообщения для {word} и завершает attempt"
)]
#[allow(clippy::needless_pass_by_value)]
fn process_agent_publishes_messages(world: &mut LifecycleWorld, agent_type: String) {
    let events = match agent_type.as_str() {
        "codex" => {
            "printf '%s\\n' '{\"type\":\"future.event\",\"payload\":true}'\nprintf '%s\\n' '{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"first\"}}'\nprintf '%s\\n' '{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\" second\\n  message \"}}'"
        }
        "claude" => {
            "printf '%s\\n' '{\"type\":\"future.event\",\"payload\":true}'\nprintf '%s\\n' '{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"first\"}]}}'\nprintf '%s\\n' '{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\" second\"},{\"type\":\"tool_use\"},{\"type\":\"text\",\"text\":\"message \"}]}}'"
        }
        other => panic!("unknown Agent type: {other}"),
    };
    prepare_process_agent(
        world,
        &format!("#!/bin/sh\n{events}\n\"$ORC_TEST_ORCHESTRATOR\" attempt complete\n"),
    );
}

#[then("TTY board показывает 2 и последнее сообщение одной строкой")]
fn board_shows_normalized_message(world: &mut LifecycleWorld) {
    assert!(
        world
            .observed()
            .lines
            .iter()
            .any(|line| line.contains("first · 2 · second message"))
    );
}

#[then("Agent board отсутствует в выводе")]
fn board_is_absent(world: &mut LifecycleWorld) {
    assert!(
        world
            .observed()
            .lines
            .iter()
            .all(|line| !line.contains("Agent sessions:"))
    );
}

#[then("session view отсутствует в durable run")]
fn session_view_is_not_durable(world: &mut LifecycleWorld) {
    let run = run_directory(world);
    for entry in fs::read_dir(run).expect("run directory must be readable") {
        let path = entry.expect("run entry must be readable").path();
        if path.is_file() {
            let bytes = fs::read(path).expect("durable file must be readable");
            assert!(
                !bytes
                    .windows(b"second message".len())
                    .any(|window| window == b"second message")
            );
        }
    }
}

#[given(expr = "process Agent для {word} возвращает {word} protocol")]
#[allow(clippy::needless_pass_by_value)]
fn process_agent_returns_protocol(
    world: &mut LifecycleWorld,
    agent_type: String,
    protocol_case: String,
) {
    let session = |id: &str| match agent_type.as_str() {
        "codex" => format!("{{\"type\":\"thread.started\",\"thread_id\":\"{id}\"}}"),
        "claude" => format!("{{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"{id}\"}}"),
        other => panic!("unknown Agent type: {other}"),
    };
    let lines = match protocol_case.as_str() {
        "malformed" => vec!["not-json".to_owned()],
        "missing" => Vec::new(),
        "contradictory" => vec![session("first-session"), session("second-session")],
        other => panic!("unknown protocol case: {other}"),
    };
    let body = lines
        .iter()
        .map(|line| format!("printf '%s\\n' '{line}'"))
        .collect::<Vec<_>>()
        .join("\n");
    prepare_raw_process_agent(world, &format!("#!/bin/sh\n{body}\n"));
    world.protocol_case = Some(protocol_case);
}

#[given(
    expr = "process Agent для {word} повторяет session ID и передаёт неизвестный валидный event"
)]
#[allow(clippy::needless_pass_by_value)]
fn process_agent_repeats_session_and_unknown_event(world: &mut LifecycleWorld, agent_type: String) {
    let session = match agent_type.as_str() {
        "codex" => "{\"type\":\"thread.started\",\"thread_id\":\"same-session\"}",
        "claude" => "{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"same-session\"}",
        other => panic!("unknown Agent type: {other}"),
    };
    prepare_raw_process_agent(
        world,
        &format!(
            "#!/bin/sh\nprintf '%s\\n' '{session}'\nprintf '%s\\n' '{{\"type\":\"future.event\",\"payload\":true}}'\nprintf '%s\\n' '{session}'\n\"$ORC_TEST_ORCHESTRATOR\" attempt complete\n"
        ),
    );
}

#[then("durable attempt содержит одну session activation")]
fn durable_attempt_has_one_session_activation(world: &mut LifecycleWorld) {
    let text = fs::read_to_string(run_directory(world).join("0.first.attempt.yaml"))
        .expect("attempt record must be readable");
    let sessions: Vec<&str> = text
        .lines()
        .filter_map(|line| line.strip_prefix("  session-id: "))
        .collect();
    assert_eq!(sessions, ["same-session"]);
}

#[then("protocol failure не создал выдуманную session")]
fn protocol_failure_has_no_invented_session(world: &mut LifecycleWorld) {
    let text = fs::read_to_string(run_directory(world).join("0.first.attempt.yaml"))
        .expect("attempt record must be readable");
    let sessions: Vec<&str> = text
        .lines()
        .filter_map(|line| line.strip_prefix("  session-id: "))
        .collect();
    if world.protocol_case.as_deref() == Some("contradictory") {
        assert_eq!(sessions, ["first-session"]);
    } else {
        assert!(sessions.is_empty());
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
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
    human: bool,
}

#[derive(Debug)]
enum Behavior {
    ReturnWithoutCompletion,
    Activate(Vec<String>),
    Complete { input_id: String, bytes: Vec<u8> },
    CompleteExternalSymlink,
    CompleteTwice,
    CompleteActivateComplete,
    CompleteThenInvalid,
    CompleteWithExit(i32),
    CompleteEmpty,
    ActivateThenUserExit(String),
}

#[derive(Debug)]
struct FakeAgentRegistry {
    root: PathBuf,
    behaviors: Mutex<VecDeque<Behavior>>,
    calls: Mutex<Vec<Call>>,
}

#[derive(Debug)]
struct NoNativeResumeRegistry;

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
                human: request.human,
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
            Behavior::CompleteExternalSymlink => {
                use std::os::unix::fs::symlink;

                let source = tempfile::NamedTempFile::new().map_err(|error| error.to_string())?;
                fs::write(source.path(), b"external").map_err(|error| error.to_string())?;
                let link = self.root.join("external-source-link");
                symlink(source.path(), &link).map_err(|error| error.to_string())?;
                control.complete(&[("result".to_owned(), link)])?;
            }
            Behavior::CompleteTwice => {
                let source = self.root.join("source-artifact");
                fs::write(&source, b"draft").map_err(|error| error.to_string())?;
                control.complete(&[("result".to_owned(), source.clone())])?;
                fs::write(&source, b"final").map_err(|error| error.to_string())?;
                control.complete(&[("result".to_owned(), source)])?;
            }
            Behavior::CompleteActivateComplete => {
                let source = self.root.join("source-artifact");
                fs::write(&source, b"draft").map_err(|error| error.to_string())?;
                control.complete(&[("result".to_owned(), source.clone())])?;
                control.activate_session("late-session")?;
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
                return Ok(AgentExit::Returned(code));
            }
            Behavior::CompleteEmpty => control.complete(&[])?,
            Behavior::ActivateThenUserExit(session) => {
                control.activate_session(&session)?;
                return Ok(AgentExit::UserExit);
            }
        }
        Ok(AgentExit::Returned(0))
    }
}

impl AgentRegistry for NoNativeResumeRegistry {
    fn validate(&self, _type_id: &str, _model: &str, _reasoning: &str) -> Result<(), String> {
        Err("Agent type не поддерживает native resume".to_owned())
    }

    fn run(
        &self,
        _request: &AgentRunRequest<'_>,
        _control: &mut dyn AttemptControl,
    ) -> Result<AgentExit, String> {
        panic!("Agent без native resume не должен запускаться")
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

#[derive(Debug)]
struct UserShutdownRegistry {
    branches_started: Barrier,
}

impl AgentRegistry for UserShutdownRegistry {
    fn validate(&self, type_id: &str, model: &str, reasoning: &str) -> Result<(), String> {
        if type_id == "codex" && !model.is_empty() && !reasoning.is_empty() {
            Ok(())
        } else {
            Err("невалидный user shutdown fake Agent".to_owned())
        }
    }

    fn run(
        &self,
        request: &AgentRunRequest<'_>,
        control: &mut dyn AttemptControl,
    ) -> Result<AgentExit, String> {
        match request.step_id {
            "root" => control.complete(&[])?,
            "human-branch" => {
                control.activate_session("human-session")?;
                self.branches_started.wait();
                return Ok(AgentExit::UserExit);
            }
            "worker" => {
                self.branches_started.wait();
                control.complete(&[])?;
            }
            "after" => return Err("работа после user shutdown не должна запускаться".to_owned()),
            other => return Err(format!("неизвестный Step {other}")),
        }
        Ok(AgentExit::Returned(0))
    }
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
                human: request.human,
            });
        if matches!(request.step_id, "left" | "right") {
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_active.fetch_max(active, Ordering::SeqCst);
            control.activate_session(request.step_id)?;
            self.branch_barrier.wait();
            if self.fail_left && request.step_id == "left" {
                self.active.fetch_sub(1, Ordering::SeqCst);
                return Ok(AgentExit::Returned(7));
            }
            control.complete(&[])?;
            self.active.fetch_sub(1, Ordering::SeqCst);
        } else {
            control.complete(&[])?;
        }
        Ok(AgentExit::Returned(0))
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

#[given("подготовлен single-step workflow с prompt original")]
fn workflow_with_original_prompt(world: &mut LifecycleWorld) {
    prepare_graph(
        world,
        "steps:\n  - id: first\n    agent: main\n    prompt: stable\n    human: false\n    depends-on: []\n    outputs: []\n",
        Some(("stable", "ORIGINAL")),
    );
}

#[given("подготовлен single-step workflow с обязательным parameter mode")]
fn workflow_with_required_parameter(world: &mut LifecycleWorld) {
    prepare_graph(
        world,
        "parameters:\n  mode: string\nsteps:\n  - id: first\n    agent: main\n    prompt: null\n    human: false\n    depends-on: []\n    outputs: []\n",
        None,
    );
}

#[given("путь run занят regular file")]
fn run_path_is_regular_file(world: &mut LifecycleWorld) {
    let root = world.root.as_ref().expect("scenario must define root");
    fs::write(root.path().join("run"), b"occupied").expect("run path fixture must be written");
}

#[given("подготовлен последовательный workflow без outputs")]
fn sequential_workflow_without_outputs(world: &mut LifecycleWorld) {
    prepare_graph(
        world,
        "steps:\n  - id: root\n    agent: main\n    prompt: null\n    human: false\n    depends-on: []\n    outputs: []\n  - id: target\n    agent: main\n    prompt: null\n    human: false\n    depends-on: [root]\n    outputs: []\n",
        None,
    );
}

#[given("подготовлен single-step workflow с output result")]
fn workflow_with_output(world: &mut LifecycleWorld) {
    prepare_workflow(world, &["result"]);
}

#[given("подготовлен незавершённый run без session activations")]
fn unfinished_run_without_session(world: &mut LifecycleWorld) {
    prepare_workflow(world, &[]);
    run_start(world, [Behavior::ReturnWithoutCompletion]);
    world.calls.clear();
    world.observed = None;
    world.durable_snapshot = durable_snapshot(world);
}

#[given("run содержит оставшийся unlocked lock-файл")]
fn run_contains_unlocked_lock_file(world: &mut LifecycleWorld) {
    assert!(run_directory(world).join("active.lock").is_file());
}

#[given("unlocked lock-файл содержит произвольные bytes")]
fn unlocked_lock_file_contains_arbitrary_bytes(world: &mut LifecycleWorld) {
    fs::write(run_directory(world).join("active.lock"), [0x00, 0xff, 0x80])
        .expect("unlocked lock file must accept arbitrary bytes");
}

#[given("подготовлен single-step human workflow")]
fn human_workflow(world: &mut LifecycleWorld) {
    prepare_graph(
        world,
        "steps:\n  - id: human-step\n    agent: main\n    prompt: null\n    human: true\n    depends-on: []\n    outputs: []\n",
        None,
    );
}

#[given("подготовлен workflow с двумя готовыми human Steps")]
fn workflow_with_two_ready_human_steps(world: &mut LifecycleWorld) {
    prepare_graph(
        world,
        "steps:\n  - id: root\n    agent: main\n    prompt: null\n    human: false\n    depends-on: []\n    outputs: []\n  - id: first\n    agent: main\n    prompt: null\n    human: true\n    depends-on: [root]\n    outputs: []\n  - id: second\n    agent: main\n    prompt: null\n    human: true\n    depends-on: [root]\n    outputs: []\n",
        None,
    );
    let root = world.root.as_ref().expect("scenario must define root");
    fs::write(
        root.path().join("config.yaml"),
        "default-agent: main\nmax-parallel-agents: 2\nagents:\n  main:\n    type: codex\n    model: model\n    reasoning: high\n",
    )
    .expect("human order config must be written");
}

#[given("подготовлен workflow с human и non-human ветвями")]
fn human_and_non_human_workflow(world: &mut LifecycleWorld) {
    prepare_graph(
        world,
        "steps:\n  - id: root\n    agent: main\n    prompt: null\n    human: false\n    depends-on: []\n    outputs: []\n  - id: human-branch\n    agent: main\n    prompt: null\n    human: true\n    depends-on: [root]\n    outputs: []\n  - id: worker\n    agent: main\n    prompt: null\n    human: false\n    depends-on: [root]\n    outputs: []\n  - id: after\n    agent: main\n    prompt: null\n    human: false\n    depends-on: [worker]\n    outputs: []\n",
        None,
    );
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

#[given("подготовлен циклический workflow a → b → c → a")]
fn cyclic_workflow(world: &mut LifecycleWorld) {
    prepare_graph(
        world,
        "steps:\n  - id: a\n    agent: main\n    prompt: null\n    human: false\n    depends-on: [c]\n    outputs: [result]\n  - id: b\n    agent: main\n    prompt: null\n    human: false\n    depends-on: [a]\n    outputs: [result]\n  - id: c\n    agent: main\n    prompt: null\n    human: false\n    depends-on: [b]\n    outputs: [result]\n",
        None,
    );
}

#[given("подготовлен cycle с side dependency и внешним consumer")]
fn mixed_cycle_workflow(world: &mut LifecycleWorld) {
    prepare_graph(
        world,
        "steps:\n  - id: a\n    agent: main\n    prompt: null\n    human: false\n    depends-on: [c, side]\n    outputs: [forward]\n  - id: sink\n    agent: main\n    prompt: null\n    human: false\n    depends-on: [c]\n    outputs: []\n  - id: b\n    agent: main\n    prompt: null\n    human: false\n    depends-on: [a]\n    outputs: [bridge]\n  - id: side\n    agent: main\n    prompt: null\n    human: false\n    depends-on: [a]\n    outputs: [context]\n  - id: c\n    agent: main\n    prompt: null\n    human: false\n    depends-on: [b]\n    outputs: [feedback]\n",
        None,
    );
    let root = world.root.as_ref().expect("scenario must define root");
    fs::write(
        root.path().join("config.yaml"),
        "default-agent: main\nmax-parallel-agents: 1\nagents:\n  main:\n    type: codex\n    model: model\n    reasoning: high\n",
    )
    .expect("mixed cycle config must be written");
}

#[given("подготовлен durable run с частично удовлетворёнными dependency groups")]
fn partially_satisfied_durable_run(world: &mut LifecycleWorld) {
    prepare_durable_run(
        world,
        "workflow-id: delivery\nmax-parallel-agents: 5\nsteps:\n- id: a\n  agent: &agent\n    type: codex\n    model: model\n    reasoning: high\n  prompt: null\n  human: false\n  depends-on: [b, c]\n  outputs: []\n- id: b\n  agent: *agent\n  prompt: null\n  human: false\n  depends-on: [a, c]\n  outputs: []\n- id: c\n  agent: *agent\n  prompt: null\n  human: false\n  depends-on: [a, b]\n  outputs: []\n",
        &[(
            "0.a.attempt.yaml",
            "input: []\nevents:\n- type: completed\n",
        )],
    );
}

#[given("подготовлен durable run с полностью удовлетворённой dependency group")]
fn fully_satisfied_durable_run(world: &mut LifecycleWorld) {
    prepare_satisfied_dependency_run(
        world,
        &[
            (
                "0.root.attempt.yaml",
                "input: []\nevents:\n- type: completed\n",
            ),
            (
                "1.left.attempt.yaml",
                "input:\n- 0\nevents:\n- type: completed\n",
            ),
            (
                "2.right.attempt.yaml",
                "input:\n- 0\nevents:\n- type: completed\n",
            ),
        ],
    );
}

#[given("подготовлен ready durable run с attempts 0, 2 и 4")]
fn ready_run_with_attempt_number_gaps(world: &mut LifecycleWorld) {
    prepare_satisfied_dependency_run(
        world,
        &[
            (
                "0.root.attempt.yaml",
                "input: []\nevents:\n- type: completed\n",
            ),
            (
                "2.left.attempt.yaml",
                "input:\n- 0\nevents:\n- type: completed\n",
            ),
            (
                "4.right.attempt.yaml",
                "input:\n- 0\nevents:\n- type: completed\n",
            ),
        ],
    );
}

#[given("подготовлен циклический durable run без attempt records")]
fn durable_run_without_attempt_records(world: &mut LifecycleWorld) {
    prepare_durable_run(
        world,
        "workflow-id: delivery\nmax-parallel-agents: 5\nsteps:\n- id: a\n  agent: &agent\n    type: codex\n    model: model\n    reasoning: high\n  prompt: null\n  human: false\n  depends-on: [c]\n  outputs: [result]\n- id: b\n  agent: *agent\n  prompt: null\n  human: false\n  depends-on: [a]\n  outputs: [result]\n- id: c\n  agent: *agent\n  prompt: null\n  human: false\n  depends-on: [b]\n  outputs: [result]\n",
        &[],
    );
}

#[given("подготовлен циклический durable run со старой input group")]
fn cyclic_durable_run_with_stale_input(world: &mut LifecycleWorld) {
    prepare_durable_run(
        world,
        "workflow-id: delivery\nmax-parallel-agents: 5\nsteps:\n- id: a\n  agent: &agent\n    type: codex\n    model: model\n    reasoning: high\n  prompt: null\n  human: false\n  depends-on: [b]\n  outputs: [result]\n- id: b\n  agent: *agent\n  prompt: null\n  human: false\n  depends-on: [a]\n  outputs: [result]\n",
        &[
            (
                "0.a.attempt.yaml",
                "input: []\nevents:\n- type: completed\n",
            ),
            (
                "1.b.attempt.yaml",
                "input:\n- 0\nevents:\n- type: completed\n",
            ),
            (
                "2.a.attempt.yaml",
                "input:\n- 1\nevents:\n- type: completed\n",
            ),
            (
                "3.b.attempt.yaml",
                "input:\n- 2\nevents:\n- type: completed\n",
            ),
            (
                "4.a.attempt.yaml",
                "input:\n- 1\nevents:\n- type: completed\n",
            ),
        ],
    );
    let directory = run_directory(world);
    for name in [
        "0.a.result.artifact",
        "1.b.result.artifact",
        "2.a.result.artifact",
        "3.b.result.artifact",
        "4.a.result.artifact",
    ] {
        fs::write(directory.join(name), name).expect("cycle artifact must be written");
    }
}

fn prepare_satisfied_dependency_run(world: &mut LifecycleWorld, attempts: &[(&str, &str)]) {
    prepare_durable_run(
        world,
        "workflow-id: delivery\nmax-parallel-agents: 5\nsteps:\n- id: root\n  agent: &agent\n    type: codex\n    model: model\n    reasoning: high\n  prompt: null\n  human: false\n  depends-on: []\n  outputs: []\n- id: left\n  agent: *agent\n  prompt: null\n  human: false\n  depends-on: [root]\n  outputs: []\n- id: right\n  agent: *agent\n  prompt: null\n  human: false\n  depends-on: [root]\n  outputs: []\n- id: join\n  agent: *agent\n  prompt: null\n  human: false\n  depends-on: [left, right]\n  outputs: []\n",
        attempts,
    );
}

#[given("подготовлен завершённый линейный durable run")]
fn completed_linear_durable_run(world: &mut LifecycleWorld) {
    linear_workflow(world);
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
    world.calls.clear();
    world.observed = None;
}

#[given(expr = "durable-модель повреждена как {string}")]
#[allow(clippy::needless_pass_by_value)]
fn corrupt_durable_model(world: &mut LifecycleWorld, corruption: String) {
    let directory = run_directory(world);
    if corrupt_attempt_record(&directory, &corruption) {
        return;
    }
    match corruption.as_str() {
        "отсутствующий spec" => {
            fs::remove_file(directory.join("spec.yaml")).expect("spec must be removed");
        }
        "невалидный spec YAML" => {
            fs::write(directory.join("spec.yaml"), b"steps: [")
                .expect("invalid spec must be written");
        }
        "отсутствующий completed artifact" => {
            fs::remove_file(directory.join("0.source.result.artifact"))
                .expect("completed artifact must be removed");
        }
        "дополнительный completed artifact" => {
            fs::write(directory.join("0.source.extra.artifact"), b"extra")
                .expect("extra artifact must be written");
        }
        "неполная input group" => {
            fs::write(
                directory.join("1.target.attempt.yaml"),
                b"input: []\nevents:\n- type: completed\n",
            )
            .expect("incomplete input must be written");
        }
        "отсутствующий input source" => {
            fs::write(
                directory.join("1.target.attempt.yaml"),
                b"input:\n- 99\nevents:\n- type: completed\n",
            )
            .expect("missing input source must be written");
        }
        "незавершённый input source" => {
            fs::write(
                directory.join("0.source.attempt.yaml"),
                b"input: []\nevents: []\n",
            )
            .expect("unfinished input source must be written");
        }
        "противоречивый input" => {
            fs::write(
                directory.join("1.target.attempt.yaml"),
                b"input:\n- 1\nevents:\n- type: completed\n",
            )
            .expect("contradictory input must be written");
        }
        "повторный глобальный attempt number" => {
            fs::write(
                directory.join("1.source.attempt.yaml"),
                b"input: []\nevents:\n- type: completed\n",
            )
            .expect("duplicate attempt number must be written");
        }
        "невалидный content artifact" => {
            fs::write(directory.join("0.source.result.artifact"), [0xff])
                .expect("invalid content artifact must be written");
        }
        other => panic!("unknown durable corruption {other}"),
    }
}

fn corrupt_attempt_record(directory: &Path, corruption: &str) -> bool {
    let record = match corruption {
        "невалидный attempt record" => Some("events: ["),
        "attempt record без input" => Some("events: []\n"),
        "attempt record с неизвестным полем" => {
            Some("input: [0]\nevents: []\nstatus: running\n")
        }
        "attempt record с duplicate root key" => Some("input: [0]\ninput: [0]\nevents: []\n"),
        "input не sequence" => Some("input: 0\nevents: []\n"),
        "events не sequence" => Some("input: [0]\nevents: completed\n"),
        "неизвестный event type" => Some("input: [0]\nevents:\n- type: paused\n"),
        "session event без session-id" => {
            Some("input: [0]\nevents:\n- type: session-activated\n")
        }
        "completion event с лишним полем" => {
            Some("input: [0]\nevents:\n- type: completed\n  status: done\n")
        }
        "соседний повтор session activation" => Some(
            "input: [0]\nevents:\n- type: session-activated\n  session-id: same\n- type: session-activated\n  session-id: same\n",
        ),
        "completed не является последним" => Some(
            "input: [0]\nevents:\n- type: completed\n- type: session-activated\n  session-id: later\n",
        ),
        "completed повторяется" => {
            Some("input: [0]\nevents:\n- type: completed\n- type: completed\n")
        }
        "initial attempt с непустым input" => {
            fs::write(
                directory.join("0.source.attempt.yaml"),
                "input: [0]\nevents:\n- type: completed\n",
            )
            .expect("contradictory initial attempt must be written");
            return true;
        }
        _ => None,
    };
    if let Some(record) = record {
        fs::write(directory.join("1.target.attempt.yaml"), record)
            .expect("invalid attempt record must be written");
        return true;
    }
    let invalid_name = match corruption {
        "невалидное имя attempt record" => Some("invalid.attempt.yaml"),
        "нечисловой номер attempt" => Some("x.target.attempt.yaml"),
        "attempt с неизвестным Step в имени" => Some("9.missing.attempt.yaml"),
        _ => None,
    };
    if let Some(name) = invalid_name {
        fs::write(directory.join(name), "input: []\nevents: []\n")
            .expect("invalid named attempt must be written");
        return true;
    }
    if corruption == "attempt record не regular file" {
        fs::create_dir(directory.join("9.target.attempt.yaml"))
            .expect("attempt directory must be created");
        return true;
    }
    false
}

#[given("подготовлен незавершённый durable run с crash leftovers")]
fn unfinished_run_with_crash_leftovers(world: &mut LifecycleWorld) {
    prepare_workflow(world, &["result"]);
    run_start(world, [Behavior::ReturnWithoutCompletion]);
    let directory = run_directory(world);
    fs::write(directory.join("0.first.result.artifact"), b"partial")
        .expect("unfinished artifact must be written");
    fs::write(directory.join("99.ghost.result.artifact"), b"orphan")
        .expect("orphan artifact must be written");
    fs::write(
        directory.join(".0.first.attempt.yaml.123.0.tmp"),
        b"temporary",
    )
    .expect("temporary file must be written");
    world.calls.clear();
    world.observed = None;
    world.durable_snapshot = durable_snapshot(world);
}

#[given("подготовлен ready durable run с orphan artifact 99")]
fn ready_run_with_orphan_artifact(world: &mut LifecycleWorld) {
    fully_satisfied_durable_run(world);
    fs::write(
        run_directory(world).join("99.ghost.result.artifact"),
        b"orphan",
    )
    .expect("orphan artifact must be written");
}

#[given("process Agent не должен запускаться")]
fn process_agent_must_not_run(world: &mut LifecycleWorld) {
    prepare_process_agent(world, "#!/bin/sh\nexit 99\n");
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
    prepare_gated_process_agent(
        world,
        "#!/bin/sh\nprintf '%s' \"$$\" > \"$ORC_HOME/lock-agent.pid\"\n: > \"$ORC_TEST_READY\"\nIFS= read -r ignored < \"$ORC_TEST_GATE\"\n",
    );
}

#[given("process Agent завершает initial Step и ожидает на target")]
fn process_agent_waits_on_target(world: &mut LifecycleWorld) {
    prepare_gated_process_agent(
        world,
        "#!/bin/sh\ncase \"$ORC_STEP_ID\" in\n  root) \"$ORC_TEST_ORCHESTRATOR\" attempt complete ;;\n  target) printf '%s' \"$$\" > \"$ORC_HOME/lock-agent.pid\"; : > \"$ORC_TEST_READY\"; IFS= read -r ignored < \"$ORC_TEST_GATE\" ;;\nesac\n",
    );
}

fn prepare_gated_process_agent(world: &mut LifecycleWorld, script: &str) {
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
    prepare_process_agent(world, script);
}

#[given("process Agent завершает attempt без outputs")]
fn process_agent_completes_without_outputs(world: &mut LifecycleWorld) {
    prepare_process_agent(
        world,
        "#!/bin/sh\n\"$ORC_TEST_ORCHESTRATOR\" attempt complete\n",
    );
}

#[given("process Agent передаёт completion draft и блокируется до возврата")]
fn process_agent_blocks_after_draft_completion(world: &mut LifecycleWorld) {
    let root = world.root.as_ref().expect("scenario must define root");
    let gate = root.path().join("completion-crash-gate");
    let status = Command::new("mkfifo")
        .arg(&gate)
        .status()
        .expect("mkfifo must run");
    assert!(status.success());
    world.process_gate = Some(gate);
    world.process_ready = Some(root.path().join("completion-crash-ready"));
    prepare_process_agent(
        world,
        "#!/bin/sh\nprintf '%s' \"$$\" > \"$ORC_HOME/crash-agent.pid\"\nprintf '%s' \"$ORC_RUN_ID\" > \"$ORC_HOME/saved.run-id\"\nprintf draft > \"$ORC_HOME/crash-source\"\n\"$ORC_TEST_ORCHESTRATOR\" attempt complete --artifact result \"$ORC_HOME/crash-source\"\n: > \"$ORC_TEST_READY\"\nIFS= read -r ignored < \"$ORC_TEST_GATE\"\n",
    );
}

#[given("process Agent ожидает termination signal")]
fn process_agent_waits_for_signal(world: &mut LifecycleWorld) {
    prepare_signal_gate(world);
    prepare_process_agent(
        world,
        "#!/bin/sh\ntrap 'printf HUP > \"$ORC_TEST_SIGNALLED\"; exit 0' HUP\ntrap 'printf INT > \"$ORC_TEST_SIGNALLED\"; exit 0' INT\ntrap 'printf TERM > \"$ORC_TEST_SIGNALLED\"; exit 0' TERM\n: > \"$ORC_TEST_READY\"\nwhile :; do IFS= read -r ignored < \"$ORC_TEST_GATE\" || :; done\n",
    );
}

#[given("process Agent игнорирует первый SIGTERM")]
fn process_agent_ignores_first_term(world: &mut LifecycleWorld) {
    prepare_signal_gate(world);
    prepare_process_agent(
        world,
        "#!/bin/sh\ntrap ': > \"$ORC_TEST_SIGNALLED\"' TERM\n: > \"$ORC_TEST_READY\"\nwhile :; do IFS= read -r ignored < \"$ORC_TEST_GATE\" || :; done\n",
    );
}

#[given("process Agent публикует artifact final через attempt complete")]
fn process_agent_with_completion(world: &mut LifecycleWorld) {
    prepare_process_agent(
        world,
        "#!/bin/sh\nprintf final > \"$ORC_TEST_ARTIFACT\"\n\"$ORC_TEST_ORCHESTRATOR\" attempt complete --artifact result \"$ORC_TEST_ARTIFACT\"\n",
    );
}

#[given(expr = "process Agent отправляет completion с чужим control context {string}")]
#[allow(clippy::needless_pass_by_value)]
fn process_agent_with_foreign_control_context(world: &mut LifecycleWorld, context: String) {
    let root = world.root.as_ref().expect("scenario must define root");
    world.control_code = Some(root.path().join("control-code"));
    let environment = match context.as_str() {
        "RunId" => "ORC_RUN_ID=999999",
        "attempt" => "ORC_ATTEMPT=999999",
        other => panic!("unknown foreign control context: {other}"),
    };
    prepare_process_agent(
        world,
        &format!(
            "#!/bin/sh\nprintf final > \"$ORC_TEST_ARTIFACT\"\n{environment} \"$ORC_TEST_ORCHESTRATOR\" attempt complete --artifact result \"$ORC_TEST_ARTIFACT\"\nprintf %s $? > \"$ORC_TEST_CONTROL_CODE\"\n"
        ),
    );
}

#[given("process Agent сохраняет control context и возвращается")]
fn process_agent_saves_control_context(world: &mut LifecycleWorld) {
    prepare_process_agent(
        world,
        "#!/bin/sh\nprintf '%s' \"$ORC_CONTROL_ENDPOINT\" > \"$ORC_HOME/saved.endpoint\"\nprintf '%s' \"$ORC_RUN_ID\" > \"$ORC_HOME/saved.run-id\"\nprintf '%s' \"$ORC_ATTEMPT\" > \"$ORC_HOME/saved.attempt\"\n",
    );
}

#[when(expr = "запускается agent-facing команда {string} с явным selector {string}")]
#[allow(clippy::needless_pass_by_value)]
fn run_agent_facing_command_with_explicit_control_selector(
    world: &mut LifecycleWorld,
    command: String,
    selector: String,
) {
    let root = world.root.as_ref().expect("scenario must define root");
    let output = Command::new(env!("CARGO_BIN_EXE_orchestrator"))
        .args(command.split_whitespace())
        .args(selector.split_whitespace())
        .env("ORC_HOME", root.path())
        .env_remove("ORC_CONTROL_ENDPOINT")
        .env_remove("ORC_RUN_ID")
        .env_remove("ORC_ATTEMPT")
        .output()
        .expect("agent-facing command must run");
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

#[given("подготовлен process Agent блокирующийся после сохранения control context")]
fn process_agent_blocks_after_saving_control_context(world: &mut LifecycleWorld) {
    workflow_without_outputs(world);
    let root = world.root.as_ref().expect("scenario must define root");
    let gate = root.path().join("crash-gate");
    let status = Command::new("mkfifo")
        .arg(&gate)
        .status()
        .expect("mkfifo must run");
    assert!(status.success());
    world.process_gate = Some(gate);
    world.process_ready = Some(root.path().join("crash-ready"));
    prepare_process_agent(
        world,
        "#!/bin/sh\nprintf '%s' \"$$\" > \"$ORC_HOME/crash-agent.pid\"\nprintf '%s' \"$ORC_CONTROL_ENDPOINT\" > \"$ORC_HOME/stale.endpoint\"\nprintf '%s' \"$ORC_RUN_ID\" > \"$ORC_HOME/saved.run-id\"\n: > \"$ORC_TEST_READY\"\nIFS= read -r ignored < \"$ORC_TEST_GATE\"\n",
    );
}

#[then(
    "дочерний orchestrator использовал унаследованные ORC_HOME, ORC_CONTROL_ENDPOINT, ORC_RUN_ID и ORC_ATTEMPT"
)]
fn child_orchestrator_used_inherited_control_context(world: &mut LifecycleWorld) {
    assert_eq!(
        world.observed().exit_code,
        0,
        "{:?}",
        world.observed().error
    );
    assert!(
        run_directory(world)
            .join("0.first.result.artifact")
            .is_file()
    );
}

#[given("process human Agent проверяет stdin и stdout TTY")]
fn process_human_agent_checks_terminal(world: &mut LifecycleWorld) {
    prepare_process_agent(
        world,
        "#!/bin/sh\ntest -t 0 && test -t 1 || exit 9\n: > \"$ORC_TEST_TTY_CONFIRMED\"\n\"$ORC_TEST_ORCHESTRATOR\" attempt complete\n",
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
        "#!/bin/sh\ntest \"$ORC_RESUME_SESSION\" = process-session || exit 9\ntest \"$1|$2|$3|$4|$5|$6|$7|$8|$9\" = 'exec|resume|--json|--model|model|--config|model_reasoning_effort=\"high\"|process-session|' || exit 9\n\"$ORC_TEST_ORCHESTRATOR\" attempt complete\n",
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

#[when("start materialize'ит workflow, а source definitions изменяются перед resume")]
fn resume_after_source_definitions_change(world: &mut LifecycleWorld) {
    run_start(world, [Behavior::ReturnWithoutCompletion]);
    assert_eq!(
        world.observed().exit_code,
        1,
        "{:?}",
        world.observed().error
    );
    let spec = fs::read(run_directory(world).join("spec.yaml")).expect("spec must be readable");
    world.durable_snapshot = vec![("spec.yaml".to_owned(), spec)];
    let root = world.root.as_ref().expect("scenario must define root");
    fs::write(root.path().join("config.yaml"), "unknown: true\n")
        .expect("changed config must be written");
    fs::write(root.path().join("workflow/delivery.yaml"), "steps: []\n")
        .expect("changed workflow must be written");
    fs::write(root.path().join("prompt/stable.md"), "CHANGED")
        .expect("changed prompt must be written");
    let (observed, calls) = resume_with_fake(world, [Behavior::ReturnWithoutCompletion]);
    world.observed = Some(observed);
    world.calls = calls;
}

#[when("lifecycle API получает parameter mode с NUL")]
fn start_with_nul_parameter(world: &mut LifecycleWorld) {
    let root = world.root.as_ref().expect("scenario must define root");
    let registry = FakeAgentRegistry::new(root.path().to_owned(), []);
    let command = LifecycleCommand::start_explicit_with_parameters(
        "delivery",
        BTreeMap::from([("mode".to_owned(), "before\0after".to_owned())]),
    )
    .expect("workflow ID must be valid");
    let mut reporter = VecReporter::default();
    let result = execute_lifecycle(
        &command,
        &environment(root),
        TerminalMode::Unavailable,
        &LifecycleSignals::default(),
        &registry,
        &mut reporter,
    );
    world.calls = registry
        .calls
        .into_inner()
        .expect("call log must be available");
    world.observed = Some(observe(result, reporter));
}

#[when("workflow запускается через lifecycle API с Agent type без native resume")]
fn start_without_native_resume_support(world: &mut LifecycleWorld) {
    let root = world.root.as_ref().expect("scenario must define root");
    let mut reporter = VecReporter::default();
    let result = execute_lifecycle(
        &LifecycleCommand::start_explicit("delivery").expect("workflow ID must be valid"),
        &environment(root),
        TerminalMode::Unavailable,
        &LifecycleSignals::default(),
        &NoNativeResumeRegistry,
        &mut reporter,
    );
    world.observed = Some(observe(result, reporter));
}

#[when("human workflow запускается без TTY")]
fn start_human_without_terminal(world: &mut LifecycleWorld) {
    run_start(world, [Behavior::CompleteEmpty]);
}

#[when("human workflow запускается с доступным TTY")]
fn start_human_with_terminal(world: &mut LifecycleWorld) {
    run_start_with_terminal(world, [Behavior::CompleteEmpty], TerminalMode::Available);
}

#[when("human Steps планируются с доступным TTY")]
fn schedule_human_steps_with_terminal(world: &mut LifecycleWorld) {
    run_start_with_terminal(
        world,
        [
            Behavior::CompleteEmpty,
            Behavior::CompleteEmpty,
            Behavior::ReturnWithoutCompletion,
        ],
        TerminalMode::Available,
    );
}

#[when("human Agent активирует session human-session и выполняет /exit")]
fn human_agent_user_exit(world: &mut LifecycleWorld) {
    run_start_with_terminal(
        world,
        [Behavior::ActivateThenUserExit("human-session".to_owned())],
        TerminalMode::Available,
    );
}

#[when("human ветвь выполняет /exit одновременно с completion worker")]
fn user_exit_with_parallel_completion(world: &mut LifecycleWorld) {
    let root = world.root.as_ref().expect("scenario must define root");
    let registry = UserShutdownRegistry {
        branches_started: Barrier::new(2),
    };
    let mut reporter = VecReporter::default();
    let result = execute_lifecycle(
        &LifecycleCommand::start_explicit("delivery").expect("workflow ID must be valid"),
        &environment(root),
        TerminalMode::Available,
        &LifecycleSignals::default(),
        &registry,
        &mut reporter,
    );
    world.run_id = reporter
        .0
        .iter()
        .find_map(|line| line.strip_prefix("Run "))
        .filter(|value| !value.ends_with(" exited"))
        .map(str::to_owned);
    world.observed = Some(observe(result, reporter));
}

#[when("human run продолжается с TTY и завершается")]
fn resume_human_with_terminal(world: &mut LifecycleWorld) {
    let root = world.root.as_ref().expect("scenario must define root");
    let registry = FakeAgentRegistry::new(root.path().to_owned(), [Behavior::CompleteEmpty]);
    let run_id = world.run_id.as_ref().expect("scenario must define run ID");
    let command =
        LifecycleCommand::Resume(orchestrator::RunId::parse(run_id).expect("run ID must be valid"));
    let mut reporter = VecReporter::default();
    let result = execute_lifecycle(
        &command,
        &environment(root),
        TerminalMode::Available,
        &LifecycleSignals::default(),
        &registry,
        &mut reporter,
    );
    world.calls = registry
        .calls
        .into_inner()
        .expect("call log must be available");
    world.observed = Some(observe(result, reporter));
}

#[when(
    "Agent активирует opaque sessions vendor/a:1, vendor/b:2, vendor/a:1, vendor/a:1 и возвращается без completion"
)]
fn activate_opaque_sessions(world: &mut LifecycleWorld) {
    run_start(
        world,
        [Behavior::Activate(vec![
            "vendor/a:1".to_owned(),
            "vendor/b:2".to_owned(),
            "vendor/a:1".to_owned(),
            "vendor/a:1".to_owned(),
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

#[when("Agent передаёт completion с произвольными binary bytes и возвращает управление")]
fn complete_with_binary_artifact(world: &mut LifecycleWorld) {
    run_start(
        world,
        [Behavior::Complete {
            input_id: "result".to_owned(),
            bytes: vec![0x00, 0xff, 0x0a, 0x80],
        }],
    );
}

#[when("Agent передаёт completion без artifacts и возвращает управление")]
fn complete_without_artifacts(world: &mut LifecycleWorld) {
    run_start(world, [Behavior::CompleteEmpty]);
}

#[when("Agent передаёт external source через absolute symbolic link и возвращает управление")]
fn complete_from_external_symlink(world: &mut LifecycleWorld) {
    run_start(world, [Behavior::CompleteExternalSymlink]);
}

#[when("Agent передаёт completion сначала с bytes draft, затем final и возвращает управление")]
fn replace_completion_candidate(world: &mut LifecycleWorld) {
    run_start(world, [Behavior::CompleteTwice]);
}

#[when(
    "Agent передаёт completion draft, активирует session late-session, передаёт completion final и возвращается"
)]
fn replace_completion_after_session_activation(world: &mut LifecycleWorld) {
    run_start(world, [Behavior::CompleteActivateComplete]);
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
    use std::os::unix::fs::{FileTypeExt, PermissionsExt};

    let root = world
        .root
        .as_ref()
        .expect("scenario must define root")
        .path()
        .to_owned();
    let agent = world
        .process_agent
        .as_ref()
        .expect("scenario must define process Agent");
    let child = Command::new(env!("CARGO_BIN_EXE_orchestrator"))
        .args(["start", "delivery"])
        .env("ORC_HOME", &root)
        .env("ORC_AGENT_COMMAND", agent)
        .env("ORC_TEST_ORCHESTRATOR", env!("CARGO_BIN_EXE_orchestrator"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("orchestrator must start");
    wait_for_path(&root.join("left.ready"));
    wait_for_path(&root.join("right.ready"));
    let endpoint = PathBuf::from(
        fs::read_to_string(root.join("left.endpoint")).expect("control endpoint must be readable"),
    );
    let metadata = fs::metadata(&endpoint).expect("active control endpoint must exist");
    world.control_endpoint_observation = Some((
        endpoint,
        metadata.file_type().is_socket(),
        metadata.permissions().mode() & 0o777,
    ));
    fs::write(root.join("left.gate"), b"fail\n").expect("left Agent must be released");
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
        TerminalMode::Unavailable,
        &LifecycleSignals::default(),
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

#[when("цикл доходит до незавершённой повторной activation a")]
fn cycle_reaches_unfinished_repeated_a(world: &mut LifecycleWorld) {
    run_start(
        world,
        [
            Behavior::Complete {
                input_id: "result".to_owned(),
                bytes: b"a0".to_vec(),
            },
            Behavior::Complete {
                input_id: "result".to_owned(),
                bytes: b"b1".to_vec(),
            },
            Behavior::Complete {
                input_id: "result".to_owned(),
                bytes: b"c2".to_vec(),
            },
            Behavior::ReturnWithoutCompletion,
        ],
    );
}

#[when("mixed cycle завершает повторный a и доходит до следующего незавершённого b")]
fn mixed_cycle_reaches_next_unfinished_b(world: &mut LifecycleWorld) {
    run_start(
        world,
        [
            Behavior::Complete {
                input_id: "forward".to_owned(),
                bytes: b"a0".to_vec(),
            },
            Behavior::Complete {
                input_id: "bridge".to_owned(),
                bytes: b"b1".to_vec(),
            },
            Behavior::Complete {
                input_id: "context".to_owned(),
                bytes: b"side2".to_vec(),
            },
            Behavior::Complete {
                input_id: "feedback".to_owned(),
                bytes: b"c3".to_vec(),
            },
            Behavior::Complete {
                input_id: "forward".to_owned(),
                bytes: b"a4".to_vec(),
            },
            Behavior::CompleteEmpty,
            Behavior::ReturnWithoutCompletion,
        ],
    );
}

#[when("незавершённая повторная activation a продолжается через resume")]
fn resume_unfinished_repeated_a(world: &mut LifecycleWorld) {
    cycle_reaches_unfinished_repeated_a(world);
    let root = world.root.as_ref().expect("scenario must define root");
    let registry = FakeAgentRegistry::new(
        root.path().to_owned(),
        [
            Behavior::Complete {
                input_id: "result".to_owned(),
                bytes: b"a3".to_vec(),
            },
            Behavior::ReturnWithoutCompletion,
        ],
    );
    let run_id = world.run_id.as_ref().expect("scenario must define run ID");
    let command =
        LifecycleCommand::Resume(orchestrator::RunId::parse(run_id).expect("run ID must be valid"));
    let mut reporter = VecReporter::default();
    let result = execute_lifecycle(
        &command,
        &environment(root),
        TerminalMode::Unavailable,
        &LifecycleSignals::default(),
        &registry,
        &mut reporter,
    );
    world.calls = registry
        .calls
        .into_inner()
        .expect("call log must be available");
    world.observed = Some(observe(result, reporter));
}

#[when("blocked run дважды продолжается через lifecycle API")]
fn resume_blocked_run_twice(world: &mut LifecycleWorld) {
    world.durable_snapshot = durable_snapshot(world);
    let (first, first_calls) = resume_with_fake(world, []);
    let (second, second_calls) = resume_with_fake(world, []);
    world.prior_observed = Some(first);
    world.observed = Some(second);
    world.calls = first_calls.into_iter().chain(second_calls).collect();
}

#[when("готовый target продолжается через lifecycle API")]
fn resume_ready_target(world: &mut LifecycleWorld) {
    let (observed, calls) = resume_with_fake(world, [Behavior::ReturnWithoutCompletion]);
    world.observed = Some(observed);
    world.calls = calls;
}

#[when("blocked run продолжается через CLI")]
fn resume_blocked_run_through_cli(world: &mut LifecycleWorld) {
    let root = world.root.as_ref().expect("scenario must define root");
    let agent = world
        .process_agent
        .as_ref()
        .expect("scenario must define process Agent");
    let output = Command::new(env!("CARGO_BIN_EXE_orchestrator"))
        .args(["resume", "0"])
        .env("ORC_HOME", root.path())
        .env("ORC_AGENT_COMMAND", agent)
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

#[when("повреждённый run продолжается через lifecycle API")]
fn resume_corrupted_run(world: &mut LifecycleWorld) {
    world.durable_snapshot = durable_snapshot(world);
    let (observed, calls) = resume_with_fake(world, []);
    world.observed = Some(observed);
    world.calls = calls;
}

#[when("run с crash leftovers продолжается через lifecycle API")]
fn resume_run_with_crash_leftovers(world: &mut LifecycleWorld) {
    let (observed, calls) = resume_with_fake(world, [Behavior::ReturnWithoutCompletion]);
    world.observed = Some(observed);
    world.calls = calls;
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
    let result = execute_lifecycle(
        &command,
        &environment(root),
        TerminalMode::Unavailable,
        &LifecycleSignals::default(),
        &registry,
        &mut reporter,
    );
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

#[when("дочерняя session activate вызывается после завершения supervisor")]
fn activate_session_after_supervisor_exit(world: &mut LifecycleWorld) {
    start_through_process(world);
    assert_eq!(
        world.observed().exit_code,
        1,
        "{:?}",
        world.observed().error
    );
    world.durable_snapshot = durable_snapshot(world);
    let root = world.root.as_ref().expect("scenario must define root");
    let endpoint = PathBuf::from(
        fs::read_to_string(root.path().join("saved.endpoint"))
            .expect("saved endpoint must be readable"),
    );
    let run_id =
        fs::read_to_string(root.path().join("saved.run-id")).expect("saved RunId must be readable");
    let attempt = fs::read_to_string(root.path().join("saved.attempt"))
        .expect("saved attempt must be readable");
    let output = Command::new(env!("CARGO_BIN_EXE_orchestrator"))
        .args(["session", "activate", "late-session"])
        .env("ORC_HOME", root.path())
        .env("ORC_CONTROL_ENDPOINT", &endpoint)
        .env("ORC_RUN_ID", run_id)
        .env("ORC_ATTEMPT", attempt)
        .output()
        .expect("child session activate must run");
    world.stale_control_endpoint = Some(endpoint);
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

#[when("supervisor завершается SIGKILL и run продолжается новым supervisor")]
fn resume_after_supervisor_sigkill(world: &mut LifecycleWorld) {
    let root = world
        .root
        .as_ref()
        .expect("scenario must define root")
        .path()
        .to_owned();
    let agent = world
        .process_agent
        .as_ref()
        .expect("scenario must define process Agent");
    let gate = world
        .process_gate
        .as_ref()
        .expect("scenario must define process gate");
    let ready = world
        .process_ready
        .as_ref()
        .expect("scenario must define ready marker");
    let child = Command::new(env!("CARGO_BIN_EXE_orchestrator"))
        .args(["start", "delivery"])
        .env("ORC_HOME", &root)
        .env("ORC_AGENT_COMMAND", agent)
        .env("ORC_TEST_ORCHESTRATOR", env!("CARGO_BIN_EXE_orchestrator"))
        .env("ORC_TEST_GATE", gate)
        .env("ORC_TEST_READY", ready)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("orchestrator must start");
    let mut processes = ProcessTreeGuard::new(child, root.join("crash-agent.pid"));
    wait_for_path(ready);
    let old_endpoint = PathBuf::from(
        fs::read_to_string(root.join("stale.endpoint"))
            .expect("stale endpoint path must be readable"),
    );
    let run_id =
        fs::read_to_string(root.join("saved.run-id")).expect("saved RunId must be readable");
    let agent_pid =
        fs::read_to_string(root.join("crash-agent.pid")).expect("Agent pid must be readable");
    send_signal(processes.id(), "KILL");
    send_signal(
        agent_pid.trim().parse().expect("Agent pid must be numeric"),
        "KILL",
    );
    let first_output = processes.wait_with_output();
    assert!(first_output.status.code().is_none());
    world.run_id = Some(run_id);
    world.stale_endpoint_existed = old_endpoint.exists();
    world.stale_control_endpoint = Some(old_endpoint);
    prepare_process_agent(
        world,
        "#!/bin/sh\nprintf '%s' \"$ORC_CONTROL_ENDPOINT\" > \"$ORC_HOME/new.endpoint\"\n\"$ORC_TEST_ORCHESTRATOR\" attempt complete\n",
    );
    resume_through_process(world);
    world.new_control_endpoint = Some(PathBuf::from(
        fs::read_to_string(root.join("new.endpoint")).expect("new endpoint path must be readable"),
    ));
}

#[when("supervisor завершается SIGKILL до возврата Agent, а resume передаёт final")]
fn replace_volatile_completion_after_sigkill(world: &mut LifecycleWorld) {
    let root = world
        .root
        .as_ref()
        .expect("scenario must define root")
        .path()
        .to_owned();
    let agent = world
        .process_agent
        .as_ref()
        .expect("scenario must define process Agent");
    let gate = world
        .process_gate
        .as_ref()
        .expect("scenario must define process gate");
    let ready = world
        .process_ready
        .as_ref()
        .expect("scenario must define ready marker");
    let child = Command::new(env!("CARGO_BIN_EXE_orchestrator"))
        .args(["start", "delivery"])
        .env("ORC_HOME", &root)
        .env("ORC_AGENT_COMMAND", agent)
        .env("ORC_TEST_ORCHESTRATOR", env!("CARGO_BIN_EXE_orchestrator"))
        .env("ORC_TEST_GATE", gate)
        .env("ORC_TEST_READY", ready)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("orchestrator must start");
    let mut processes = ProcessTreeGuard::new(child, root.join("crash-agent.pid"));
    wait_for_path(ready);
    let run_id =
        fs::read_to_string(root.join("saved.run-id")).expect("saved RunId must be readable");
    let agent_pid =
        fs::read_to_string(root.join("crash-agent.pid")).expect("Agent pid must be readable");
    send_signal(processes.id(), "KILL");
    send_signal(
        agent_pid.trim().parse().expect("Agent pid must be numeric"),
        "KILL",
    );
    let first_output = processes.wait_with_output();
    assert!(first_output.status.code().is_none());
    world.run_id = Some(run_id);
    let (observed, calls) = resume_with_fake(
        world,
        [Behavior::Complete {
            input_id: "result".to_owned(),
            bytes: b"final".to_vec(),
        }],
    );
    world.observed = Some(observed);
    world.calls = calls;
}

fn resume_through_process(world: &mut LifecycleWorld) {
    let root = world.root.as_ref().expect("scenario must define root");
    let agent = world
        .process_agent
        .as_ref()
        .expect("scenario must define process Agent");
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

#[when("human workflow запускается через системный pseudo-terminal")]
#[when("workflow запускается через pseudo-terminal")]
fn start_human_through_terminal(world: &mut LifecycleWorld) {
    let root = world.root.as_ref().expect("scenario must define root");
    let agent = world
        .process_agent
        .as_ref()
        .expect("scenario must define process Agent");
    let output = Command::new("/usr/bin/script")
        .args([
            "-q",
            "-e",
            "/dev/null",
            env!("CARGO_BIN_EXE_orchestrator"),
            "start",
            "delivery",
        ])
        .env("ORC_HOME", root.path())
        .env("ORC_AGENT_COMMAND", agent)
        .env("ORC_TEST_ORCHESTRATOR", env!("CARGO_BIN_EXE_orchestrator"))
        .env("ORC_TEST_TTY_CONFIRMED", root.path().join("tty-confirmed"))
        .output()
        .expect("pseudo-terminal command must run");
    let lines: Vec<String> = String::from_utf8(output.stdout)
        .expect("stdout must be UTF-8")
        .lines()
        .map(|line| line.trim_end_matches('\r').to_owned())
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

fn resume_human_through_process_terminal(world: &mut LifecycleWorld) {
    let root = world.root.as_ref().expect("scenario must define root");
    let agent = world
        .process_agent
        .as_ref()
        .expect("scenario must define process Agent");
    let run_id = world.run_id.as_ref().expect("scenario must define run ID");
    let output = Command::new("/usr/bin/script")
        .args([
            "-q",
            "-e",
            "/dev/null",
            env!("CARGO_BIN_EXE_orchestrator"),
            "resume",
            run_id,
        ])
        .env("ORC_HOME", root.path())
        .env("ORC_AGENT_COMMAND", agent)
        .env("ORC_TEST_ORCHESTRATOR", env!("CARGO_BIN_EXE_orchestrator"))
        .output()
        .expect("pseudo-terminal resume must run");
    world.observed = Some(Observed {
        exit_code: u8::try_from(output.status.code().expect("process must exit normally"))
            .expect("fixture exit code must fit u8"),
        lines: String::from_utf8(output.stdout)
            .expect("stdout must be UTF-8")
            .lines()
            .map(|line| line.trim_end_matches('\r').to_owned())
            .collect(),
        error: Some(String::from_utf8_lossy(&output.stderr).into_owned()),
    });
}

#[when(expr = "supervisor получает {word}")]
#[allow(clippy::needless_pass_by_value)]
fn supervisor_receives_signal(world: &mut LifecycleWorld, signal: String) {
    run_process_signal_shutdown(world, &signal, false);
}

#[when("supervisor получает второй SIGTERM")]
fn supervisor_receives_second_term(world: &mut LifecycleWorld) {
    run_process_signal_shutdown(world, "TERM", true);
}

#[when("после одного SIGTERM истекает константный shutdown deadline")]
fn supervisor_signal_timeout(world: &mut LifecycleWorld) {
    run_process_signal_shutdown(world, "TERM", false);
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
        .env("ORC_TEST_ORCHESTRATOR", env!("CARGO_BIN_EXE_orchestrator"))
        .env("ORC_TEST_GATE", gate)
        .env("ORC_TEST_READY", ready)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("first orchestrator must start");
    let mut first = ProcessTreeGuard::new(first, root.path().join("lock-agent.pid"));
    wait_for_path(ready);
    let run_id = fs::read_dir(root.path().join("run"))
        .expect("run root must be readable")
        .filter_map(Result::ok)
        .find(|entry| entry.path().is_dir())
        .expect("run must be reserved")
        .file_name()
        .to_string_lossy()
        .into_owned();
    world.run_id = Some(run_id.clone());
    let target_attempt = run_directory(world).join("1.target.attempt.yaml");
    if target_attempt.exists() {
        wait_for_file_content(&target_attempt, "session-activated");
    }
    world.durable_snapshot = durable_snapshot(world);
    let competing = Command::new(env!("CARGO_BIN_EXE_orchestrator"))
        .args(["resume", &run_id])
        .env("ORC_HOME", root.path())
        .env("ORC_AGENT_COMMAND", agent)
        .output()
        .expect("competing orchestrator must run");
    world.competing_durable_unchanged = durable_snapshot(world) == world.durable_snapshot;
    fs::write(gate, b"release\n").expect("Agent must be released");
    let first_output = first.wait_with_output();
    assert_eq!(first_output.status.code(), Some(1));
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

#[when("одновременно запускаются два orchestrator start delivery")]
fn two_starts_run_concurrently(world: &mut LifecycleWorld) {
    let root = world.root.as_ref().expect("scenario must define root");
    let agent = world
        .process_agent
        .as_ref()
        .expect("scenario must define process Agent");
    let spawn = || {
        Command::new(env!("CARGO_BIN_EXE_orchestrator"))
            .args(["start", "delivery"])
            .env("ORC_HOME", root.path())
            .env("ORC_AGENT_COMMAND", agent)
            .env("ORC_TEST_ORCHESTRATOR", env!("CARGO_BIN_EXE_orchestrator"))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("orchestrator start must run")
    };
    let mut children = [
        ProcessTreeGuard::new(spawn(), root.path().join("missing-agent-1.pid")),
        ProcessTreeGuard::new(spawn(), root.path().join("missing-agent-2.pid")),
    ];
    let observed: Vec<Observed> = children
        .iter_mut()
        .map(|child| observe_process_output(child.wait_with_output()))
        .collect();
    let run_ids = observed
        .iter()
        .map(|result| {
            result
                .lines
                .iter()
                .find_map(|line| line.strip_prefix("Run "))
                .filter(|line| !line.ends_with(" exited"))
                .expect("start must report RunId")
                .to_owned()
        })
        .collect();
    world.concurrent_observed = observed;
    world.concurrent_run_ids = run_ids;
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
    let result = execute_lifecycle(
        &command,
        &environment(root),
        TerminalMode::Unavailable,
        &LifecycleSignals::default(),
        &registry,
        &mut reporter,
    );
    world.calls = registry
        .calls
        .into_inner()
        .expect("call log must be available");
    world.observed = Some(observe(result, reporter));
}

#[when("run продолжается с Agent type без native resume")]
fn resume_without_native_resume_support(world: &mut LifecycleWorld) {
    let root = world.root.as_ref().expect("scenario must define root");
    let run_id = world.run_id.as_ref().expect("scenario must define run ID");
    let command =
        LifecycleCommand::Resume(orchestrator::RunId::parse(run_id).expect("run ID must be valid"));
    let mut reporter = VecReporter::default();
    let result = execute_lifecycle(
        &command,
        &environment(root),
        TerminalMode::Unavailable,
        &LifecycleSignals::default(),
        &NoNativeResumeRegistry,
        &mut reporter,
    );
    world.observed = Some(observe(result, reporter));
}

#[when("запускается orchestrator resume без RunId")]
fn resume_without_run_id(world: &mut LifecycleWorld) {
    let root = world.root.as_ref().expect("scenario must define root");
    let output = Command::new(env!("CARGO_BIN_EXE_orchestrator"))
        .arg("resume")
        .env("ORC_HOME", root.path())
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

#[when("run после signal shutdown продолжается и завершается")]
fn resume_after_signal_shutdown(world: &mut LifecycleWorld) {
    let root = world.root.as_ref().expect("scenario must define root");
    let registry = FakeAgentRegistry::new(root.path().to_owned(), [Behavior::CompleteEmpty]);
    let run_id = world.run_id.as_ref().expect("scenario must define run ID");
    let command =
        LifecycleCommand::Resume(orchestrator::RunId::parse(run_id).expect("run ID must be valid"));
    let mut reporter = VecReporter::default();
    let result = execute_lifecycle(
        &command,
        &environment(root),
        TerminalMode::Unavailable,
        &LifecycleSignals::default(),
        &registry,
        &mut reporter,
    );
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
            human: false,
        }],
        "{:?}",
        world.observed().error
    );
}

#[then("durable spec содержит закрытую Agent Step schema без source references")]
fn durable_spec_has_closed_agent_schema(world: &mut LifecycleWorld) {
    let actual: serde_yaml::Value = serde_yaml::from_slice(
        &fs::read(run_directory(world).join("spec.yaml")).expect("spec must be readable"),
    )
    .expect("spec must be YAML");
    let expected: serde_yaml::Value = serde_yaml::from_str(
        "workflow-id: delivery\nmax-parallel-agents: 5\nparameters: {}\nsteps:\n  - id: first\n    agent:\n      type: codex\n      model: model\n      reasoning: high\n    prompt: null\n    human: false\n    process: null\n    depends-on: []\n    outputs: []\n",
    )
    .expect("expected spec must be YAML");
    assert_eq!(actual, expected);
}

#[then("initial attempt record содержит только пустые input и events")]
fn initial_attempt_has_exact_pending_schema(world: &mut LifecycleWorld) {
    assert_eq!(attempt_record_names(world), ["0.first.attempt.yaml"]);
    let actual: serde_yaml::Value = serde_yaml::from_slice(
        &fs::read(run_directory(world).join("0.first.attempt.yaml"))
            .expect("initial attempt must be readable"),
    )
    .expect("initial attempt must be YAML");
    let expected: serde_yaml::Value =
        serde_yaml::from_str("input: []\nevents: []\n").expect("expected attempt must be YAML");
    assert_eq!(actual, expected);
}

#[then("resume использует сохранённые Agent, prompt и topology, а spec неизменен")]
fn resume_uses_materialized_source_snapshot(world: &mut LifecycleWorld) {
    assert_eq!(world.calls.len(), 1);
    assert_eq!(world.calls[0].step_id, "first");
    assert_eq!(world.calls[0].prompt, "ORIGINAL");
    let expected = &world
        .durable_snapshot
        .first()
        .expect("original spec must be captured")
        .1;
    let actual = fs::read(run_directory(world).join("spec.yaml")).expect("spec must be readable");
    assert_eq!(&actual, expected);
}

#[then("initial attempt остаётся незавершённым")]
fn attempt_unfinished(world: &mut LifecycleWorld) {
    let text = fs::read_to_string(run_directory(world).join("0.first.attempt.yaml"))
        .expect("attempt record must be readable");
    assert!(!text.contains("completed"));
}

#[then("Agent запускался ровно один раз")]
fn agent_started_once(world: &mut LifecycleWorld) {
    assert_eq!(world.calls.len(), 1);
}

#[then("lifecycle run не создан")]
fn lifecycle_run_was_not_created(world: &mut LifecycleWorld) {
    let root = world.root.as_ref().expect("scenario must define root");
    assert!(!root.path().join("run").exists());
}

#[then("Agent не запускался и regular file run не изменился")]
fn reservation_failure_has_no_side_effects(world: &mut LifecycleWorld) {
    assert!(world.calls.is_empty());
    let root = world.root.as_ref().expect("scenario must define root");
    assert_eq!(
        fs::read(root.path().join("run")).expect("run path fixture must be readable"),
        b"occupied"
    );
}

#[then("competing resume не изменяет initial attempt")]
fn competing_resume_does_not_mutate(world: &mut LifecycleWorld) {
    attempt_unfinished(world);
}

#[then("competing resume не изменяет durable run")]
fn competing_resume_does_not_change_durable_run(world: &mut LifecycleWorld) {
    assert!(world.competing_durable_unchanged);
}

#[then("оба lifecycle завершаются с кодом 0")]
fn both_lifecycles_complete_successfully(world: &mut LifecycleWorld) {
    assert_eq!(world.concurrent_observed.len(), 2);
    assert!(
        world
            .concurrent_observed
            .iter()
            .all(|result| result.exit_code == 0)
    );
}

#[then("start публикуют два разных RunId и два независимых durable run")]
fn concurrent_starts_publish_distinct_runs(world: &mut LifecycleWorld) {
    assert_eq!(world.concurrent_run_ids.len(), 2);
    assert_ne!(world.concurrent_run_ids[0], world.concurrent_run_ids[1]);
    let root = world.root.as_ref().expect("scenario must define root");
    for run_id in &world.concurrent_run_ids {
        let directory = root.path().join("run").join(run_id);
        assert!(directory.join("spec.yaml").is_file());
        let attempt = fs::read_to_string(directory.join("0.first.attempt.yaml"))
            .expect("completed attempt must be readable");
        assert!(attempt.contains("type: completed"));
    }
}

#[then("resume запускает незавершённый target attempt 1")]
fn resume_runs_unfinished_target_attempt(world: &mut LifecycleWorld) {
    assert_eq!(world.calls.len(), 1);
    assert_eq!(world.calls[0].step_id, "target");
    assert_eq!(world.calls[0].attempt, 1);
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

#[then("Agent process group получила тот же signal")]
fn agent_received_same_signal(world: &mut LifecycleWorld) {
    let root = world.root.as_ref().expect("scenario must define root");
    let received = fs::read_to_string(root.path().join("signal-received"))
        .expect("signal marker must be readable");
    assert_eq!(
        received,
        *world.sent_signal.as_ref().expect("signal must be sent")
    );
}

#[then("последняя lifecycle строка сообщает exited")]
fn last_line_reports_exited(world: &mut LifecycleWorld) {
    assert!(
        world
            .observed()
            .lines
            .last()
            .is_some_and(|line| line.ends_with(" exited"))
    );
}

#[then("shutdown эскалирован до SIGKILL без изменения кода")]
fn shutdown_escalated(world: &mut LifecycleWorld) {
    assert_eq!(world.observed().exit_code, 143);
    let root = world.root.as_ref().expect("scenario must define root");
    assert!(root.path().join("signal-received").is_file());
}

#[then("эскалация произошла не раньше 10 секунд")]
fn shutdown_waited_ten_seconds(world: &mut LifecycleWorld) {
    assert!(
        world
            .shutdown_elapsed
            .is_some_and(|elapsed| elapsed >= Duration::from_secs(10))
    );
}

#[then("resume запускает тот же attempt 0 с session vendor/a:1")]
fn resumed_same_attempt(world: &mut LifecycleWorld) {
    assert_eq!(
        world.calls,
        vec![Call {
            step_id: "first".to_owned(),
            attempt: 0,
            resume_session: Some("vendor/a:1".to_owned()),
            inputs: Vec::new(),
            prompt: String::new(),
            human: false,
        }],
        "{:?}",
        world.observed().error
    );
}

#[then("resume запускает тот же attempt 0 без session activation")]
fn resumed_same_attempt_without_session(world: &mut LifecycleWorld) {
    assert_eq!(
        world.calls,
        vec![Call {
            step_id: "first".to_owned(),
            attempt: 0,
            resume_session: None,
            inputs: Vec::new(),
            prompt: String::new(),
            human: false,
        }],
        "{:?}",
        world.observed().error
    );
}

#[then("durable activations равны vendor/a:1, vendor/b:2, vendor/a:1")]
fn durable_opaque_activations(world: &mut LifecycleWorld) {
    let text = fs::read_to_string(run_directory(world).join("0.first.attempt.yaml"))
        .expect("attempt record must be readable");
    let sessions: Vec<&str> = text
        .lines()
        .filter_map(|line| line.strip_prefix("  session-id: "))
        .collect();
    assert_eq!(sessions, ["vendor/a:1", "vendor/b:2", "vendor/a:1"]);
}

#[then("durable activations не содержат вид create, resume или fork")]
fn durable_activations_have_no_origin_kind(world: &mut LifecycleWorld) {
    let text = fs::read_to_string(run_directory(world).join("0.first.attempt.yaml"))
        .expect("attempt record must be readable");
    assert!(!text.contains("create"));
    assert!(!text.contains("resume"));
    assert!(!text.contains("fork"));
}

#[then("RunId является десятичным Unix timestamp создания в миллисекундах")]
fn run_id_is_creation_timestamp_ms(world: &mut LifecycleWorld) {
    let run_id = world
        .run_id
        .as_deref()
        .expect("scenario must create a run")
        .parse::<u128>()
        .expect("RunId must be decimal");
    let (started, finished) = world
        .start_window_ms
        .expect("scenario must capture the start time window");
    assert!((started..=finished).contains(&run_id));
}

#[then("durable activations равны process-session")]
fn durable_process_activation(world: &mut LifecycleWorld) {
    let text = fs::read_to_string(run_directory(world).join("0.first.attempt.yaml"))
        .expect("attempt record must be readable");
    assert!(
        text.contains("session-id: process-session"),
        "attempt={text}; observed={:?}",
        world.observed
    );
}

#[then("session activation не создала второй run")]
fn session_activation_did_not_create_another_run(world: &mut LifecycleWorld) {
    let root = world.root.as_ref().expect("scenario must define root");
    let run_count = fs::read_dir(root.path().join("run"))
        .expect("run root must be readable")
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .count();
    assert_eq!(run_count, 1);
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

#[then("durable artifact result содержит bytes external")]
fn artifact_has_external_bytes(world: &mut LifecycleWorld) {
    assert_eq!(
        fs::read(run_directory(world).join("0.first.result.artifact"))
            .expect("artifact must be readable"),
        b"external"
    );
}

#[then(
    "единственный durable artifact является regular file 0.first.result.artifact с исходными binary bytes"
)]
fn artifact_has_exact_name_and_binary_bytes(world: &mut LifecycleWorld) {
    let directory = run_directory(world);
    let path = directory.join("0.first.result.artifact");
    assert!(
        fs::symlink_metadata(&path)
            .expect("artifact metadata must be readable")
            .file_type()
            .is_file()
    );
    assert_eq!(
        fs::read(&path).expect("artifact must be readable"),
        [0x00, 0xff, 0x0a, 0x80]
    );
    let mut artifacts = fs::read_dir(directory)
        .expect("run directory must be readable")
        .map(|entry| {
            entry
                .expect("durable entry must be readable")
                .file_name()
                .into_string()
                .expect("durable entry name must be UTF-8")
        })
        .filter(|name| name.ends_with(".artifact"))
        .collect::<Vec<_>>();
    artifacts.sort();
    assert_eq!(artifacts, ["0.first.result.artifact"]);
}

#[then("отдельный output.yaml не создан")]
fn output_marker_is_absent(world: &mut LifecycleWorld) {
    let output_markers = fs::read_dir(run_directory(world))
        .expect("run directory must be readable")
        .map(|entry| {
            entry
                .expect("durable entry must be readable")
                .file_name()
                .into_string()
                .expect("durable entry name must be UTF-8")
        })
        .filter(|name| name.ends_with(".output.yaml"))
        .collect::<Vec<_>>();
    assert!(output_markers.is_empty(), "markers={output_markers:?}");
}

#[then("symbolic link остался caller-owned после удаления source")]
fn external_source_is_not_durable(world: &mut LifecycleWorld) {
    let root = world.root.as_ref().expect("scenario must define root");
    let link = root.path().join("external-source-link");
    assert!(
        fs::symlink_metadata(&link)
            .expect("source symbolic link must remain")
            .file_type()
            .is_symlink()
    );
    assert!(!link.exists());
}

#[then("source draft остался caller-owned вне durable run")]
fn crash_source_remains_outside_durable_run(world: &mut LifecycleWorld) {
    let root = world.root.as_ref().expect("scenario must define root");
    assert_eq!(
        fs::read(root.path().join("crash-source")).expect("crash source must remain readable"),
        b"draft"
    );
}

#[then("durable attempt содержит late-session перед completed")]
fn durable_attempt_contains_late_session_before_completion(world: &mut LifecycleWorld) {
    let record = fs::read_to_string(run_directory(world).join("0.first.attempt.yaml"))
        .expect("attempt record must be readable");
    let session = record
        .find("session-id: late-session")
        .expect("late session must be durable");
    let completed = record
        .find("type: completed")
        .expect("completed must be durable");
    assert!(session < completed);
}

#[then("completed attempt record содержит только input и ordered events")]
fn completed_attempt_has_exact_ordered_events(world: &mut LifecycleWorld) {
    let actual: serde_yaml::Value = serde_yaml::from_slice(
        &fs::read(run_directory(world).join("0.first.attempt.yaml"))
            .expect("completed attempt must be readable"),
    )
    .expect("completed attempt must be YAML");
    let expected: serde_yaml::Value = serde_yaml::from_str(
        "input: []\nevents:\n- type: session-activated\n  session-id: late-session\n- type: completed\n",
    )
    .expect("expected completed attempt must be YAML");
    assert_eq!(actual, expected);
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

#[then("дочерний control call завершился с кодом 5")]
fn child_control_call_is_rejected(world: &mut LifecycleWorld) {
    let path = world
        .control_code
        .as_ref()
        .expect("scenario must define control code path");
    assert_eq!(
        fs::read_to_string(path).expect("control code must be readable"),
        "5"
    );
}

#[then("чужой completion не добавил completed или artifact")]
fn foreign_completion_does_not_change_attempt(world: &mut LifecycleWorld) {
    let directory = run_directory(world);
    let record = fs::read_to_string(directory.join("0.first.attempt.yaml"))
        .expect("attempt record must be readable");
    assert!(!record.contains("completed"));
    assert!(!directory.join("0.first.result.artifact").exists());
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
    let record: serde_yaml::Value = serde_yaml::from_slice(
        &fs::read(run_directory(world).join("3.join.attempt.yaml"))
            .expect("join attempt must be readable"),
    )
    .expect("join attempt must be YAML");
    assert_eq!(record["input"][0].as_u64(), Some(1));
    assert_eq!(record["input"][1].as_u64(), Some(2));
    assert_eq!(record["input"].as_sequence().map(Vec::len), Some(2));
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

#[then("attempts созданы как 0 a, 1 b, 2 c, 3 a")]
fn first_cycle_attempt_order(world: &mut LifecycleWorld) {
    assert_eq!(
        attempt_record_names(world),
        [
            "0.a.attempt.yaml",
            "1.b.attempt.yaml",
            "2.c.attempt.yaml",
            "3.a.attempt.yaml",
        ]
    );
    let calls: Vec<(&str, u64)> = world
        .calls
        .iter()
        .map(|call| (call.step_id.as_str(), call.attempt))
        .collect();
    assert_eq!(calls, [("a", 0), ("b", 1), ("c", 2), ("a", 3)]);
}

#[then("bootstrap a получает пустой input, а повторный a получает input 2")]
fn repeated_a_uses_feedback_input(world: &mut LifecycleWorld) {
    let directory = run_directory(world);
    let bootstrap = fs::read_to_string(directory.join("0.a.attempt.yaml"))
        .expect("bootstrap attempt must be readable");
    let repeated = fs::read_to_string(directory.join("3.a.attempt.yaml"))
        .expect("repeated attempt must be readable");
    assert!(bootstrap.contains("input: []"));
    assert!(repeated.contains("input:\n- 2"));
    assert!(world.calls[0].inputs.is_empty());
    assert_eq!(
        input_versions(&world.calls[3]),
        ["c:result:2.c.result.artifact"]
    );
}

#[then("каждый завершённый Step передаёт следующему свежий artifact")]
fn cycle_passes_fresh_artifacts(world: &mut LifecycleWorld) {
    assert_eq!(
        input_versions(&world.calls[1]),
        ["a:result:0.a.result.artifact"]
    );
    assert_eq!(
        input_versions(&world.calls[2]),
        ["b:result:1.b.result.artifact"]
    );
    assert_eq!(
        input_versions(&world.calls[3]),
        ["c:result:2.c.result.artifact"]
    );
}

#[then("mixed cycle создал attempts 0 a, 1 b, 2 side, 3 c, 4 a, 5 sink, 6 b, 7 side")]
fn mixed_cycle_attempt_order(world: &mut LifecycleWorld) {
    assert_eq!(
        attempt_record_names(world),
        [
            "0.a.attempt.yaml",
            "1.b.attempt.yaml",
            "2.side.attempt.yaml",
            "3.c.attempt.yaml",
            "4.a.attempt.yaml",
            "5.sink.attempt.yaml",
            "6.b.attempt.yaml",
            "7.side.attempt.yaml",
        ]
    );
}

#[then("повторный a получает feedback c и context side, а sink получает feedback c")]
fn mixed_cycle_keeps_independent_inputs_and_outputs(world: &mut LifecycleWorld) {
    let repeated_a = world
        .calls
        .iter()
        .find(|call| call.step_id == "a" && call.attempt == 4)
        .expect("repeated a must be called");
    assert_eq!(
        input_versions(repeated_a),
        [
            "c:feedback:3.c.feedback.artifact",
            "side:context:2.side.context.artifact"
        ]
    );
    let sink = world
        .calls
        .iter()
        .find(|call| call.step_id == "sink")
        .expect("external sink must be called");
    assert_eq!(input_versions(sink), ["c:feedback:3.c.feedback.artifact"]);
}

#[then("lifecycle не сообщает о завершении run")]
fn does_not_report_completed(world: &mut LifecycleWorld) {
    assert!(
        world
            .observed()
            .lines
            .iter()
            .all(|line| !line.contains(": completed"))
    );
}

#[then("resume запускает attempts 3 a, 4 b")]
fn resume_runs_repeated_a_and_next_b(world: &mut LifecycleWorld) {
    let calls: Vec<(&str, u64)> = world
        .calls
        .iter()
        .map(|call| (call.step_id.as_str(), call.attempt))
        .collect();
    assert_eq!(calls, [("a", 3), ("b", 4)]);
    assert_eq!(
        attempt_record_names(world),
        [
            "0.a.attempt.yaml",
            "1.b.attempt.yaml",
            "2.c.attempt.yaml",
            "3.a.attempt.yaml",
            "4.b.attempt.yaml",
        ]
    );
}

#[then("следующий b получает input 3")]
fn next_b_uses_repeated_a(world: &mut LifecycleWorld) {
    let record = fs::read_to_string(run_directory(world).join("4.b.attempt.yaml"))
        .expect("next b attempt must be readable");
    assert!(record.contains("input:\n- 3"));
    assert_eq!(
        input_versions(&world.calls[1]),
        ["a:result:3.a.result.artifact"]
    );
}

#[then("attempts первого обхода и их artifacts не изменены")]
fn first_cycle_history_is_immutable(world: &mut LifecycleWorld) {
    let directory = run_directory(world);
    for (name, input, artifact) in [
        ("0.a", "input: []", b"a0".as_slice()),
        ("1.b", "input:\n- 0", b"b1".as_slice()),
        ("2.c", "input:\n- 1", b"c2".as_slice()),
    ] {
        let record = fs::read_to_string(directory.join(format!("{name}.attempt.yaml")))
            .expect("historical attempt must be readable");
        assert!(record.contains(input));
        assert!(record.trim_end().ends_with("type: completed"));
        assert_eq!(
            fs::read(directory.join(format!("{name}.result.artifact")))
                .expect("historical artifact must be readable"),
            artifact
        );
    }
}

#[then("оба resume завершаются с кодом 1 и одинаковой диагностикой")]
fn repeated_blocked_resume_is_stable(world: &mut LifecycleWorld) {
    let first = world
        .prior_observed
        .as_ref()
        .expect("scenario must execute first resume");
    assert_eq!(first.exit_code, 1);
    assert_eq!(world.observed().exit_code, 1);
    assert_eq!(first.error, world.observed().error);
}

#[then("диагностика перечисляет отсутствующие source Steps c, b")]
fn blocked_diagnostic_lists_missing_sources(world: &mut LifecycleWorld) {
    let error = world
        .observed()
        .error
        .as_deref()
        .expect("blocked resume must return diagnostics");
    assert!(error.contains("blocked: отсутствуют source Steps c, b"));
    assert_eq!(error.matches("source Steps").count(), 1);
}

#[then("Agent не запускался и durable run не изменился")]
fn blocked_resume_has_no_side_effects(world: &mut LifecycleWorld) {
    assert!(world.calls.is_empty());
    assert_eq!(durable_snapshot(world), world.durable_snapshot);
}

#[then("создан ровно один join attempt с input 1, 2")]
fn exactly_one_ready_join_is_published(world: &mut LifecycleWorld) {
    assert_eq!(
        attempt_record_names(world),
        [
            "0.root.attempt.yaml",
            "1.left.attempt.yaml",
            "2.right.attempt.yaml",
            "3.join.attempt.yaml",
        ]
    );
    let record = fs::read_to_string(run_directory(world).join("3.join.attempt.yaml"))
        .expect("join attempt must be readable");
    assert!(record.contains("input:\n- 1\n- 2"));
    assert_eq!(world.calls.len(), 1);
    assert_eq!(world.calls[0].step_id, "join");
    assert_eq!(world.calls[0].attempt, 3);
}

#[then("stderr сообщает blocked и отсутствующие source Steps c, b")]
fn cli_reports_blocked_sources(world: &mut LifecycleWorld) {
    assert_eq!(
        world.observed().error.as_deref(),
        Some(
            "error: resume: run 0: blocked: отсутствуют source Steps c, b: workflow frontier blocked\n"
        )
    );
}

#[then("Agent не запускался и повреждённый durable run не изменился")]
fn invalid_run_has_no_side_effects(world: &mut LifecycleWorld) {
    assert!(world.calls.is_empty());
    assert_eq!(durable_snapshot(world), world.durable_snapshot);
}

#[then("resume продолжает исходный unfinished attempt")]
fn resume_continues_original_unfinished_attempt(world: &mut LifecycleWorld) {
    assert_eq!(world.calls.len(), 1);
    assert_eq!(world.calls[0].step_id, "first");
    assert_eq!(world.calls[0].attempt, 0);
}

#[then("crash leftovers остались побайтово неизменными")]
fn crash_leftovers_remain_unchanged(world: &mut LifecycleWorld) {
    assert_eq!(durable_snapshot(world), world.durable_snapshot);
}

#[then("target получает следующий свободный глобальный номер 100")]
fn target_skips_orphan_attempt_number(world: &mut LifecycleWorld) {
    assert_eq!(world.calls.len(), 1);
    assert_eq!(world.calls[0].step_id, "join");
    assert_eq!(world.calls[0].attempt, 100);
    let record = fs::read_to_string(run_directory(world).join("100.join.attempt.yaml"))
        .expect("join attempt must be readable");
    assert!(record.contains("input:\n- 1\n- 2"));
    assert!(!run_directory(world).join("3.join.attempt.yaml").exists());
}

#[then("новый join attempt получает номер 5 без заполнения пропусков 1 и 3")]
fn next_attempt_does_not_fill_global_number_gaps(world: &mut LifecycleWorld) {
    assert_eq!(world.calls.len(), 1);
    assert_eq!(world.calls[0].step_id, "join");
    assert_eq!(world.calls[0].attempt, 5);
    assert_eq!(
        attempt_record_names(world),
        [
            "0.root.attempt.yaml",
            "2.left.attempt.yaml",
            "4.right.attempt.yaml",
            "5.join.attempt.yaml",
        ]
    );
}

#[then("resume публикует и запускает первый attempt 0")]
fn resume_publishes_first_attempt_zero(world: &mut LifecycleWorld) {
    assert_eq!(world.calls.len(), 1);
    assert_eq!(world.calls[0].step_id, "a");
    assert_eq!(world.calls[0].attempt, 0);
    assert_eq!(attempt_record_names(world), ["0.a.attempt.yaml"]);
    let record = fs::read_to_string(run_directory(world).join("0.a.attempt.yaml"))
        .expect("initial attempt must be readable");
    assert!(record.contains("input: []"));
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

#[then("общий endpoint был Unix socket с правами 600 и удалён после supervisor")]
fn shared_endpoint_is_protected_and_removed(world: &mut LifecycleWorld) {
    let (path, was_socket, mode) = world
        .control_endpoint_observation
        .as_ref()
        .expect("scenario must observe control endpoint");
    assert!(*was_socket);
    assert_eq!(*mode, 0o600);
    assert!(!path.exists());
}

#[then("закрытый control endpoint удалён")]
fn closed_control_endpoint_is_removed(world: &mut LifecycleWorld) {
    let endpoint = world
        .stale_control_endpoint
        .as_ref()
        .expect("scenario must save control endpoint");
    assert!(!endpoint.exists());
}

#[then("stale endpoint существовал после crash, но удалён при resume")]
fn stale_endpoint_is_removed_on_resume(world: &mut LifecycleWorld) {
    assert!(world.stale_endpoint_existed);
    assert!(
        !world
            .stale_control_endpoint
            .as_ref()
            .expect("scenario must save stale endpoint")
            .exists()
    );
}

#[then("новый endpoint отличается от stale и удалён после supervisor")]
fn resumed_supervisor_uses_new_endpoint(world: &mut LifecycleWorld) {
    let stale = world
        .stale_control_endpoint
        .as_ref()
        .expect("scenario must save stale endpoint");
    let new = world
        .new_control_endpoint
        .as_ref()
        .expect("scenario must save new endpoint");
    assert_ne!(stale, new);
    assert!(!new.exists());
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

#[then("human Agent не запускался")]
fn human_agent_not_started(world: &mut LifecycleWorld) {
    assert!(world.calls.is_empty());
}

#[then("Agent получил human terminal mode")]
fn agent_received_human_mode(world: &mut LifecycleWorld) {
    assert_eq!(world.calls.len(), 1);
    assert!(world.calls[0].human);
    assert_eq!(world.calls[0].step_id, "human-step");
}

#[then("process human Agent подтвердил прямой TTY")]
fn process_human_confirmed_terminal(world: &mut LifecycleWorld) {
    let root = world.root.as_ref().expect("scenario must define root");
    assert!(root.path().join("tty-confirmed").is_file());
}

#[then("lifecycle сообщает interrupted и команду resume")]
fn reports_user_interruption(world: &mut LifecycleWorld) {
    let run_id = world.run_id.as_ref().expect("scenario must define run ID");
    assert!(
        world
            .observed()
            .lines
            .iter()
            .any(|line| line == &format!("run {run_id}: interrupted by user"))
    );
    assert!(
        world
            .observed()
            .lines
            .iter()
            .any(|line| line == &format!("resume: orchestrator resume {run_id}"))
    );
}

#[then("human attempt остаётся незавершённым")]
fn human_attempt_unfinished(world: &mut LifecycleWorld) {
    let record = fs::read_to_string(run_directory(world).join("0.human-step.attempt.yaml"))
        .expect("human attempt must be readable");
    assert!(!record.contains("completed"));
}

#[then("human attempts запущены по порядку first, second")]
fn human_attempts_follow_workflow_order(world: &mut LifecycleWorld) {
    let human_calls: Vec<&str> = world
        .calls
        .iter()
        .filter(|call| call.human)
        .map(|call| call.step_id.as_str())
        .collect();
    assert_eq!(human_calls, ["first", "second"]);
}

#[then("resume продолжил attempt 0 с session human-session")]
fn resumed_human_session(world: &mut LifecycleWorld) {
    assert_eq!(world.calls.len(), 1);
    assert_eq!(world.calls[0].attempt, 0);
    assert_eq!(
        world.calls[0].resume_session.as_deref(),
        Some("human-session")
    );
    assert!(world.calls[0].human);
}

#[then("worker completion durable, а следующая activation не создана")]
fn worker_completed_without_next_activation(world: &mut LifecycleWorld) {
    let directory = run_directory(world);
    let worker = fs::read_to_string(directory.join("2.worker.attempt.yaml"))
        .expect("worker attempt must be readable");
    assert!(worker.trim_end().ends_with("type: completed"));
    assert!(!directory.join("3.after.attempt.yaml").exists());
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

fn prepare_durable_run(world: &mut LifecycleWorld, spec: &str, attempts: &[(&str, &str)]) {
    let root = TempDir::new().expect("test root must be created");
    let directory = root.path().join("run/0");
    fs::create_dir_all(&directory).expect("run directory must be created");
    fs::write(directory.join("spec.yaml"), spec).expect("materialized workflow must be written");
    fs::write(directory.join("active.lock"), []).expect("run lock file must be written");
    for (name, record) in attempts {
        fs::write(directory.join(name), record).expect("attempt record must be written");
    }
    world.root = Some(root);
    world.run_id = Some("0".to_owned());
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
    let body = script
        .strip_prefix("#!/bin/sh\n")
        .expect("process Agent fixture must have a shell header");
    let session_id = if script.contains("process-session") {
        "process-session"
    } else {
        "fake-session"
    };
    let script = format!(
        "#!/bin/sh\ncase \"$1\" in\n  exec) printf '%s\\n' '{{\"type\":\"thread.started\",\"thread_id\":\"{session_id}\"}}' ;;\n  --print) printf '%s\\n' '{{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"{session_id}\"}}' ;;\nesac\n{body}"
    );
    fs::write(&path, script).expect("process Agent must be written");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
        .expect("process Agent must be executable");
    world.process_agent = Some(path);
}

fn prepare_raw_process_agent(world: &mut LifecycleWorld, script: &str) {
    use std::os::unix::fs::PermissionsExt;

    let root = world.root.as_ref().expect("scenario must define root");
    let path = root.path().join("fake-agent.sh");
    fs::write(&path, script).expect("process Agent must be written");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
        .expect("process Agent must be executable");
    world.process_agent = Some(path);
}

fn prepare_signal_gate(world: &mut LifecycleWorld) {
    let root = world.root.as_ref().expect("scenario must define root");
    let gate = root.path().join("signal-gate");
    let status = Command::new("mkfifo")
        .arg(&gate)
        .status()
        .expect("mkfifo must run");
    assert!(status.success());
    world.process_gate = Some(gate);
    world.process_ready = Some(root.path().join("signal-ready"));
}

fn run_process_signal_shutdown(world: &mut LifecycleWorld, signal: &str, repeat: bool) {
    let root = world.root.as_ref().expect("scenario must define root");
    let agent = world
        .process_agent
        .as_ref()
        .expect("scenario must define process Agent");
    let gate = world
        .process_gate
        .as_ref()
        .expect("scenario must define signal gate");
    let ready = world
        .process_ready
        .as_ref()
        .expect("scenario must define ready marker");
    let marker = root.path().join("signal-received");
    let child = Command::new(env!("CARGO_BIN_EXE_orchestrator"))
        .args(["start", "delivery"])
        .env("ORC_HOME", root.path())
        .env("ORC_AGENT_COMMAND", agent)
        .env("ORC_TEST_GATE", gate)
        .env("ORC_TEST_READY", ready)
        .env("ORC_TEST_SIGNALLED", &marker)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("orchestrator must start");
    wait_for_path(ready);
    let short_signal = signal.strip_prefix("SIG").unwrap_or(signal);
    let started = Instant::now();
    send_signal(child.id(), short_signal);
    if repeat {
        wait_for_path(&marker);
        send_signal(child.id(), short_signal);
    }
    let output = child.wait_with_output().expect("orchestrator must exit");
    world.shutdown_elapsed = Some(started.elapsed());
    world.sent_signal = Some(short_signal.to_owned());
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

fn send_signal(process_id: u32, signal: &str) {
    let status = Command::new("/bin/kill")
        .args([format!("-{signal}"), process_id.to_string()])
        .status()
        .expect("kill command must run");
    assert!(status.success());
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

fn wait_for_file_content(path: &std::path::Path, expected: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if fs::read_to_string(path).is_ok_and(|content| content.contains(expected)) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timeout waiting for {expected} in {}",
            path.display()
        );
        std::thread::yield_now();
    }
}

fn run_start(world: &mut LifecycleWorld, behaviors: impl IntoIterator<Item = Behavior>) {
    run_start_with_terminal(world, behaviors, TerminalMode::Unavailable);
}

fn resume_with_fake(
    world: &LifecycleWorld,
    behaviors: impl IntoIterator<Item = Behavior>,
) -> (Observed, Vec<Call>) {
    let root = world.root.as_ref().expect("scenario must define root");
    let registry = FakeAgentRegistry::new(root.path().to_owned(), behaviors);
    let run_id = world.run_id.as_ref().expect("scenario must define run ID");
    let command =
        LifecycleCommand::Resume(orchestrator::RunId::parse(run_id).expect("run ID must be valid"));
    let mut reporter = VecReporter::default();
    let result = execute_lifecycle(
        &command,
        &environment(root),
        TerminalMode::Unavailable,
        &LifecycleSignals::default(),
        &registry,
        &mut reporter,
    );
    let calls = registry
        .calls
        .into_inner()
        .expect("call log must be available");
    (observe(result, reporter), calls)
}

fn run_start_with_terminal(
    world: &mut LifecycleWorld,
    behaviors: impl IntoIterator<Item = Behavior>,
    terminal: TerminalMode,
) {
    let root = world.root.as_ref().expect("scenario must define root");
    let registry = FakeAgentRegistry::new(root.path().to_owned(), behaviors);
    let mut reporter = VecReporter::default();
    let started = unix_time_ms();
    let result = execute_lifecycle(
        &LifecycleCommand::start_explicit("delivery").expect("workflow ID must be valid"),
        &environment(root),
        terminal,
        &LifecycleSignals::default(),
        &registry,
        &mut reporter,
    );
    let finished = unix_time_ms();
    world.start_window_ms = Some((started, finished));
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

fn unix_time_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time must be after Unix epoch")
        .as_millis()
}

fn observe_process_output(output: std::process::Output) -> Observed {
    Observed {
        exit_code: u8::try_from(output.status.code().expect("process must exit normally"))
            .expect("fixture exit code must fit u8"),
        lines: String::from_utf8(output.stdout)
            .expect("stdout must be UTF-8")
            .lines()
            .map(str::to_owned)
            .collect(),
        error: Some(String::from_utf8_lossy(&output.stderr).into_owned()),
    }
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
        current_dir: None,
        path: None,
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

fn attempt_record_names(world: &LifecycleWorld) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(run_directory(world))
        .expect("run directory must be readable")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".attempt.yaml"))
        .collect();
    names.sort();
    names
}

fn durable_snapshot(world: &LifecycleWorld) -> Vec<(String, Vec<u8>)> {
    let mut files: Vec<(String, Vec<u8>)> = fs::read_dir(run_directory(world))
        .expect("run directory must be readable")
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_file())
        .map(|entry| {
            (
                entry.file_name().to_string_lossy().into_owned(),
                fs::read(entry.path()).expect("durable file must be readable"),
            )
        })
        .collect();
    files.sort_by(|left, right| left.0.cmp(&right.0));
    files
}

fn input_versions(call: &Call) -> Vec<String> {
    call.inputs
        .iter()
        .map(|input| {
            format!(
                "{}:{}:{}",
                input.step_id,
                input.input_id,
                input
                    .path
                    .file_name()
                    .expect("artifact path must have a file name")
                    .to_string_lossy()
            )
        })
        .collect()
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
    LifecycleWorld::run("features/artifact_completion.feature").await;
    LifecycleWorld::run("features/run_lock.feature").await;
    LifecycleWorld::run("features/control_endpoint.feature").await;
    LifecycleWorld::run("features/graph_execution.feature").await;
    LifecycleWorld::run("features/recovery.feature").await;
    LifecycleWorld::run("features/human_execution.feature").await;
    LifecycleWorld::run("features/signal_shutdown.feature").await;
    LifecycleWorld::run("features/agent_types.feature").await;
}
