//! Cucumber-проверка read-only run inspection API и его CLI-представления.

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use cucumber::{World, given, then, when};
use fs2::FileExt;
use orchestrator::{
    ProcessEnvironment, RunId, RunInspection, execute_run_list, execute_run_show, inspect_run,
};
use tempfile::TempDir;

#[derive(Debug, Default, World)]
struct InspectionWorld {
    root: Option<TempDir>,
    observed: Option<Observed>,
    durable_snapshot: Vec<(PathBuf, Vec<u8>)>,
    lock: Option<File>,
    typed_snapshot: Option<RunInspection>,
    watch_quiet_before_completion: Option<bool>,
}

#[derive(Debug)]
struct Observed {
    exit_code: u8,
    stdout: Vec<u8>,
    stderr: String,
}

struct ChildGuard(Option<Child>);

impl ChildGuard {
    fn new(child: Child) -> Self {
        Self(Some(child))
    }

    fn child_mut(&mut self) -> &mut Child {
        self.0.as_mut().expect("child must be running")
    }

    fn id(&self) -> u32 {
        self.0.as_ref().expect("child must be running").id()
    }

    fn wait_with_output(&mut self) -> std::process::Output {
        self.0
            .take()
            .expect("child must be running")
            .wait_with_output()
            .expect("orchestrator must exit")
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(child) = &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[given("подготовлен пустой inspection root")]
fn empty_inspection_root(world: &mut InspectionWorld) {
    world.root = Some(TempDir::new().expect("inspection root must be created"));
}

#[given("подготовлены active, закрытый условный и completed durable runs")]
fn three_run_states(world: &mut InspectionWorld) {
    empty_inspection_root(world);
    write_active_run(world.root(), 10);
    write_blocked_run(world.root(), 20);
    write_completed_run(world.root(), 30);
}

#[given("подготовлены active, закрытый условный и completed durable runs и нечисловые entries")]
fn three_run_states_and_non_numeric_entries(world: &mut InspectionWorld) {
    three_run_states(world);
    fs::create_dir_all(world.root().join("run/draft"))
        .expect("non-numeric directory must be created");
    fs::write(world.root().join("run/README"), "not a run")
        .expect("non-numeric file must be written");
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

#[given(expr = "подготовлен active durable run {int} без session")]
fn active_run_without_session(world: &mut InspectionWorld, run_id: u64) {
    empty_inspection_root(world);
    write_active_run(world.root(), run_id);
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

#[given("подготовлен run с двумя завершёнными версиями бинарного artifact")]
fn versioned_binary_artifacts(world: &mut InspectionWorld) {
    empty_inspection_root(world);
    let directory = run_directory(world.root(), 30);
    write_run_files(
        &directory,
        &(single_step_spec("binary", "[result]").replace("depends-on: []", "depends-on: [bridge]")
            + "- id: bridge\n  agent:\n    type: codex\n    model: model\n    reasoning: high\n  prompt: null\n  human: false\n  depends-on: [first]\n  outputs: []\n"),
        &[
            (
                "0.first.attempt.yaml",
                "input: []\nevents:\n- type: completed\n",
            ),
            (
                "1.bridge.attempt.yaml",
                "input: [0]\nevents: [{type: completed}]\n",
            ),
            ("3.bridge.attempt.yaml", "input: [2]\nevents: []\n"),
            (
                "2.first.attempt.yaml",
                "input: [1]\nevents:\n- type: completed\n",
            ),
        ],
    );
    fs::write(directory.join("0.first.result.artifact"), [0, 0xff, b'\n'])
        .expect("first artifact must be written");
    fs::write(directory.join("2.first.result.artifact"), b"second")
        .expect("second artifact must be written");
    fs::write(directory.join("3.bridge.result.artifact"), b"unfinished")
        .expect("unfinished artifact must be written");
    fs::write(directory.join(".artifact.tmp"), b"temporary")
        .expect("temporary artifact must be written");
}

#[given(expr = "подготовлен completed durable run {int}")]
fn completed_durable_run(world: &mut InspectionWorld, run_id: u64) {
    empty_inspection_root(world);
    write_completed_run(world.root(), run_id);
}

#[given("подготовлены valid и два противоречивых durable runs")]
fn valid_and_two_invalid_runs(world: &mut InspectionWorld) {
    empty_inspection_root(world);
    write_completed_run(world.root(), 10);
    for run_id in [20_u64, 30] {
        let directory = run_directory(world.root(), run_id);
        fs::create_dir_all(&directory).expect("invalid run directory must be created");
        fs::write(directory.join("active.lock"), []).expect("lock file must be written");
        fs::write(directory.join("spec.yaml"), b"steps: [").expect("invalid spec must be written");
    }
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

#[when("запускается orchestrator run list только для completed")]
fn completed_list_through_cli(world: &mut InspectionWorld) {
    run_cli(world, ["run", "list", "--state", "completed"]);
}

#[when("запускается orchestrator run list с active, completed, completed и workflow completed")]
fn composed_list_filters_through_cli(world: &mut InspectionWorld) {
    run_cli(
        world,
        [
            "run",
            "list",
            "--state",
            "active",
            "--state",
            "completed",
            "--state",
            "completed",
            "--workflow",
            "completed",
        ],
    );
}

#[when(expr = "запускается orchestrator run list с невалидным filter {string}")]
#[allow(clippy::needless_pass_by_value)]
fn invalid_list_filter_through_cli(world: &mut InspectionWorld, filter: String) {
    match filter.as_str() {
        "state" => run_cli(world, ["run", "list", "--state", "unknown"]),
        "format" => run_cli(world, ["run", "list", "--format", "yaml"]),
        "workflow-id" => run_cli(world, ["run", "list", "--workflow", "Bad"]),
        other => panic!("unknown invalid filter: {other}"),
    }
}

#[when("запускается orchestrator run list для completed workflow")]
fn filtered_list_through_cli(world: &mut InspectionWorld) {
    run_cli(
        world,
        [
            "run",
            "list",
            "--state",
            "completed",
            "--workflow",
            "completed",
        ],
    );
}

#[when(expr = "запускается orchestrator run show {word}")]
#[allow(clippy::needless_pass_by_value)]
fn show_through_cli(world: &mut InspectionWorld, run_id: String) {
    run_cli(world, ["run", "show", &run_id]);
}

#[when(expr = "запускается orchestrator run show {word} в JSON")]
#[allow(clippy::needless_pass_by_value)]
fn show_json_through_cli(world: &mut InspectionWorld, run_id: String) {
    run_cli(world, ["run", "show", &run_id, "--format", "json"]);
}

#[when(expr = "запускается orchestrator run artifacts {word}")]
#[allow(clippy::needless_pass_by_value)]
fn artifacts_through_cli(world: &mut InspectionWorld, run_id: String) {
    run_cli(world, ["run", "artifacts", &run_id]);
}

#[when(expr = "строится typed inspection snapshot run {int} через публичный API")]
fn typed_snapshot_through_api(world: &mut InspectionWorld, run_id: u64) {
    world.capture_snapshot();
    match inspect_run(
        RunId::parse(&run_id.to_string()).expect("fixture RunId must be valid"),
        &environment(world.root()),
    ) {
        Ok(snapshot) => {
            world.typed_snapshot = Some(snapshot);
            world.observed = Some(Observed {
                exit_code: 0,
                stdout: Vec::new(),
                stderr: String::new(),
            });
        }
        Err(error) => {
            world.observed = Some(Observed {
                exit_code: error.exit_code(),
                stdout: Vec::new(),
                stderr: error.to_string(),
            });
        }
    }
}

#[when(expr = "запускается orchestrator run watch {word} в JSON")]
#[allow(clippy::needless_pass_by_value)]
fn watch_json_through_cli(world: &mut InspectionWorld, run_id: String) {
    run_cli(world, ["run", "watch", &run_id, "--format", "json"]);
}

#[when("watch наблюдает изменения lock и временного entry до durable completion")]
fn watch_volatile_entries_then_completion(world: &mut InspectionWorld) {
    world.capture_snapshot();
    let directory = run_directory(world.root(), 10);
    let mut process = spawn_json_watch(world.root(), 10);
    let (lines, reader) = read_child_lines(&mut process);
    let mut snapshots = vec![recv_watch_line(&lines)];

    fs::write(directory.join("active.lock"), b"volatile").expect("lock must be changed");
    fs::write(directory.join(".inspection.tmp"), b"volatile")
        .expect("temporary entry must be changed");
    match lines.recv_timeout(Duration::from_millis(350)) {
        Err(RecvTimeoutError::Timeout) => world.watch_quiet_before_completion = Some(true),
        Ok(snapshot) => {
            world.watch_quiet_before_completion = Some(false);
            snapshots.push(snapshot);
        }
        Err(RecvTimeoutError::Disconnected) => {
            panic!("watch stdout must remain connected before completion")
        }
    }

    fs::write(
        directory.join("0.first.attempt.yaml"),
        b"input: []\nevents:\n- type: completed\n",
    )
    .expect("completion must be published");
    let output = process.wait_with_output();
    reader.join().expect("watch stdout reader must finish");
    snapshots.extend(lines.try_iter());
    world.observed = Some(observe_watch_output(&output, &snapshots));
}

#[when("active watch получает SIGTERM после initial snapshot")]
fn active_watch_receives_sigterm(world: &mut InspectionWorld) {
    world.capture_snapshot();
    let mut process = spawn_json_watch(world.root(), 10);
    let (lines, reader) = read_child_lines(&mut process);
    let initial = recv_watch_line(&lines);
    let status = Command::new("/bin/kill")
        .args(["-TERM", &process.id().to_string()])
        .status()
        .expect("SIGTERM must be sent");
    assert!(status.success());

    let output = process.wait_with_output();
    reader.join().expect("watch stdout reader must finish");
    let mut snapshots = vec![initial];
    snapshots.extend(lines.try_iter());
    world.observed = Some(observe_watch_output(&output, &snapshots));
}

#[when("запускается orchestrator run verify в JSON")]
fn verify_json_through_cli(world: &mut InspectionWorld) {
    run_cli(world, ["run", "verify", "--format", "json"]);
}

#[when("запускается orchestrator run verify 10 в JSON")]
fn verify_selected_run_json_through_cli(world: &mut InspectionWorld) {
    run_cli(world, ["run", "verify", "10", "--format", "json"]);
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

#[then(
    "text list содержит по одной отсортированной summary-строке для active, закрытого условного и completed"
)]
fn list_is_sorted_with_states(world: &mut InspectionWorld) {
    assert_eq!(
        world.stdout_text(),
        "run 10: workflow=active state=active\nrun 20: workflow=skipped state=completed\nrun 30: workflow=completed state=completed\n"
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

#[then("список artifacts содержит две опубликованные версии")]
fn artifacts_list_has_two_versions(world: &mut InspectionWorld) {
    let output = world.stdout_text();
    assert!(output.contains("artifact 0: step=first input=result bytes=3 path="));
    assert!(output.contains("artifact 2: step=first input=result bytes=6 path="));
    assert!(!output.contains("artifact 1:"));
}

#[then("typed snapshot содержит две версии result")]
fn typed_snapshot_has_two_versions(world: &mut InspectionWorld) {
    let snapshot = world
        .typed_snapshot
        .as_ref()
        .expect("typed snapshot must be captured");
    assert_eq!(snapshot.run_id(), 30);
    assert_eq!(snapshot.artifacts().len(), 2);
    assert!(
        snapshot
            .artifacts()
            .iter()
            .all(|artifact| artifact.input() == "result")
    );
    assert_eq!(snapshot.artifacts()[0].attempt(), 0);
    assert_eq!(snapshot.artifacts()[1].attempt(), 2);
}

#[then("typed inspection snapshot отсутствует")]
fn typed_snapshot_is_absent(world: &mut InspectionWorld) {
    assert!(world.typed_snapshot.is_none());
}

#[then(expr = "JSON show содержит snake_case typed snapshot run {int} с number, null и arrays")]
fn json_show_has_typed_snapshot(world: &mut InspectionWorld, run_id: u64) {
    let value: serde_json::Value =
        serde_json::from_slice(&world.observed().stdout).expect("show stdout must be JSON");
    assert_eq!(value["run_id"], run_id);
    assert_eq!(value["attempts"][0]["number"], 0);
    assert!(value["attempts"][0]["session"].is_null());
    assert!(value["steps"].is_array());
    assert!(value["attempts"].is_array());
    assert!(value["frontier"]["ready"].is_array());
    assert!(value["frontier"]["missing"].is_array());
    assert!(value["artifacts"].is_array());
}

#[then("inspection output содержит только completed run")]
fn filtered_output_contains_only_completed(world: &mut InspectionWorld) {
    assert_eq!(
        world.stdout_text(),
        "run 30: workflow=completed state=completed\n"
    );
}

#[then("watch опубликовал один JSON snapshot")]
fn watch_published_one_json_snapshot(world: &mut InspectionWorld) {
    let lines = world.stdout_text().lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 1);
    let value: serde_json::Value = serde_json::from_str(lines[0]).expect("watch line must be JSON");
    assert_eq!(value["run_id"], 30);
    assert_eq!(value["state"], "completed");
}

#[then("watch опубликовал initial active и final completed snapshots")]
fn watch_published_active_and_completed_snapshots(world: &mut InspectionWorld) {
    let lines = world.stdout_text().lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 2);
    let initial: serde_json::Value =
        serde_json::from_str(lines[0]).expect("initial watch line must be JSON");
    let completed: serde_json::Value =
        serde_json::from_str(lines[1]).expect("completed watch line must be JSON");
    assert_eq!(initial["run_id"], 10);
    assert_eq!(initial["state"], "active");
    assert_eq!(completed["run_id"], 10);
    assert_eq!(completed["state"], "completed");
}

#[then("watch не публиковал snapshot для volatile изменений за три polling interval")]
fn watch_ignored_volatile_changes(world: &mut InspectionWorld) {
    assert_eq!(world.watch_quiet_before_completion, Some(true));
}

#[then("watch опубликовал только initial active snapshot")]
fn watch_published_only_initial_active_snapshot(world: &mut InspectionWorld) {
    let lines = world.stdout_text().lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 1);
    let initial: serde_json::Value =
        serde_json::from_str(lines[0]).expect("initial watch line must be JSON");
    assert_eq!(initial["run_id"], 10);
    assert_eq!(initial["state"], "active");
}

#[then("verify JSON report содержит все три runs по RunId и diagnostics каждого invalid run")]
fn verify_report_contains_all_runs(world: &mut InspectionWorld) {
    let value: serde_json::Value =
        serde_json::from_slice(&world.observed().stdout).expect("verify stdout must be JSON");
    assert_eq!(value["runs"].as_array().map(Vec::len), Some(3));
    assert_eq!(value["runs"][0]["run_id"], 10);
    assert_eq!(value["runs"][1]["run_id"], 20);
    assert_eq!(value["runs"][2]["run_id"], 30);
    assert_eq!(value["runs"][0]["valid"], true);
    assert_eq!(value["runs"][1]["valid"], false);
    assert_eq!(value["runs"][2]["valid"], false);
    assert_eq!(
        value["runs"][0]["diagnostics"].as_array().map(Vec::len),
        Some(0)
    );
    assert!(
        value["runs"][1]["diagnostics"]
            .as_array()
            .is_some_and(|diagnostics| !diagnostics.is_empty())
    );
    assert!(
        value["runs"][2]["diagnostics"]
            .as_array()
            .is_some_and(|diagnostics| !diagnostics.is_empty())
    );
}

#[then("verify JSON report содержит только valid run 10")]
fn verify_report_contains_only_selected_run(world: &mut InspectionWorld) {
    let value: serde_json::Value =
        serde_json::from_slice(&world.observed().stdout).expect("verify stdout must be JSON");
    assert_eq!(value["runs"].as_array().map(Vec::len), Some(1));
    assert_eq!(value["runs"][0]["run_id"], 10);
    assert_eq!(value["runs"][0]["valid"], true);
    assert_eq!(
        value["runs"][0]["diagnostics"].as_array().map(Vec::len),
        Some(0)
    );
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

fn spawn_json_watch(root: &Path, run_id: u64) -> ChildGuard {
    ChildGuard::new(
        Command::new(env!("CARGO_BIN_EXE_orchestrator"))
            .args(["run", "watch", &run_id.to_string(), "--format", "json"])
            .env("ORC_HOME", root)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("orchestrator watch must start"),
    )
}

fn read_child_lines(process: &mut ChildGuard) -> (Receiver<String>, JoinHandle<()>) {
    let stdout = process
        .child_mut()
        .stdout
        .take()
        .expect("watch stdout must be piped");
    let (sender, receiver) = mpsc::channel();
    let reader = thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else {
                break;
            };
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    (receiver, reader)
}

fn recv_watch_line(lines: &Receiver<String>) -> String {
    lines
        .recv_timeout(Duration::from_secs(2))
        .expect("watch must publish snapshot before deadline")
}

fn observe_watch_output(output: &std::process::Output, snapshots: &[String]) -> Observed {
    let mut stdout = snapshots.join("\n").into_bytes();
    if !stdout.is_empty() {
        stdout.push(b'\n');
    }
    Observed {
        exit_code: u8::try_from(output.status.code().expect("process must exit normally"))
            .expect("fixture exit code must fit u8"),
        stdout,
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
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
        current_dir: None,
        path: None,
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
            "input: []\nevents:\n- type: session-activated\n  session-id: previous-session\n- type: session-activated\n  session-id: native-session\n- type: completed\n",
        )],
    );
}

fn write_blocked_run(root: &Path, run_id: u64) {
    let directory = run_directory(root, run_id);
    let spec = single_step_spec("skipped", "[{one-of: [fix, done]}]")
        + "- id: fix\n  agent:\n    type: codex\n    model: model\n    reasoning: high\n  prompt: null\n  human: false\n  depends-on: [{step: first, output: fix}]\n  outputs: []\n";
    write_run_files(
        &directory,
        &spec,
        &[(
            "0.first.attempt.yaml",
            "input: []\nevents:\n- type: completed\n",
        )],
    );
    fs::write(directory.join("0.first.done.artifact"), "done").unwrap();
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
