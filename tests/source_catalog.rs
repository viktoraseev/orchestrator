//! Cucumber-проверка публичного API и CLI source catalogs.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use cucumber::{World, given, then, when};
use orchestrator::{
    AgentCatalogEntry, InspectionFormat, ProcessEnvironment, PromptCatalogEntry, PromptTemplate,
    SourceWorkflow, execute_workflow_list, list_agents, list_prompts, show_agent, show_prompt,
    show_workflow,
};
use tempfile::TempDir;

#[derive(Debug, Default, World)]
struct CatalogWorld {
    root: Option<TempDir>,
    observed: Option<Observed>,
    initial_state: Vec<(PathBuf, Vec<u8>)>,
    agents: Vec<AgentCatalogEntry>,
    prompts: Vec<PromptCatalogEntry>,
    shown_workflow: Option<SourceWorkflow>,
    shown_agent: Option<AgentCatalogEntry>,
    shown_prompt: Option<PromptTemplate>,
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

#[given("подготовлен workflow contract path, который является directory")]
fn non_regular_workflow(world: &mut CatalogWorld) {
    empty_root(world);
    let directory = world.root().join("workflow");
    fs::create_dir(&directory).expect("workflow directory must be created");
    fs::create_dir(directory.join("nested.yaml"))
        .expect("workflow contract directory must be created");
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

#[given("подготовлен prompt template unicode с содержимым Привет")]
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

#[given("подготовлен prompt contract file с невалидным ID")]
fn invalid_prompt_id(world: &mut CatalogWorld) {
    empty_root(world);
    let directory = world.root().join("prompt");
    fs::create_dir(&directory).expect("prompt directory must be created");
    fs::write(directory.join("Bad-ID.md"), "prompt").expect("prompt must be written");
}

#[given("подготовлен prompt contract path, который является directory")]
fn non_regular_prompt(world: &mut CatalogWorld) {
    empty_root(world);
    let directory = world.root().join("prompt");
    fs::create_dir(&directory).expect("prompt directory must be created");
    fs::create_dir(directory.join("nested.md")).expect("prompt contract directory must be created");
}

#[given("подготовлен source workflow demo с несуществующими Agent и prompt references")]
fn source_workflow_with_unresolved_references(world: &mut CatalogWorld) {
    empty_root(world);
    let directory = world.root().join("workflow");
    fs::create_dir(&directory).expect("workflow directory must be created");
    fs::write(
        directory.join("demo.yaml"),
        b"steps:\n  - id: plan\n    agent: missing-agent\n    prompt: missing-prompt\n    human: false\n    depends-on: []\n    outputs: [brief]\n  - id: write\n    agent: missing-writer\n    prompt: missing-draft\n    human: true\n    depends-on: [ghost-step]\n    outputs: [article]\n",
    )
    .expect("workflow must be written");
}

#[given("подготовлен source workflow demo с пустым списком Steps")]
fn source_workflow_with_empty_steps(world: &mut CatalogWorld) {
    empty_root(world);
    let directory = world.root().join("workflow");
    fs::create_dir(&directory).expect("workflow directory must be created");
    fs::write(directory.join("demo.yaml"), b"steps: []\n").expect("workflow must be written");
}

#[given("подготовлен prompt demo без финального newline и повреждённый соседний template")]
fn selected_prompt_without_newline(world: &mut CatalogWorld) {
    empty_root(world);
    let directory = world.root().join("prompt");
    fs::create_dir(&directory).expect("prompt directory must be created");
    fs::write(directory.join("demo.md"), "точный prompt").expect("prompt must be written");
    fs::write(directory.join("broken.md"), [0xff]).expect("neighbor must be written");
}

#[given("подготовлен prompt demo с YAML-похожим Markdown")]
fn yaml_like_prompt(world: &mut CatalogWorld) {
    empty_root(world);
    let directory = world.root().join("prompt");
    fs::create_dir(&directory).expect("prompt directory must be created");
    fs::write(directory.join("demo.md"), "title: value\n- raw item")
        .expect("YAML-like prompt must be written");
}

#[given("подготовлен prompt demo с финальным newline")]
fn selected_prompt_with_newline(world: &mut CatalogWorld) {
    empty_root(world);
    let directory = world.root().join("prompt");
    fs::create_dir(&directory).expect("prompt directory must be created");
    fs::write(directory.join("demo.md"), "строка\n").expect("prompt must be written");
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

#[when("workflow demo читается через публичный API")]
fn workflow_show_api(world: &mut CatalogWorld) {
    world.capture_state();
    match show_workflow("demo", &environment(world.root())) {
        Ok(workflow) => {
            world.shown_workflow = Some(workflow);
            world.observed = Some(success());
        }
        Err(error) => world.observed = Some(failure(&error)),
    }
}

#[when("Agent alpha читается через публичный API")]
fn agent_show_api(world: &mut CatalogWorld) {
    world.capture_state();
    match show_agent("alpha", &environment(world.root())) {
        Ok(agent) => {
            world.shown_agent = Some(agent);
            world.observed = Some(success());
        }
        Err(error) => world.observed = Some(failure(&error)),
    }
}

#[when("prompt demo читается через публичный API")]
fn prompt_show_api(world: &mut CatalogWorld) {
    world.capture_state();
    match show_prompt("demo", &environment(world.root())) {
        Ok(prompt) => {
            world.shown_prompt = Some(prompt);
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

#[when("запускается orchestrator workflow show demo в JSON")]
fn workflow_show_json_cli(world: &mut CatalogWorld) {
    run_cli(world, &["workflow", "show", "demo", "--format", "json"]);
}

#[when("запускается orchestrator workflow show demo")]
fn workflow_show_text_cli(world: &mut CatalogWorld) {
    run_cli(world, &["workflow", "show", "demo"]);
}

#[when("запускается orchestrator workflow show missing")]
fn missing_workflow_show_cli(world: &mut CatalogWorld) {
    run_cli(world, &["workflow", "show", "missing"]);
}

#[when("запускается orchestrator workflow show с невалидным ID")]
fn invalid_workflow_show_cli(world: &mut CatalogWorld) {
    run_cli(world, &["workflow", "show", "Bad.ID"]);
}

#[when("запускается orchestrator agent show beta в JSON")]
fn agent_show_json_cli(world: &mut CatalogWorld) {
    run_cli(world, &["agent", "show", "beta", "--format", "json"]);
}

#[when("запускается orchestrator agent show alpha")]
fn agent_show_text_cli(world: &mut CatalogWorld) {
    run_cli(world, &["agent", "show", "alpha"]);
}

#[when("запускается orchestrator agent show с невалидным ID")]
fn invalid_agent_show_cli(world: &mut CatalogWorld) {
    run_cli(world, &["agent", "show", "Bad-ID"]);
}

#[when("запускается orchestrator agent show missing")]
fn missing_agent_show_cli(world: &mut CatalogWorld) {
    run_cli(world, &["agent", "show", "missing"]);
}

#[when("запускается orchestrator prompt show demo")]
fn prompt_show_text_cli(world: &mut CatalogWorld) {
    run_cli(world, &["prompt", "show", "demo"]);
}

#[when("запускается orchestrator prompt show demo в JSON")]
fn prompt_show_json_cli(world: &mut CatalogWorld) {
    run_cli(world, &["prompt", "show", "demo", "--format", "json"]);
}

#[when("запускается orchestrator prompt show missing")]
fn missing_prompt_show_cli(world: &mut CatalogWorld) {
    run_cli(world, &["prompt", "show", "missing"]);
}

#[when("запускается orchestrator prompt show с невалидным ID")]
fn invalid_prompt_show_cli(world: &mut CatalogWorld) {
    run_cli(world, &["prompt", "show", "Bad.ID"]);
}

#[when("запускается orchestrator prompt show binary")]
fn non_utf8_prompt_show_cli(world: &mut CatalogWorld) {
    run_cli(world, &["prompt", "show", "binary"]);
}

#[then(expr = "catalog завершается с кодом {int}")]
fn catalog_code(world: &mut CatalogWorld, code: u8) {
    assert_eq!(world.observed().code, code, "{}", world.observed().stderr);
}

#[then("catalog output пуст")]
fn catalog_output_empty(world: &mut CatalogWorld) {
    assert!(world.observed().stdout.is_empty());
}

#[then("JSON workflow catalog равен alpha и beta с absolute paths по порядку")]
fn workflow_json_sorted(world: &mut CatalogWorld) {
    let value = world.json();
    assert_eq!(value[0]["workflow"], "alpha");
    assert_eq!(value[1]["workflow"], "beta");
    assert!(Path::new(value[0]["path"].as_str().expect("path must be a string")).is_absolute());
    assert!(Path::new(value[1]["path"].as_str().expect("path must be a string")).is_absolute());
    assert_eq!(value.as_array().map(Vec::len), Some(2));
}

#[then("workflow catalog text равен строкам alpha и beta")]
fn workflow_text_sorted(world: &mut CatalogWorld) {
    assert_eq!(world.stdout(), "alpha\nbeta\n");
}

#[then("typed Agent catalog равен alpha codex gpt high и beta claude opus high по порядку")]
fn typed_agents_sorted(world: &mut CatalogWorld) {
    assert_eq!(world.agents.len(), 2);
    assert_eq!(world.agents[0].agent(), "alpha");
    assert_eq!(world.agents[0].agent_type(), "codex");
    assert_eq!(world.agents[0].model(), "gpt");
    assert_eq!(world.agents[0].reasoning(), "high");
    assert_eq!(world.agents[1].agent(), "beta");
    assert_eq!(world.agents[1].agent_type(), "claude");
    assert_eq!(world.agents[1].model(), "opus");
    assert_eq!(world.agents[1].reasoning(), "high");
}

#[then("Agent catalog text равен строкам alpha codex gpt high и beta claude opus high")]
fn agent_text_sorted(world: &mut CatalogWorld) {
    assert_eq!(
        world.stdout(),
        "agent alpha: type=codex model=gpt reasoning=high\nagent beta: type=claude model=opus reasoning=high\n"
    );
}

#[then("JSON Agent catalog равен alpha codex gpt high и beta claude opus high по порядку")]
fn agent_json_sorted(world: &mut CatalogWorld) {
    let value = world.json();
    assert_eq!(value[0]["agent"], "alpha");
    assert_eq!(value[0]["type"], "codex");
    assert_eq!(value[0]["model"], "gpt");
    assert_eq!(value[0]["reasoning"], "high");
    assert_eq!(value[1]["agent"], "beta");
    assert_eq!(value[1]["type"], "claude");
    assert_eq!(value[1]["model"], "opus");
    assert_eq!(value[1]["reasoning"], "high");
    assert_eq!(value.as_array().map(Vec::len), Some(2));
}

#[then("typed prompt unicode имеет размер 12 bytes")]
fn unicode_prompt_bytes(world: &mut CatalogWorld) {
    assert_eq!(world.prompts.len(), 1);
    assert_eq!(world.prompts[0].prompt(), "unicode");
    assert_eq!(world.prompts[0].bytes(), 12);
}

#[then("JSON prompt catalog равен alpha 10 bytes и beta 4 bytes с absolute paths по порядку")]
fn prompt_json_sorted(world: &mut CatalogWorld) {
    let value = world.json();
    assert_eq!(value[0]["prompt"], "alpha");
    assert_eq!(value[0]["bytes"], 10);
    assert!(Path::new(value[0]["path"].as_str().expect("path must be a string")).is_absolute());
    assert_eq!(value[1]["prompt"], "beta");
    assert_eq!(value[1]["bytes"], 4);
    assert!(Path::new(value[1]["path"].as_str().expect("path must be a string")).is_absolute());
    assert_eq!(value.as_array().map(Vec::len), Some(2));
}

#[then("prompt catalog text равен alpha 10 bytes и beta 4 bytes с absolute paths по порядку")]
fn prompt_text_sorted(world: &mut CatalogWorld) {
    let lines = world.stdout().lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].starts_with("prompt alpha: bytes=10 path=/"));
    assert!(lines[0].ends_with("/prompt/alpha.md"));
    assert!(lines[1].starts_with("prompt beta: bytes=4 path=/"));
    assert!(lines[1].ends_with("/prompt/beta.md"));
}

#[then("JSON catalog равен пустому array")]
fn empty_json_catalog(world: &mut CatalogWorld) {
    assert_eq!(world.json().as_array().map(Vec::len), Some(0));
}

#[then("typed source workflow содержит исходные Steps и references")]
fn typed_source_workflow(world: &mut CatalogWorld) {
    let workflow = world
        .shown_workflow
        .as_ref()
        .expect("workflow show must succeed");
    assert_eq!(workflow.workflow(), "demo");
    assert!(Path::new(workflow.path()).is_absolute());
    assert_eq!(workflow.steps()[0].id(), "plan");
    assert_eq!(workflow.steps()[0].agent(), Some("missing-agent"));
    assert_eq!(workflow.steps()[0].prompt(), Some("missing-prompt"));
    assert_eq!(workflow.steps()[0].outputs(), ["brief"]);
    assert_eq!(workflow.steps()[1].id(), "write");
    assert!(workflow.steps()[1].is_human());
    assert_eq!(workflow.steps()[1].depends_on(), ["ghost-step"]);
}

#[then("JSON workflow show содержит абсолютный path и Steps в source order")]
fn workflow_show_json(world: &mut CatalogWorld) {
    let value = world.json();
    assert_eq!(value["workflow"], "demo");
    assert!(Path::new(value["path"].as_str().expect("path must be a string")).is_absolute());
    assert_eq!(value["steps"][0]["id"], "plan");
    assert_eq!(value["steps"][0]["agent"], "missing-agent");
    assert_eq!(value["steps"][1]["id"], "write");
    assert_eq!(value["steps"][1]["depends_on"][0], "ghost-step");
}

#[then("workflow show text содержит header и обе source Step строки")]
fn workflow_show_text(world: &mut CatalogWorld) {
    let lines = world.stdout().lines().collect::<Vec<_>>();
    assert!(lines[0].starts_with("workflow demo: path=/"));
    assert_eq!(
        lines[1],
        "step plan: agent=missing-agent prompt=missing-prompt human=false depends-on=- outputs=brief"
    );
    assert_eq!(
        lines[2],
        "step write: agent=missing-writer prompt=missing-draft human=true depends-on=ghost-step outputs=article"
    );
    assert_eq!(lines.len(), 3);
}

#[then("typed Agent show равен alpha codex gpt high")]
fn typed_agent_show(world: &mut CatalogWorld) {
    let agent = world.shown_agent.as_ref().expect("agent show must succeed");
    assert_eq!(agent.agent(), "alpha");
    assert_eq!(agent.agent_type(), "codex");
    assert_eq!(agent.model(), "gpt");
    assert_eq!(agent.reasoning(), "high");
}

#[then("JSON Agent show равен beta claude opus high")]
fn agent_show_json(world: &mut CatalogWorld) {
    let value = world.json();
    assert_eq!(value["agent"], "beta");
    assert_eq!(value["type"], "claude");
    assert_eq!(value["model"], "opus");
    assert_eq!(value["reasoning"], "high");
}

#[then("Agent show text равен строке agent alpha: type=codex model=gpt reasoning=high")]
fn agent_show_text(world: &mut CatalogWorld) {
    assert_eq!(
        world.stdout(),
        "agent alpha: type=codex model=gpt reasoning=high\n"
    );
}

#[then("typed prompt show содержит точное содержимое demo")]
fn typed_prompt_show(world: &mut CatalogWorld) {
    let prompt = world
        .shown_prompt
        .as_ref()
        .expect("prompt show must succeed");
    assert_eq!(prompt.prompt(), "demo");
    assert_eq!(prompt.content(), "точный prompt");
    assert_eq!(prompt.bytes(), 19);
    assert!(Path::new(prompt.path()).is_absolute());
}

#[then("typed prompt show содержит точный YAML-похожий Markdown")]
fn typed_prompt_show_keeps_yaml_like_markdown(world: &mut CatalogWorld) {
    let prompt = world
        .shown_prompt
        .as_ref()
        .expect("prompt show must succeed");
    assert_eq!(prompt.content(), "title: value\n- raw item");
}

#[then("prompt show stdout побайтово равен template без newline")]
fn prompt_show_exact_stdout(world: &mut CatalogWorld) {
    assert_eq!(world.observed().stdout, "точный prompt".as_bytes());
}

#[then("JSON prompt show содержит точный content и bytes")]
fn prompt_show_json(world: &mut CatalogWorld) {
    let value = world.json();
    assert_eq!(value["prompt"], "demo");
    assert_eq!(value["content"], "строка\n");
    assert_eq!(value["bytes"], 13);
    assert!(Path::new(value["path"].as_str().expect("path must be a string")).is_absolute());
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
        current_dir: None,
        path: None,
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
