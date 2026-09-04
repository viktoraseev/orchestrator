//! Cucumber-проверка read-only bulk validation, source graph и materialized plan.

use std::fs;
use std::path::Path;
use std::process::Command;

use cucumber::{World, given, then, when};
use orchestrator::{
    ProcessEnvironment, WorkflowGraph, WorkflowPlan, WorkflowValidationReport,
    build_workflow_graph, build_workflow_plan, validate_all_workflows,
};
use tempfile::TempDir;

#[derive(Debug, Default, World)]
struct WorkflowToolWorld {
    root: Option<TempDir>,
    observed: Option<Observed>,
    report: Option<WorkflowValidationReport>,
    graph: Option<WorkflowGraph>,
    plan: Option<WorkflowPlan>,
}

#[derive(Debug)]
struct Observed {
    code: u8,
    stdout: Vec<u8>,
    stderr: String,
}

#[given("подготовлен пустой workflow tool root")]
fn empty_root(world: &mut WorkflowToolWorld) {
    world.root = Some(TempDir::new().expect("workflow tool root must be created"));
}

#[given("подготовлен workflow catalog с valid beta и invalid alpha")]
fn mixed_workflow_catalog(world: &mut WorkflowToolWorld) {
    empty_root(world);
    prepare_directories(world.root());
    write_config(world.root());
    fs::write(world.root().join("workflow/alpha.yaml"), "steps: []\n")
        .expect("alpha must be written");
    fs::write(world.root().join("workflow/beta.yaml"), single_step(None))
        .expect("beta must be written");
}

#[given("подготовлен source graph delivery без config и prompts")]
fn source_graph(world: &mut WorkflowToolWorld) {
    empty_root(world);
    fs::create_dir(world.root().join("workflow")).expect("workflow directory must be created");
    fs::write(
        world.root().join("workflow/delivery.yaml"),
        graph_workflow("[plan]"),
    )
    .expect("graph workflow must be written");
}

#[given("подготовлен недостижимый source graph delivery")]
fn unreachable_source_graph(world: &mut WorkflowToolWorld) {
    empty_root(world);
    fs::create_dir(world.root().join("workflow")).expect("workflow directory must be created");
    fs::write(
        world.root().join("workflow/delivery.yaml"),
        graph_workflow("[]"),
    )
    .expect("graph workflow must be written");
}

#[given("подготовлен source graph delivery с неизвестной dependency")]
fn source_graph_with_unknown_dependency(world: &mut WorkflowToolWorld) {
    empty_root(world);
    fs::create_dir(world.root().join("workflow")).expect("workflow directory must be created");
    fs::write(
        world.root().join("workflow/delivery.yaml"),
        graph_workflow("[missing]"),
    )
    .expect("graph workflow must be written");
}

#[given("подготовлен workflow tool contract file с невалидным ID")]
fn invalid_workflow_contract_id(world: &mut WorkflowToolWorld) {
    empty_root(world);
    fs::create_dir(world.root().join("workflow")).expect("workflow directory must be created");
    fs::write(world.root().join("workflow/Bad-ID.yaml"), single_step(None))
        .expect("invalid contract file must be written");
}

#[given("подготовлен полностью materializable workflow delivery")]
fn materializable_workflow(world: &mut WorkflowToolWorld) {
    empty_root(world);
    prepare_materializable(world.root());
}

#[given("подготовлен workflow delivery с non-UTF-8 prompt")]
fn workflow_with_non_utf8_prompt(world: &mut WorkflowToolWorld) {
    empty_root(world);
    prepare_materializable(world.root());
    fs::write(world.root().join("prompt/plan.md"), [0xff]).expect("invalid prompt must be written");
}

#[when("все workflows проверяются через публичный API")]
fn validate_all_api(world: &mut WorkflowToolWorld) {
    match validate_all_workflows(&environment(world.root())) {
        Ok(report) => {
            let code = if report.is_valid() { 0 } else { 3 };
            world.report = Some(report);
            world.observed = Some(success(code));
        }
        Err(error) => world.observed = Some(failure(&error)),
    }
}

#[when("graph delivery строится через публичный API")]
fn graph_api(world: &mut WorkflowToolWorld) {
    match build_workflow_graph("delivery", &environment(world.root())) {
        Ok(graph) => {
            world.graph = Some(graph);
            world.observed = Some(success(0));
        }
        Err(error) => world.observed = Some(failure(&error)),
    }
}

#[when("plan delivery строится через публичный API")]
fn plan_api(world: &mut WorkflowToolWorld) {
    match build_workflow_plan("delivery", &environment(world.root())) {
        Ok(plan) => {
            world.plan = Some(plan);
            world.observed = Some(success(0));
        }
        Err(error) => world.observed = Some(failure(&error)),
    }
}

#[when("запускается orchestrator validate --all в JSON")]
fn validate_all_json_cli(world: &mut WorkflowToolWorld) {
    run_cli(world, &["validate", "--all", "--format", "json"]);
}

#[when("запускается orchestrator validate --all")]
fn validate_all_text_cli(world: &mut WorkflowToolWorld) {
    run_cli(world, &["validate", "--all"]);
}

#[when("запускается orchestrator validate delivery --all")]
fn invalid_validate_all_cli(world: &mut WorkflowToolWorld) {
    run_cli(world, &["validate", "delivery", "--all"]);
}

#[when("запускается orchestrator workflow graph delivery")]
fn graph_text_cli(world: &mut WorkflowToolWorld) {
    run_cli(world, &["workflow", "graph", "delivery"]);
}

#[when("запускается orchestrator workflow graph delivery в JSON")]
fn graph_json_cli(world: &mut WorkflowToolWorld) {
    run_cli(
        world,
        &["workflow", "graph", "delivery", "--format", "json"],
    );
}

#[when("запускается orchestrator workflow plan delivery в JSON")]
fn plan_json_cli(world: &mut WorkflowToolWorld) {
    run_cli(world, &["workflow", "plan", "delivery", "--format", "json"]);
}

#[when("запускается orchestrator workflow plan delivery")]
fn plan_text_cli(world: &mut WorkflowToolWorld) {
    run_cli(world, &["workflow", "plan", "delivery"]);
}

#[then(expr = "workflow tool завершается с кодом {int}")]
fn exit_code(world: &mut WorkflowToolWorld, expected: u8) {
    assert_eq!(
        world.observed().code,
        expected,
        "{}",
        world.observed().stderr
    );
}

#[then("workflow tool stdout пуст")]
fn stdout_empty(world: &mut WorkflowToolWorld) {
    assert!(world.observed().stdout.is_empty());
}

#[then(
    "typed bulk report равен invalid alpha с diagnostics и valid beta без diagnostics по порядку"
)]
fn typed_bulk_report(world: &mut WorkflowToolWorld) {
    let report = world.report.as_ref().expect("bulk report must exist");
    assert_eq!(report.workflows().len(), 2);
    assert_eq!(report.workflows()[0].workflow(), "alpha");
    assert!(!report.workflows()[0].is_valid());
    assert!(report.workflows()[0].diagnostics()[0].contains("steps должен быть непустым"));
    assert_eq!(report.workflows()[1].workflow(), "beta");
    assert!(report.workflows()[1].is_valid());
    assert!(report.workflows()[1].diagnostics().is_empty());
}

#[then("JSON bulk report равен invalid alpha и valid beta по порядку")]
fn json_bulk_report(world: &mut WorkflowToolWorld) {
    let value = world.json();
    assert_eq!(value["workflows"][0]["workflow"], "alpha");
    assert_eq!(value["workflows"][0]["valid"], false);
    assert_eq!(value["workflows"][1]["workflow"], "beta");
    assert_eq!(value["workflows"][1]["valid"], true);
    assert_eq!(value["workflows"].as_array().map(Vec::len), Some(2));
}

#[then("bulk report text содержит invalid alpha перед valid beta")]
fn text_bulk_report(world: &mut WorkflowToolWorld) {
    let lines = world.stdout().lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].starts_with("workflow alpha: invalid: "));
    assert!(lines[0].contains("steps должен быть непустым"));
    assert_eq!(lines[1], "workflow beta: valid");
}

#[then(
    "typed graph равен workflow delivery, bootstrap plan, nodes plan и implement, edge plan to implement"
)]
fn typed_graph(world: &mut WorkflowToolWorld) {
    let graph = world.graph.as_ref().expect("graph must exist");
    assert_eq!(graph.workflow(), "delivery");
    assert_eq!(graph.bootstrap(), "plan");
    assert_eq!(graph.nodes(), ["plan", "implement"]);
    assert_eq!(graph.edges().len(), 1);
    assert_eq!(graph.edges()[0].from(), "plan");
    assert_eq!(graph.edges()[0].to(), "implement");
}

#[then(
    "graph text состоит из absolute path workflow delivery, bootstrap plan и edge plan to implement"
)]
fn graph_text(world: &mut WorkflowToolWorld) {
    let lines = world.stdout().lines().collect::<Vec<_>>();
    assert!(lines.first().is_some_and(|line| {
        line.starts_with("workflow delivery: path=/") && line.ends_with("/workflow/delivery.yaml")
    }));
    assert_eq!(lines.get(1), Some(&"bootstrap: plan"));
    assert_eq!(lines.get(2), Some(&"plan -> implement"));
    assert_eq!(lines.len(), 3);
}

#[then(
    "JSON graph равен workflow delivery, absolute path, bootstrap plan, nodes plan и implement, edge plan to implement"
)]
fn json_graph(world: &mut WorkflowToolWorld) {
    let value = world.json();
    assert_eq!(value["workflow"], "delivery");
    assert!(value["path"].as_str().is_some_and(
        |path| Path::new(path).is_absolute() && path.ends_with("/workflow/delivery.yaml")
    ));
    assert_eq!(value["bootstrap"], "plan");
    assert_eq!(value["nodes"][0], "plan");
    assert_eq!(value["nodes"][1], "implement");
    assert_eq!(value["nodes"].as_array().map(Vec::len), Some(2));
    assert_eq!(value["edges"][0]["from"], "plan");
    assert_eq!(value["edges"][0]["to"], "implement");
    assert_eq!(value["edges"].as_array().map(Vec::len), Some(1));
}

#[then("typed plan содержит effective Agent и prompt content")]
fn typed_plan(world: &mut WorkflowToolWorld) {
    let plan = world.plan.as_ref().expect("plan must exist");
    assert_eq!(plan.workflow_id(), "delivery");
    assert_eq!(plan.max_parallel_agents(), 7);
    let agent = plan.steps()[0]
        .agent()
        .expect("Agent Step must contain Agent");
    assert_eq!(agent.agent_type(), "codex");
    assert_eq!(agent.model(), "gpt-test");
    assert_eq!(plan.steps()[0].prompt(), Some("SECRET PLAN"));
    assert_eq!(plan.steps()[1].prompt(), Some("{{content:plan:spec}}"));
}

#[then("JSON plan содержит effective limit Agent и prompt")]
fn json_plan(world: &mut WorkflowToolWorld) {
    let value = world.json();
    assert_eq!(value["workflow_id"], "delivery");
    assert_eq!(value["max_parallel_agents"], 7);
    assert_eq!(value["steps"][0]["agent"]["type"], "codex");
    assert_eq!(value["steps"][0]["agent"]["model"], "gpt-test");
    assert_eq!(value["steps"][0]["prompt"], "SECRET PLAN");
}

#[then("plan text содержит effective summaries без prompt content")]
fn plan_text(world: &mut WorkflowToolWorld) {
    let stdout = world.stdout();
    assert!(stdout.starts_with("workflow delivery: max-parallel-agents=7\n"));
    assert!(stdout.contains(
        "step plan: type=codex model=gpt-test reasoning=high prompt-bytes=11 human=false depends-on=- outputs=spec"
    ));
    assert!(!stdout.contains("SECRET PLAN"));
}

#[then("каталог run отсутствует после workflow tool")]
fn run_absent(world: &mut WorkflowToolWorld) {
    assert!(!world.root().join("run").exists());
}

impl WorkflowToolWorld {
    fn root(&self) -> &Path {
        self.root
            .as_ref()
            .expect("scenario must define root")
            .path()
    }

    fn observed(&self) -> &Observed {
        self.observed
            .as_ref()
            .expect("scenario must execute workflow tool")
    }

    fn stdout(&self) -> &str {
        std::str::from_utf8(&self.observed().stdout).expect("stdout must be UTF-8")
    }

    fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.observed().stdout).expect("stdout must be JSON")
    }
}

fn prepare_directories(root: &Path) {
    fs::create_dir(root.join("workflow")).expect("workflow directory must be created");
    fs::create_dir(root.join("prompt")).expect("prompt directory must be created");
}

fn write_config(root: &Path) {
    fs::write(
        root.join("config.yaml"),
        "default-agent: main\nmax-parallel-agents: 7\nagents:\n  main:\n    type: codex\n    model: gpt-test\n    reasoning: high\n",
    )
    .expect("config must be written");
}

fn prepare_materializable(root: &Path) {
    prepare_directories(root);
    write_config(root);
    fs::write(
        root.join("workflow/delivery.yaml"),
        "steps:\n  - id: plan\n    prompt: plan\n    human: false\n    depends-on: []\n    outputs: [spec]\n  - id: implement\n    prompt: implement\n    human: true\n    depends-on: [plan]\n    outputs: [source]\n",
    )
    .expect("workflow must be written");
    fs::write(root.join("prompt/plan.md"), "SECRET PLAN").expect("plan prompt must be written");
    fs::write(root.join("prompt/implement.md"), "{{content:plan:spec}}")
        .expect("implement prompt must be written");
}

fn graph_workflow(implement_dependencies: &str) -> String {
    format!(
        "steps:\n  - id: plan\n    agent: missing-agent\n    prompt: missing-prompt\n    human: false\n    depends-on: []\n    outputs: [spec]\n  - id: implement\n    agent: other-agent\n    prompt: other-prompt\n    human: false\n    depends-on: {implement_dependencies}\n    outputs: [source]\n"
    )
}

fn single_step(prompt: Option<&str>) -> String {
    let prompt = prompt.map_or(String::new(), |id| format!("    prompt: {id}\n"));
    format!("steps:\n  - id: plan\n{prompt}    human: false\n    depends-on: []\n    outputs: []\n")
}

fn environment(root: &Path) -> ProcessEnvironment {
    ProcessEnvironment {
        home: None,
        orc_home: Some(root.as_os_str().to_owned()),
        current_dir: None,
        path: None,
    }
}

fn run_cli(world: &mut WorkflowToolWorld, arguments: &[&str]) {
    let output = Command::new(env!("CARGO_BIN_EXE_orchestrator"))
        .args(arguments)
        .env("ORC_HOME", world.root())
        .output()
        .expect("orchestrator must run");
    world.observed = Some(Observed {
        code: u8::try_from(output.status.code().expect("process must exit normally"))
            .expect("exit code must fit u8"),
        stdout: output.stdout,
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    });
}

fn success(code: u8) -> Observed {
    Observed {
        code,
        stdout: Vec::new(),
        stderr: String::new(),
    }
}

fn failure(error: &orchestrator::CommandError) -> Observed {
    Observed {
        code: error.exit_code(),
        stdout: Vec::new(),
        stderr: error.to_string(),
    }
}

#[tokio::main]
async fn main() {
    WorkflowToolWorld::run("features/workflow_tools.feature").await;
}
