//! Cucumber-проверка read-only run inspection API и его CLI-представления.

use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::Command;

use cucumber::{World, given, then, when};
use fs2::FileExt;
use orchestrator::{ProcessEnvironment, RunId, execute_run_list, execute_run_show};
use tempfile::TempDir;

#[derive(Debug, Default, World)]
struct InspectionWorld {
    root: Option<TempDir>,
    observed: Option<Observed>,
    durable_snapshot: Vec<(PathBuf, Vec<u8>)>,
    lock: Option<File>,
}

#[derive(Debug)]
struct Observed {
    exit_code: u8,
    stdout: Vec<u8>,
    stderr: String,
}

#[given("подготовлен пустой inspection root")]
fn empty_inspection_root(world: &mut InspectionWorld) {
    world.root = Some(TempDir::new().expect("inspection root must be created"));
}

#[given("подготовлены active, blocked и completed durable runs")]
fn three_run_states(world: &mut InspectionWorld) {
    empty_inspection_root(world);
    write_active_run(world.root(), 10);
    write_blocked_run(world.root(), 20);
    write_completed_run(world.root(), 30);
}

#[given("подготовлены completed и противоречивый durable runs")]
fn valid_and_invalid_runs(world: &mut InspectionWorld) {
    empty_inspection_root(world);
    write_completed_run(world.root(), 10);
    let directory = run_directory(world.root(), 20);
    fs::create_dir_all(&directory).expect("invalid run directory must be created");
    fs::write(directory.join("active.lock"), []).expect("lock file must be written");
    fs::write(directory.join("spec.yaml"), b"steps: [").expect("invalid spec must be written");
}

#[given("подготовлен active durable run с session и ready Step")]
fn active_run_with_ready_step(world: &mut InspectionWorld) {
    empty_inspection_root(world);
    write_ready_run(world.root(), 20);
}

#[given("подготовлен active durable run с удерживаемым lock")]
fn active_locked_run(world: &mut InspectionWorld) {
    active_run_with_ready_step(world);
    let path = run_directory(world.root(), 20).join("active.lock");
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .expect("lock file must open");
    lock.try_lock_exclusive()
        .expect("test lock must be acquired");
    world.lock = Some(lock);
}

#[given("подготовлен completed run с двумя версиями бинарного artifact")]
fn versioned_binary_artifacts(world: &mut InspectionWorld) {
    empty_inspection_root(world);
    let directory = run_directory(world.root(), 30);
    write_run_files(
        &directory,
        &single_step_spec("binary", "[result]"),
        &[
            (
                "0.first.attempt.yaml",
                "input: []\nevents:\n- type: completed\n",
            ),
            ("1.first.attempt.yaml", "input: []\nevents: []\n"),
            (
                "2.first.attempt.yaml",
                "input: []\nevents:\n- type: completed\n",
            ),
        ],
    );
    fs::write(directory.join("0.first.result.artifact"), [0, 0xff, b'\n'])
        .expect("first artifact must be written");
    fs::write(directory.join("2.first.result.artifact"), b"second")
        .expect("second artifact must be written");
}

#[when("выполняется run list через публичный API")]
fn list_through_api(world: &mut InspectionWorld) {
    world.capture_snapshot();
    world.observed = Some(observe_api(execute_run_list(&environment(world.root()))));
}

#[when(expr = "выполняется run show {word} через публичный API")]
#[allow(clippy::needless_pass_by_value)]
fn show_through_api(world: &mut InspectionWorld, run_id: String) {
    world.capture_snapshot();
    let run_id = RunId::parse(&run_id).expect("fixture RunId must be valid");
    world.observed = Some(observe_api(execute_run_show(
        run_id,
        &environment(world.root()),
    )));
}

#[when("запускается orchestrator run list")]
fn list_through_cli(world: &mut InspectionWorld) {
    run_cli(world, ["run", "list"]);
}

#[when(expr = "запускается orchestrator run show {word}")]
#[allow(clippy::needless_pass_by_value)]
fn show_through_cli(world: &mut InspectionWorld, run_id: String) {
    run_cli(world, ["run", "show", &run_id]);
}

#[when(expr = "запускается orchestrator run artifact {word} {word} {word}")]
#[allow(clippy::needless_pass_by_value)]
fn artifact_through_cli(
    world: &mut InspectionWorld,
    run_id: String,
    attempt: String,
    input_id: String,
) {
    run_cli(world, ["run", "artifact", &run_id, &attempt, &input_id]);
}

#[then(expr = "inspection завершается с кодом {int}")]
fn inspection_exit_code(world: &mut InspectionWorld, code: u8) {
    assert_eq!(
        world.observed().exit_code,
        code,
        "{}",
        world.observed().stderr
    );
}

#[then("inspection output пуст")]
fn inspection_output_is_empty(world: &mut InspectionWorld) {
    assert!(world.observed().stdout.is_empty());
}

#[then("список runs отсортирован и содержит три вычисленных состояния")]
fn list_is_sorted_with_states(world: &mut InspectionWorld) {
    assert_eq!(
        world.stdout_text(),
        "run 10: workflow=active state=active\nrun 20: workflow=blocked state=blocked\nrun 30: workflow=completed state=completed\n"
    );
}

#[then("run show содержит workflow Steps attempt session и ready frontier")]
fn show_contains_read_model(world: &mut InspectionWorld) {
    assert_eq!(
        world.stdout_text(),
        "run 20: workflow=delivery state=active\nstep first: attempts=0\nstep target: attempts=-\nattempt 0: step=first state=completed session=native-session input=-\nfrontier: ready=target missing=-"
    );
}

#[then("run show содержит active run 20")]
fn show_contains_active_run(world: &mut InspectionWorld) {
    assert!(
        world
            .stdout_text()
            .starts_with("run 20: workflow=delivery state=active\n")
    );
}

#[then("stdout побайтово равен первой версии artifact")]
fn stdout_is_first_artifact(world: &mut InspectionWorld) {
    assert_eq!(world.observed().stdout, [0, 0xff, b'\n']);
}

#[then("inspection не изменил durable state")]
fn durable_state_is_unchanged(world: &mut InspectionWorld) {
    assert_eq!(world.durable_snapshot, snapshot(world.root()));
}

impl InspectionWorld {
    fn root(&self) -> &Path {
        self.root
            .as_ref()
            .expect("scenario must define root")
            .path()
    }

    fn observed(&self) -> &Observed {
        self.observed.as_ref().expect("scenario must run command")
    }

    fn stdout_text(&self) -> &str {
        std::str::from_utf8(&self.observed().stdout).expect("stdout must be UTF-8")
    }

    fn capture_snapshot(&mut self) {
        self.durable_snapshot = snapshot(self.root());
    }
}

fn run_cli<const N: usize>(world: &mut InspectionWorld, arguments: [&str; N]) {
    world.capture_snapshot();
    let output = Command::new(env!("CARGO_BIN_EXE_orchestrator"))
        .args(arguments)
        .env("ORC_HOME", world.root())
        .output()
        .expect("orchestrator must run");
    world.observed = Some(Observed {
        exit_code: u8::try_from(output.status.code().expect("process must exit normally"))
            .expect("fixture exit code must fit u8"),
        stdout: output.stdout,
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    });
}

fn observe_api(result: Result<String, orchestrator::CommandError>) -> Observed {
    match result {
        Ok(output) => Observed {
            exit_code: 0,
            stdout: output.into_bytes(),
            stderr: String::new(),
        },
        Err(error) => Observed {
            exit_code: error.exit_code(),
            stdout: Vec::new(),
            stderr: error.to_string(),
        },
    }
}

fn environment(root: &Path) -> ProcessEnvironment {
    ProcessEnvironment {
        home: None,
        orc_home: Some(root.as_os_str().to_owned()),
    }
}

fn write_active_run(root: &Path, run_id: u64) {
    let directory = run_directory(root, run_id);
    write_run_files(
        &directory,
        &single_step_spec("active", "[]"),
        &[("0.first.attempt.yaml", "input: []\nevents: []\n")],
    );
}

fn write_completed_run(root: &Path, run_id: u64) {
    let directory = run_directory(root, run_id);
    write_run_files(
        &directory,
        &single_step_spec("completed", "[]"),
        &[(
            "0.first.attempt.yaml",
            "input: []\nevents:\n- type: completed\n",
        )],
    );
}

fn write_ready_run(root: &Path, run_id: u64) {
    let directory = run_directory(root, run_id);
    let spec = "workflow-id: delivery\nmax-parallel-agents: 5\nsteps:\n- id: first\n  agent:\n    type: codex\n    model: model\n    reasoning: high\n  prompt: null\n  human: false\n  depends-on: []\n  outputs: []\n- id: target\n  agent:\n    type: codex\n    model: model\n    reasoning: high\n  prompt: null\n  human: false\n  depends-on: [first]\n  outputs: []\n";
    write_run_files(
        &directory,
        spec,
        &[(
            "0.first.attempt.yaml",
            "input: []\nevents:\n- type: session-activated\n  session-id: native-session\n- type: completed\n",
        )],
    );
}

fn write_blocked_run(root: &Path, run_id: u64) {
    let directory = run_directory(root, run_id);
    let spec = "workflow-id: blocked\nmax-parallel-agents: 5\nsteps:\n- id: a\n  agent:\n    type: codex\n    model: model\n    reasoning: high\n  prompt: null\n  human: false\n  depends-on: [b, c]\n  outputs: []\n- id: b\n  agent:\n    type: codex\n    model: model\n    reasoning: high\n  prompt: null\n  human: false\n  depends-on: [a, c]\n  outputs: []\n- id: c\n  agent:\n    type: codex\n    model: model\n    reasoning: high\n  prompt: null\n  human: false\n  depends-on: [a, b]\n  outputs: []\n";
    write_run_files(
        &directory,
        spec,
        &[(
            "0.a.attempt.yaml",
            "input: []\nevents:\n- type: completed\n",
        )],
    );
}

fn single_step_spec(workflow_id: &str, outputs: &str) -> String {
    format!(
        "workflow-id: {workflow_id}\nmax-parallel-agents: 5\nsteps:\n- id: first\n  agent:\n    type: codex\n    model: model\n    reasoning: high\n  prompt: null\n  human: false\n  depends-on: []\n  outputs: {outputs}\n"
    )
}

fn write_run_files(directory: &Path, spec: &str, attempts: &[(&str, &str)]) {
    fs::create_dir_all(directory).expect("run directory must be created");
    fs::write(directory.join("active.lock"), []).expect("lock file must be written");
    fs::write(directory.join("spec.yaml"), spec).expect("spec must be written");
    for (name, content) in attempts {
        fs::write(directory.join(name), content).expect("attempt must be written");
    }
}

fn run_directory(root: &Path, run_id: u64) -> PathBuf {
    root.join("run").join(run_id.to_string())
}

fn snapshot(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let run_root = root.join("run");
    if !run_root.exists() {
        return Vec::new();
    }
    let mut files = Vec::new();
    let mut directories = vec![run_root];
    while let Some(directory) = directories.pop() {
        for entry in fs::read_dir(directory).expect("snapshot directory must be readable") {
            let path = entry.expect("snapshot entry must be readable").path();
            if path.is_dir() {
                directories.push(path);
            } else {
                let relative = path
                    .strip_prefix(root)
                    .expect("snapshot path must be inside root")
                    .to_owned();
                files.push((
                    relative,
                    fs::read(path).expect("snapshot file must be readable"),
                ));
            }
        }
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));
    files
}

#[tokio::main]
async fn main() {
    InspectionWorld::run("features/run_inspection.feature").await;
}
