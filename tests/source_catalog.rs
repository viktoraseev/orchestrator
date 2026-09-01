//! Cucumber-проверка публичного API и CLI source catalogs.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use cucumber::{World, given, then, when};
use orchestrator::{
    AgentCatalogEntry, InspectionFormat, ProcessEnvironment, PromptCatalogEntry,
    execute_workflow_list, list_agents, list_prompts,
};
use tempfile::TempDir;

#[derive(Debug, Default, World)]
struct CatalogWorld {
    root: Option<TempDir>,
    observed: Option<Observed>,
    initial_state: Vec<(PathBuf, Vec<u8>)>,
    agents: Vec<AgentCatalogEntry>,
    prompts: Vec<PromptCatalogEntry>,
}

#[derive(Debug)]
struct Observed {
    code: u8,
    stdout: Vec<u8>,
    stderr: String,
}

#[given("подготовлен пустой source catalog root")]
fn empty_root(world: &mut CatalogWorld) {
    world.root = Some(TempDir::new().expect("catalog root must be created"));
}

#[given("подготовлены workflow templates beta и alpha с посторонними entries")]
fn workflow_templates(world: &mut CatalogWorld) {
    empty_root(world);
    let directory = world.root().join("workflow");
    fs::create_dir(&directory).expect("workflow directory must be created");
    fs::write(directory.join("beta.yaml"), b"not parsed").expect("beta must be written");
    fs::write(directory.join("alpha.yaml"), b"also not parsed").expect("alpha must be written");
    fs::write(directory.join("notes.txt"), b"ignored").expect("foreign file must be written");
    fs::write(directory.join(".draft.yaml"), b"ignored").expect("temporary file must be written");
}

#[given("подготовлен workflow contract file с невалидным ID")]
fn invalid_workflow_id(world: &mut CatalogWorld) {
    empty_root(world);
    let directory = world.root().join("workflow");
    fs::create_dir(&directory).expect("workflow directory must be created");
    fs::write(directory.join("Bad-ID.yaml"), b"steps: []\n").expect("workflow must be written");
}

#[given("подготовлен config с Agents beta и alpha")]
fn agent_config(world: &mut CatalogWorld) {
    empty_root(world);
    fs::write(
        world.root().join("config.yaml"),
        b"agents:\n  beta:\n    type: claude\n    model: opus\n    reasoning: high\n  alpha:\n    type: codex\n    model: gpt\n    reasoning: high\n",
    )
    .expect("config must be written");
}

#[given("подготовлен невалидный config для Agent catalog")]
fn invalid_agent_config(world: &mut CatalogWorld) {
    empty_root(world);
    fs::write(world.root().join("config.yaml"), b"unknown: true\n")
        .expect("invalid config must be written");
}

#[given("подготовлен prompt template unicode")]
fn unicode_prompt(world: &mut CatalogWorld) {
    empty_root(world);
    let directory = world.root().join("prompt");
    fs::create_dir(&directory).expect("prompt directory must be created");
    fs::write(directory.join("unicode.md"), "Привет").expect("prompt must be written");
}

#[given("подготовлены prompt templates beta и alpha с temporary entry")]
fn prompt_templates(world: &mut CatalogWorld) {
    empty_root(world);
    let directory = world.root().join("prompt");
    fs::create_dir(&directory).expect("prompt directory must be created");
    fs::write(directory.join("beta.md"), "beta").expect("beta must be written");
    fs::write(directory.join("alpha.md"), "альфа").expect("alpha must be written");
    fs::write(directory.join(".draft.md"), "ignored").expect("temporary file must be written");
}

#[given("подготовлен non-UTF-8 prompt template")]
fn non_utf8_prompt(world: &mut CatalogWorld) {
    empty_root(world);
    let directory = world.root().join("prompt");
    fs::create_dir(&directory).expect("prompt directory must be created");
    fs::write(directory.join("binary.md"), [0xff, 0xfe]).expect("prompt must be written");
}

#[when("workflow catalog строится через публичный API")]
fn workflow_catalog_api(world: &mut CatalogWorld) {
    world.capture_state();
    world.observed = Some(observe_api(execute_workflow_list(
        &environment(world.root()),
        InspectionFormat::Text,
    )));
}

#[when("Agent catalog строится через публичный API")]
fn agent_catalog_api(world: &mut CatalogWorld) {
    world.capture_state();
    match list_agents(&environment(world.root())) {
        Ok(entries) => {
            world.agents = entries;
            world.observed = Some(success());
        }
        Err(error) => world.observed = Some(failure(&error)),
    }
}

#[when("prompt catalog строится через публичный API")]
fn prompt_catalog_api(world: &mut CatalogWorld) {
    world.capture_state();
    match list_prompts(&environment(world.root())) {
        Ok(entries) => {
            world.prompts = entries;
            world.observed = Some(success());
        }
        Err(error) => world.observed = Some(failure(&error)),
    }
}

#[when("запускается orchestrator workflow list в JSON")]
fn workflow_json_cli(world: &mut CatalogWorld) {
    run_cli(world, &["workflow", "list", "--format", "json"]);
}

#[when("запускается orchestrator workflow list")]
fn workflow_text_cli(world: &mut CatalogWorld) {
    run_cli(world, &["workflow", "list"]);
}

#[when("запускается orchestrator agent list")]
fn agent_text_cli(world: &mut CatalogWorld) {
    run_cli(world, &["agent", "list"]);
}

#[when("запускается orchestrator agent list в JSON")]
fn agent_json_cli(world: &mut CatalogWorld) {
    run_cli(world, &["agent", "list", "--format", "json"]);
}

#[when("запускается orchestrator prompt list в JSON")]
fn prompt_json_cli(world: &mut CatalogWorld) {
    run_cli(world, &["prompt", "list", "--format", "json"]);
}

#[when("запускается orchestrator prompt list")]
fn prompt_text_cli(world: &mut CatalogWorld) {
    run_cli(world, &["prompt", "list"]);
}

#[then(expr = "catalog завершается с кодом {int}")]
fn catalog_code(world: &mut CatalogWorld, code: u8) {
    assert_eq!(world.observed().code, code, "{}", world.observed().stderr);
}

#[then("catalog output пуст")]
fn catalog_output_empty(world: &mut CatalogWorld) {
    assert!(world.observed().stdout.is_empty());
}

#[then("JSON workflow catalog содержит alpha и beta по порядку")]
fn workflow_json_sorted(world: &mut CatalogWorld) {
    let value = world.json();
    assert_eq!(value[0]["workflow"], "alpha");
    assert_eq!(value[1]["workflow"], "beta");
    assert!(Path::new(value[0]["path"].as_str().expect("path must be a string")).is_absolute());
}

#[then("typed Agent catalog содержит alpha и beta по порядку")]
fn typed_agents_sorted(world: &mut CatalogWorld) {
    assert_eq!(world.agents.len(), 2);
    assert_eq!(world.agents[0].agent(), "alpha");
    assert_eq!(world.agents[0].agent_type(), "codex");
    assert_eq!(world.agents[1].agent(), "beta");
    assert_eq!(world.agents[1].model(), "opus");
}

#[then("Agent catalog text содержит обе validated записи по порядку")]
fn agent_text_sorted(world: &mut CatalogWorld) {
    assert_eq!(
        world.stdout(),
        "agent alpha: type=codex model=gpt reasoning=high\nagent beta: type=claude model=opus reasoning=high\n"
    );
}

#[then("typed prompt unicode имеет документированный размер bytes")]
fn unicode_prompt_bytes(world: &mut CatalogWorld) {
    assert_eq!(world.prompts.len(), 1);
    assert_eq!(world.prompts[0].prompt(), "unicode");
    assert_eq!(world.prompts[0].bytes(), 12);
}

#[then("JSON prompt catalog содержит alpha и beta по порядку")]
fn prompt_json_sorted(world: &mut CatalogWorld) {
    let value = world.json();
    assert_eq!(value[0]["prompt"], "alpha");
    assert_eq!(value[0]["bytes"], 10);
    assert_eq!(value[1]["prompt"], "beta");
    assert_eq!(value.as_array().map(Vec::len), Some(2));
}

#[then("catalog не изменил source state")]
fn source_state_unchanged(world: &mut CatalogWorld) {
    assert_eq!(world.initial_state, snapshot(world.root()));
}

impl CatalogWorld {
    fn root(&self) -> &Path {
        self.root
            .as_ref()
            .expect("scenario must define root")
            .path()
    }

    fn observed(&self) -> &Observed {
        self.observed.as_ref().expect("scenario must run catalog")
    }

    fn stdout(&self) -> &str {
        std::str::from_utf8(&self.observed().stdout).expect("stdout must be UTF-8")
    }

    fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.observed().stdout).expect("stdout must be JSON")
    }

    fn capture_state(&mut self) {
        self.initial_state = snapshot(self.root());
    }
}

fn run_cli(world: &mut CatalogWorld, arguments: &[&str]) {
    world.capture_state();
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

fn observe_api(result: Result<String, orchestrator::CommandError>) -> Observed {
    match result {
        Ok(output) => Observed {
            code: 0,
            stdout: output.into_bytes(),
            stderr: String::new(),
        },
        Err(error) => failure(&error),
    }
}

fn success() -> Observed {
    Observed {
        code: 0,
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

fn environment(root: &Path) -> ProcessEnvironment {
    ProcessEnvironment {
        home: None,
        orc_home: Some(root.as_os_str().to_owned()),
    }
}

fn snapshot(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut files = Vec::new();
    let mut directories = vec![root.to_owned()];
    while let Some(directory) = directories.pop() {
        for entry in fs::read_dir(directory).expect("catalog directory must be readable") {
            let path = entry.expect("catalog entry must be readable").path();
            if path.is_dir() {
                directories.push(path);
            } else {
                files.push((
                    path.strip_prefix(root)
                        .expect("path must be inside root")
                        .to_owned(),
                    fs::read(path).expect("catalog file must be readable"),
                ));
            }
        }
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));
    files
}

#[tokio::main]
async fn main() {
    CatalogWorld::run("features/source_catalog.feature").await;
}
