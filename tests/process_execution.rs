//! Cucumber-проверка Process executor, run parameters и process recovery.

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use cucumber::{World, given, then, when};
use orchestrator::{
    ProcessEnvironment, SourceWorkflow, ValidateCommand, WorkflowPlan, build_workflow_plan,
    execute_validate, show_workflow,
};
use tempfile::TempDir;

#[derive(Debug, Default, World)]
struct ProcessWorld {
    root: Option<TempDir>,
    outcome: Option<Observed>,
    run_id: Option<String>,
    source: Option<SourceWorkflow>,
    plan: Option<WorkflowPlan>,
}

#[derive(Debug)]
struct Observed {
    code: i32,
    stdout: String,
    stderr: String,
}

#[given("подготовлен workflow с Process producer и Process consumer")]
fn agent_and_process_workflow(world: &mut ProcessWorld) {
    let root = prepare_root(world);
    let producer = executable(
        root,
        "producer.sh",
        "#!/bin/sh\nset -eu\nprintf 'source bytes' > \"$2\"\n",
    );
    let consumer = executable(
        root,
        "consumer.sh",
        "#!/bin/sh\nset -eu\nprintf '%s\\n' \"$@\" > \"$ORC_HOME/process.args\"\nprintf '%s\\n%s\\n%s\\n' \"$ORC_STEP_ID\" \"$ORC_RUN_ID\" \"$ORC_ATTEMPT\" > \"$ORC_HOME/process.env\"\nprintf '%s' \"$ORC_INPUT\" > \"$ORC_HOME/process.input\"\nprintf '%s' \"$ORC_OUTPUT\" > \"$ORC_HOME/process.output\"\nif [ \"${ORC_CONTROL_ENDPOINT+x}\" = x ]; then exit 8; fi\nwhile [ \"$#\" -gt 0 ]; do\n  case \"$1\" in\n    --mode) mode=$2; shift 2 ;;\n    --input) input=$2; shift 2 ;;\n    --output) output=$2; shift 2 ;;\n  esac\ndone\nprintf '%s:' \"$mode\" > \"$output\"\n/bin/cat \"$input\" >> \"$output\"\n",
    );
    fs::write(
        root.join("workflow/delivery.yaml"),
        format!(
            "parameters:\n  mode: string\nsteps:\n  - id: produce\n    process:\n      executable: {}\n      args: [\"--output\", \"{{{{output:source}}}}\"]\n    human: false\n    depends-on: []\n    outputs: [source]\n  - id: convert\n    process:\n      executable: {}\n      args: [\"--mode\", \"{{{{param:mode}}}}\", \"--input\", \"{{{{path:produce:source}}}}\", \"--output\", \"{{{{output:result}}}}\"]\n    human: false\n    depends-on: [produce]\n    outputs: [result]\n",
            producer.display(),
            consumer.display()
        ),
    )
    .expect("workflow must be written");
}

#[when("запускается workflow с parameter mode содержащим пробел")]
fn start_with_spaced_parameter(world: &mut ProcessWorld) {
    run_cli(world, &["start", "delivery", "--param", "mode=fast mode"]);
}

#[then(
    "Process получает ровно шесть argv: --mode, fast mode, --input, absolute input path, --output, absolute output path"
)]
fn exact_argv(world: &mut ProcessWorld) {
    assert_eq!(world.observed().code, 0, "{}", world.observed().stderr);
    let root = world.root().to_owned();
    let args = fs::read_to_string(root.join("process.args")).expect("args must be readable");
    let values = args.lines().collect::<Vec<_>>();
    assert_eq!(values.len(), 6);
    assert_eq!(values[0..2], ["--mode", "fast mode"]);
    assert_eq!(values[2], "--input");
    assert!(Path::new(values[3]).is_absolute());
    assert_eq!(values[4], "--output");
    assert!(Path::new(values[5]).is_absolute());
}

#[then(
    "Process получает StepId convert, RunId, attempt 1, YAML input и output mappings без control endpoint"
)]
fn exact_process_environment(world: &mut ProcessWorld) {
    let root = world.root().to_owned();
    let environment =
        fs::read_to_string(root.join("process.env")).expect("Process environment must be readable");
    let values: Vec<&str> = environment.lines().collect();
    assert_eq!(values[0], "convert");
    assert_eq!(
        values[1],
        world.run_id.as_deref().expect("run ID must exist")
    );
    assert_eq!(values[2], "1");
    let input = fs::read_to_string(root.join("process.input"))
        .expect("Process input mapping must be readable");
    assert!(input.contains("step-id: produce"));
    assert!(input.contains("input-id: source"));
    assert!(input.contains("path:"));
    let output = fs::read_to_string(root.join("process.output"))
        .expect("Process output mapping must be readable");
    let outputs: std::collections::BTreeMap<String, PathBuf> =
        serde_yaml::from_str(&output).expect("Process output mapping must be YAML");
    let result = outputs.get("result").expect("result output must exist");
    assert!(result.is_absolute());
    assert_eq!(
        result.file_name().and_then(|name| name.to_str()),
        Some("result")
    );
}

#[then("Process output опубликован как artifact")]
fn process_output_is_artifact(world: &mut ProcessWorld) {
    let artifact = run_directory(world).join("1.convert.result.artifact");
    assert_eq!(
        fs::read_to_string(artifact).expect("artifact must be readable"),
        "fast mode:source bytes"
    );
}

#[given("подготовлен Process Step проверяющий non-interactive streams")]
fn process_checking_streams(world: &mut ProcessWorld) {
    let root = prepare_root(world);
    let executable = executable(
        root,
        "streams.sh",
        "#!/bin/sh\nset -eu\nif IFS= read -r unexpected; then exit 8; fi\nprintf 'process stdout\\n'\nprintf 'process stderr\\n' >&2\n",
    );
    write_process_workflow(root, &executable, "[]", "[]");
}

#[when("workflow с Process Step запускается с данными в stdin supervisor")]
fn start_process_with_supervisor_stdin(world: &mut ProcessWorld) {
    let root = world.root().to_owned();
    let mut child = Command::new(env!("CARGO_BIN_EXE_orchestrator"))
        .args(["start", "delivery"])
        .env("ORC_HOME", &root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("orchestrator must start");
    child
        .stdin
        .take()
        .expect("orchestrator stdin must be piped")
        .write_all(b"must not reach Process\n")
        .expect("supervisor stdin must accept test bytes");
    let output = child
        .wait_with_output()
        .expect("orchestrator must finish normally");
    capture_output(world, output);
}

#[then("Process завершился без stdin и его stdout и stderr наблюдаемы")]
fn process_inherits_output_streams(world: &mut ProcessWorld) {
    assert_eq!(world.observed().code, 0, "{}", world.observed().stderr);
    assert!(world.observed().stdout.contains("process stdout\n"));
    assert!(world.observed().stderr.contains("process stderr\n"));
}

#[then("materialized workflow содержит точное значение parameter")]
fn parameter_is_materialized(world: &mut ProcessWorld) {
    let spec =
        fs::read_to_string(run_directory(world).join("spec.yaml")).expect("spec must be readable");
    assert!(spec.contains("mode: fast mode"));
}

#[given("подготовлен workflow с обязательным parameter mode")]
fn required_parameter_workflow(world: &mut ProcessWorld) {
    let root = prepare_root(world);
    fs::write(
        root.join("workflow/delivery.yaml"),
        "parameters:\n  mode: string\nsteps:\n  - id: execute\n    process:\n      executable: /usr/bin/true\n      args: [\"{{param:mode}}\"]\n    human: false\n    depends-on: []\n    outputs: []\n",
    )
    .expect("workflow must be written");
}

#[when(expr = "start получает {word} parameter")]
#[allow(clippy::needless_pass_by_value)]
fn invalid_parameter(world: &mut ProcessWorld, kind: String) {
    let args = match kind.as_str() {
        "отсутствующий" => vec!["start", "delivery"],
        "неизвестный" => vec!["start", "delivery", "--param", "other=value"],
        "повторяющийся" => vec![
            "start", "delivery", "--param", "mode=one", "--param", "mode=two",
        ],
        other => panic!("unknown parameter case: {other}"),
    };
    run_cli(world, &args);
}

#[when("start получает parameter без разделителя equals")]
fn parameter_without_equals(world: &mut ProcessWorld) {
    run_cli(world, &["start", "delivery", "--param", "mode"]);
}

#[then(expr = "команда завершается с кодом {int}")]
fn exit_code(world: &mut ProcessWorld, code: i32) {
    assert_eq!(world.observed().code, code, "{}", world.observed().stderr);
}

#[then("run не создан")]
fn run_not_created(world: &mut ProcessWorld) {
    let run_root = world.root().join("run");
    assert!(
        !run_root.exists()
            || fs::read_dir(run_root)
                .expect("run root must be readable")
                .next()
                .is_none()
    );
}

#[given(regex = r"^подготовлен Process Step с ошибкой (.+)$")]
#[allow(clippy::needless_pass_by_value)]
fn invalid_process(world: &mut ProcessWorld, error: String) {
    let root = prepare_root(world);
    let executable = executable(root, "process.sh", "#!/bin/sh\nexit 0\n");
    let (parameters, agent, human, args, dependency, outputs, source) = match error.as_str() {
        "одновременно указан Agent" => (
            "",
            "    agent: main\n",
            false,
            "[]",
            "[]",
            "[]",
            String::new(),
        ),
        "установлен human" => ("", "", true, "[]", "[]", "[]", String::new()),
        "content placeholder {{content:source:data}}" => (
            "",
            "",
            false,
            "[\"{{content:source:data}}\"]",
            "[]",
            "[]",
            String::new(),
        ),
        "placeholder prefix-{{param:mode}} является частью argv" => (
            "parameters:\n  mode: string\n",
            "",
            false,
            "[\"prefix-{{param:mode}}\"]",
            "[]",
            "[]",
            String::new(),
        ),
        "parameter placeholder {{param:missing}} неизвестен" => (
            "",
            "",
            false,
            "[\"{{param:missing}}\"]",
            "[]",
            "[]",
            String::new(),
        ),
        "path placeholder {{path:missing:data}} без dependency" => (
            "",
            "",
            false,
            "[\"{{path:missing:data}}\"]",
            "[missing]",
            "[]",
            String::new(),
        ),
        "path placeholder {{path:source:missing}} без output" => (
            "",
            "",
            false,
            "[\"{{path:source:missing}}\"]",
            "[source]",
            "[]",
            format!(
                "  - id: source\n    process:\n      executable: {}\n      args: []\n    human: false\n    depends-on: []\n    outputs: [data]\n",
                executable.display()
            ),
        ),
        "output placeholder {{output:missing}} неизвестен" => (
            "",
            "",
            false,
            "[\"{{output:missing}}\"]",
            "[]",
            "[]",
            String::new(),
        ),
        other => panic!("unknown process error: {other}"),
    };
    fs::write(
        root.join("config.yaml"),
        "agents:\n  main:\n    type: codex\n    model: model\n    reasoning: high\n",
    )
    .expect("config must be written");
    fs::write(
        root.join("workflow/delivery.yaml"),
        format!(
            "{parameters}steps:\n{source}  - id: execute\n{agent}    process:\n      executable: {}\n      args: {args}\n    human: {human}\n    depends-on: {dependency}\n    outputs: {outputs}\n",
            executable.display()
        ),
    )
    .expect("workflow must be written");
}

#[when("workflow проверяется через публичный API")]
fn validate_api(world: &mut ProcessWorld) {
    let result = execute_validate(
        &ValidateCommand::explicit("delivery").expect("workflow ID must be valid"),
        &environment(world.root()),
    );
    world.outcome = Some(match result {
        Ok(stdout) => Observed {
            code: 0,
            stdout,
            stderr: String::new(),
        },
        Err(error) => Observed {
            code: i32::from(error.exit_code()),
            stdout: String::new(),
            stderr: error.to_string(),
        },
    });
}

#[then("validation отклоняет Process Step")]
fn validation_rejects(world: &mut ProcessWorld) {
    assert_eq!(world.observed().code, 3);
    assert!(world.observed().stdout.is_empty());
}

#[given("подготовлен source Process workflow с parameter mode")]
fn source_process_workflow(world: &mut ProcessWorld) {
    let root = prepare_root(world);
    fs::write(
        root.join("workflow/delivery.yaml"),
        "parameters:\n  mode: string\nsteps:\n  - id: execute\n    process:\n      executable: true\n      args: [\"{{param:mode}}\"]\n    human: false\n    depends-on: []\n    outputs: []\n",
    )
    .expect("workflow must be written");
}

#[when("Process workflow читается и планируется через публичный API")]
fn inspect_process_workflow(world: &mut ProcessWorld) {
    world.source =
        Some(show_workflow("delivery", &environment(world.root())).expect("source must load"));
    world.plan =
        Some(build_workflow_plan("delivery", &environment(world.root())).expect("plan must build"));
}

#[then("source Process сохраняет executable и argv")]
fn source_process_is_preserved(world: &mut ProcessWorld) {
    let source = world.source.as_ref().expect("source must exist");
    assert_eq!(source.parameters(), ["mode"]);
    let process = source.steps()[0].process().expect("Process must exist");
    assert_eq!(process.executable(), "true");
    assert_eq!(process.args(), ["{{param:mode}}"]);
}

#[then("plan содержит absolute executable cwd и parameter declaration")]
fn plan_has_materialized_process(world: &mut ProcessWorld) {
    let plan = world.plan.as_ref().expect("plan must exist");
    assert_eq!(plan.parameters(), ["mode"]);
    assert!(plan.steps()[0].agent().is_none());
    let process = plan.steps()[0].process().expect("Process must exist");
    assert!(Path::new(process.executable()).is_absolute());
    assert!(Path::new(process.cwd()).is_absolute());
    assert_eq!(process.args(), ["{{param:mode}}"]);
}

#[given("подготовлен Process Step завершающийся успешно со второго запуска")]
fn succeeds_on_second_run(world: &mut ProcessWorld) {
    let root = prepare_root(world);
    let executable = executable(
        root,
        "retry.sh",
        "#!/bin/sh\nset -eu\ncount=0\nif [ -f \"$ORC_HOME/count\" ]; then count=$(/bin/cat \"$ORC_HOME/count\"); fi\ncount=$((count + 1))\nprintf '%s' \"$count\" > \"$ORC_HOME/count\"\nprintf '%s' \"$0\" > \"$ORC_HOME/executable.$count\"\nprintf '%s' \"$PWD\" > \"$ORC_HOME/cwd.$count\"\nprintf '%s' \"$2\" > \"$ORC_HOME/parameter.$count\"\nprintf '%s' \"$ORC_INPUT\" > \"$ORC_HOME/input.$count\"\nprintf 'attempt %s' \"$count\" > \"$1\"\nif [ \"$count\" -eq 1 ]; then exit 9; fi\n",
    );
    fs::write(
        root.join("workflow/delivery.yaml"),
        format!(
            "parameters:\n  mode: string\nsteps:\n  - id: execute\n    process:\n      executable: {}\n      args: [\"{{{{output:result}}}}\", \"{{{{param:mode}}}}\"]\n    human: false\n    depends-on: []\n    outputs: [result]\n",
            executable.display()
        ),
    )
    .expect("workflow must be written");
}

#[when("start запускает Process первый раз")]
fn first_process_run(world: &mut ProcessWorld) {
    run_cli(world, &["start", "delivery", "--param", "mode=stable"]);
}

#[then("команда завершается runtime error и attempt не завершён")]
fn runtime_incomplete(world: &mut ProcessWorld) {
    assert_eq!(world.observed().code, 1);
    let record = fs::read_to_string(run_directory(world).join("0.execute.attempt.yaml"))
        .expect("attempt must be readable");
    assert!(!record.contains("completed"));
    assert!(
        !run_directory(world)
            .join("0.execute.result.artifact")
            .exists()
    );
}

#[then("exit code Process не записан и автоматический restart не выполнен")]
fn process_exit_is_not_durable_or_retried(world: &mut ProcessWorld) {
    let record = fs::read_to_string(run_directory(world).join("0.execute.attempt.yaml"))
        .expect("attempt must be readable");
    assert!(!record.contains("exit"));
    assert_eq!(
        fs::read_to_string(world.root().join("count")).expect("count must exist"),
        "1"
    );
}

#[when("run явно продолжается")]
fn resume_run(world: &mut ProcessWorld) {
    let run_id = world.run_id.clone().expect("run ID must exist");
    run_cli(world, &["resume", &run_id]);
}

#[then("тот же Process attempt запущен второй раз")]
fn process_ran_twice(world: &mut ProcessWorld) {
    assert_eq!(
        fs::read_to_string(world.root().join("count")).expect("count must exist"),
        "2"
    );
    assert!(run_directory(world).join("0.execute.attempt.yaml").exists());
    assert!(!run_directory(world).join("1.execute.attempt.yaml").exists());
}

#[then("оба запуска получили одинаковые executable, cwd, parameter и durable input mapping")]
fn retried_process_uses_same_materialized_input(world: &mut ProcessWorld) {
    for field in ["executable", "cwd", "parameter", "input"] {
        let first = fs::read(world.root().join(format!("{field}.1")))
            .expect("first invocation field must be readable");
        let second = fs::read(world.root().join(format!("{field}.2")))
            .expect("second invocation field must be readable");
        assert_eq!(first, second, "{field} changed between Process invocations");
    }
    assert_eq!(
        fs::read_to_string(world.root().join("parameter.2")).expect("parameter must be readable"),
        "stable"
    );
}

#[then("Process attempt не содержит session activation")]
fn process_attempt_has_no_session_activation(world: &mut ProcessWorld) {
    let record = fs::read_to_string(run_directory(world).join("0.execute.attempt.yaml"))
        .expect("attempt must be readable");
    assert!(!record.contains("session-activated"));
    assert!(!record.contains("session-id"));
}

#[then("run завершён")]
fn run_completed(world: &mut ProcessWorld) {
    assert_eq!(world.observed().code, 0, "{}", world.observed().stderr);
    assert!(world.observed().stdout.contains(": completed"));
    assert_eq!(
        fs::read_to_string(run_directory(world).join("0.execute.result.artifact"))
            .expect("artifact must be readable"),
        "attempt 2"
    );
}

#[given("подготовлен Process Step не создающий объявленный output")]
fn missing_output_process(world: &mut ProcessWorld) {
    let root = prepare_root(world);
    let executable = executable(root, "no-output.sh", "#!/bin/sh\nexit 0\n");
    write_process_workflow(root, &executable, "[]", "[result]");
}

#[given("подготовлен Process Step создающий directory вместо output")]
fn directory_output_process(world: &mut ProcessWorld) {
    let root = prepare_root(world);
    let executable = executable(root, "directory-output.sh", "#!/bin/sh\nmkdir \"$1\"\n");
    write_process_workflow(root, &executable, "[\"{{output:result}}\"]", "[result]");
}

#[given("подготовлен Process Step направляющий stdout в output")]
fn stdout_process(world: &mut ProcessWorld) {
    let root = prepare_root(world);
    let executable = executable(root, "stdout.sh", "#!/bin/sh\nprintf 'stdout bytes'\n");
    fs::write(
        root.join("workflow/delivery.yaml"),
        format!(
            "steps:\n  - id: execute\n    process:\n      executable: {}\n      args: []\n      stdout: result\n    human: false\n    depends-on: []\n    outputs: [result]\n",
            executable.display()
        ),
    )
    .expect("workflow must be written");
}

#[then("stdout Process опубликован как artifact")]
fn stdout_is_artifact(world: &mut ProcessWorld) {
    assert_eq!(world.observed().code, 0, "{}", world.observed().stderr);
    assert_eq!(
        fs::read_to_string(run_directory(world).join("0.execute.result.artifact"))
            .expect("artifact must be readable"),
        "stdout bytes"
    );
}

#[when("запускается workflow с Process Step")]
fn start_process(world: &mut ProcessWorld) {
    run_cli(world, &["start", "delivery"]);
}

fn prepare_root(world: &mut ProcessWorld) -> &Path {
    let root = world
        .root
        .get_or_insert_with(|| TempDir::new().expect("temp root must exist"));
    fs::create_dir_all(root.path().join("workflow")).expect("workflow directory must exist");
    root.path()
}

fn executable(root: &Path, name: &str, contents: &str) -> PathBuf {
    let path = root.join(name);
    fs::write(&path, contents).expect("executable must be written");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
        .expect("executable permissions must be set");
    path
}

fn write_process_workflow(root: &Path, executable: &Path, args: &str, outputs: &str) {
    fs::write(
        root.join("workflow/delivery.yaml"),
        format!(
            "steps:\n  - id: execute\n    process:\n      executable: {}\n      args: {args}\n    human: false\n    depends-on: []\n    outputs: {outputs}\n",
            executable.display()
        ),
    )
    .expect("workflow must be written");
}

fn run_cli(world: &mut ProcessWorld, args: &[&str]) {
    let root = world.root().to_owned();
    let mut command = Command::new(env!("CARGO_BIN_EXE_orchestrator"));
    command.args(args).env("ORC_HOME", &root);
    let output = command.output().expect("orchestrator must run");
    capture_output(world, output);
}

fn capture_output(world: &mut ProcessWorld, output: std::process::Output) {
    let root = world.root().to_owned();
    world.outcome = Some(Observed {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8(output.stdout).expect("stdout must be UTF-8"),
        stderr: String::from_utf8(output.stderr).expect("stderr must be UTF-8"),
    });
    world.run_id = fs::read_dir(root.join("run"))
        .ok()
        .and_then(|entries| {
            entries
                .filter_map(Result::ok)
                .find(|entry| entry.path().is_dir())
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
        })
        .or_else(|| world.run_id.clone());
}

fn environment(root: &Path) -> ProcessEnvironment {
    ProcessEnvironment {
        home: None,
        orc_home: Some(root.as_os_str().to_owned()),
        current_dir: Some(root.to_owned()),
        path: std::env::var_os("PATH"),
    }
}

fn run_directory(world: &ProcessWorld) -> PathBuf {
    world
        .root()
        .join("run")
        .join(world.run_id.as_ref().expect("run ID must exist"))
}

impl ProcessWorld {
    fn root(&self) -> &Path {
        self.root.as_ref().expect("root must exist").path()
    }

    fn observed(&self) -> &Observed {
        self.outcome.as_ref().expect("command must be observed")
    }
}

#[tokio::main]
async fn main() {
    ProcessWorld::run("features/process_execution.feature").await;
}
