//! Process-Cucumber для публичного контракта config-команд.

use std::fs;
use std::process::{Command, Output, Stdio};

use cucumber::{World, given, then, when};
use tempfile::TempDir;

#[derive(Debug, Default, World)]
struct ConfigWorld {
    root: Option<TempDir>,
    home: Option<TempDir>,
    relative_orc_home: bool,
    empty_orc_home: bool,
    output: Option<Output>,
    initial_config: Option<Vec<u8>>,
    concurrent_outputs: Vec<Output>,
}

#[given("изолированный корень состояния без config.yaml")]
fn isolated_root(world: &mut ConfigWorld) {
    world.root = Some(TempDir::new().expect("test root must be created"));
}

#[given("относительный ORC_HOME")]
fn relative_orc_home(world: &mut ConfigWorld) {
    world.relative_orc_home = true;
}

#[given("HOME содержит лимит 7, а абсолютный ORC_HOME содержит лимит 11")]
fn absolute_orc_home_replaces_home(world: &mut ConfigWorld) {
    let home = TempDir::new().expect("test HOME must be created");
    fs::create_dir(home.path().join(".orc")).expect("default state root must be created");
    fs::write(
        home.path().join(".orc/config.yaml"),
        "max-parallel-agents: 7\n",
    )
    .expect("default config must be written");
    let root = TempDir::new().expect("ORC_HOME must be created");
    fs::write(root.path().join("config.yaml"), "max-parallel-agents: 11\n")
        .expect("override config must be written");
    world.home = Some(home);
    world.root = Some(root);
}

#[given("HOME содержит лимит 7, а ORC_HOME пуст")]
fn empty_orc_home_uses_home(world: &mut ConfigWorld) {
    let home = TempDir::new().expect("test HOME must be created");
    fs::create_dir(home.path().join(".orc")).expect("default state root must be created");
    fs::write(
        home.path().join(".orc/config.yaml"),
        "max-parallel-agents: 7\n",
    )
    .expect("default config must be written");
    world.home = Some(home);
    world.empty_orc_home = true;
}

#[given("config.yaml с неизвестным полем")]
fn config_with_unknown_field(world: &mut ConfigWorld) {
    let root = TempDir::new().expect("test root must be created");
    fs::write(root.path().join("config.yaml"), "unknown: true\n")
        .expect("test config must be written");
    world.root = Some(root);
}

#[given(expr = "config.yaml с нарушением schema {string}")]
#[allow(clippy::needless_pass_by_value)]
fn config_with_schema_violation(world: &mut ConfigWorld, case: String) {
    let contents = match case.as_str() {
        "duplicate root field" => "max-parallel-agents: 7\nmax-parallel-agents: 9\n",
        "default-workflow is not string" => "default-workflow: 7\n",
        "default-agent has repeated hyphen" => "default-agent: bad--agent\n",
        "max-parallel-agents is zero" => "max-parallel-agents: 0\n",
        "agents is not mapping" => "agents: []\n",
        "AgentId contains dot" => {
            "agents:\n  bad.agent:\n    type: codex\n    model: valid\n    reasoning: high\n"
        }
        "Agent misses type" => "agents:\n  main:\n    model: valid\n    reasoning: high\n",
        "Agent model is not string" => {
            "agents:\n  main:\n    type: codex\n    model: 7\n    reasoning: high\n"
        }
        "Agent has command field" => {
            "agents:\n  main:\n    type: codex\n    model: valid\n    reasoning: high\n    command: tool\n"
        }
        "Agent has environment field" => {
            "agents:\n  main:\n    type: codex\n    model: valid\n    reasoning: high\n    environment: {}\n"
        }
        "Agent type is unknown" => {
            "agents:\n  main:\n    type: unknown\n    model: valid\n    reasoning: high\n"
        }
        "Agent model is invalid" => {
            "agents:\n  main:\n    type: codex\n    model: ''\n    reasoning: high\n"
        }
        "Agent reasoning is invalid" => {
            "agents:\n  main:\n    type: codex\n    model: valid\n    reasoning: ''\n"
        }
        other => panic!("unsupported config schema case: {other}"),
    };
    let root = TempDir::new().expect("test root must be created");
    fs::write(root.path().join("config.yaml"), contents).expect("test config must be written");
    world.root = Some(root);
}

#[given(expr = "config.yaml с лимитом {int}")]
fn config_with_limit(world: &mut ConfigWorld, limit: usize) {
    let root = TempDir::new().expect("test root must be created");
    let bytes = format!("max-parallel-agents: {limit}\n").into_bytes();
    fs::write(root.path().join("config.yaml"), &bytes).expect("test config must be written");
    world.initial_config = Some(bytes);
    world.root = Some(root);
}

#[given("config.yaml со всеми поддерживаемыми полями")]
fn config_with_all_fields(world: &mut ConfigWorld) {
    let root = TempDir::new().expect("test root must be created");
    let bytes = b"default-workflow: delivery\ndefault-agent: codex-main\nmax-parallel-agents: 7\nagents:\n  codex-main:\n    type: codex\n    model: gpt-5-codex\n    reasoning: high\n";
    fs::write(root.path().join("config.yaml"), bytes).expect("test config must be written");
    world.root = Some(root);
}

#[given("config.yaml с двумя именованными Agents")]
fn config_with_two_agents(world: &mut ConfigWorld) {
    let root = TempDir::new().expect("test root must be created");
    let bytes = b"default-workflow: delivery\nmax-parallel-agents: 7\nagents:\n  codex-main:\n    type: codex\n    model: gpt-5-codex\n    reasoning: high\n  claude-main:\n    type: claude\n    model: claude-opus\n    reasoning: high\n";
    fs::write(root.path().join("config.yaml"), bytes).expect("test config must be written");
    world.initial_config = Some(bytes.to_vec());
    world.root = Some(root);
}

#[given(expr = "config.yaml с невалидным Agent из-за {word}")]
#[allow(clippy::needless_pass_by_value)]
fn config_with_invalid_agent(world: &mut ConfigWorld, field: String) {
    let root = TempDir::new().expect("test root must be created");
    let agent = match field.as_str() {
        "type" => "type: unknown\n    model: valid\n    reasoning: high",
        "model" => "type: codex\n    model: ''\n    reasoning: high",
        "reasoning" => "type: codex\n    model: valid\n    reasoning: ''",
        _ => panic!("unsupported fixture field: {field}"),
    };
    let bytes = format!("agents:\n  codex-main:\n    {agent}\n").into_bytes();
    fs::write(root.path().join("config.yaml"), &bytes).expect("test config must be written");
    world.initial_config = Some(bytes);
    world.root = Some(root);
}

#[given(expr = "workflow {word} с невалидным содержимым")]
#[allow(clippy::needless_pass_by_value)]
fn invalid_workflow_file(world: &mut ConfigWorld, workflow_id: String) {
    let root = TempDir::new().expect("test root must be created");
    let workflow_root = root.path().join("workflow");
    fs::create_dir(&workflow_root).expect("workflow root must be created");
    fs::write(
        workflow_root.join(format!("{workflow_id}.yaml")),
        "this is not: [valid YAML",
    )
    .expect("workflow fixture must be written");
    world.root = Some(root);
}

#[given(expr = "существует workflow {word}")]
#[allow(clippy::needless_pass_by_value)]
fn workflow_exists(world: &mut ConfigWorld, workflow_id: String) {
    let root = world.root.as_ref().expect("scenario must define a root");
    let workflow_root = root.path().join("workflow");
    fs::create_dir_all(&workflow_root).expect("workflow root must be created");
    fs::write(
        workflow_root.join(format!("{workflow_id}.yaml")),
        "steps: []\n",
    )
    .expect("workflow fixture must be written");
}

#[when(expr = "я выполняю config get {word}")]
#[allow(clippy::needless_pass_by_value)]
fn run_config_get(world: &mut ConfigWorld, key: String) {
    run(world, &["config", "get", &key]);
}

#[when("я выполняю config list")]
fn run_config_list(world: &mut ConfigWorld) {
    run(world, &["config", "list"]);
}

#[when(expr = "я выполняю config set max-parallel-agents {word}")]
#[allow(clippy::needless_pass_by_value)]
fn run_config_set_limit(world: &mut ConfigWorld, value: String) {
    run(world, &["config", "set", "max-parallel-agents", &value]);
}

#[when(expr = "я выполняю config set default-workflow {word}")]
#[allow(clippy::needless_pass_by_value)]
fn run_config_set_default_workflow(world: &mut ConfigWorld, value: String) {
    run(world, &["config", "set", "default-workflow", &value]);
}

#[when(expr = "я выполняю config set default-agent {word}")]
#[allow(clippy::needless_pass_by_value)]
fn run_config_set_default_agent(world: &mut ConfigWorld, value: String) {
    run(world, &["config", "set", "default-agent", &value]);
}

#[when("я выполняю config set unknown 9")]
fn run_config_set_unknown(world: &mut ConfigWorld) {
    run(world, &["config", "set", "unknown", "9"]);
}

#[when("я выполняю config set max-parallel-agents без значения")]
fn run_config_set_without_value(world: &mut ConfigWorld) {
    run(world, &["config", "set", "max-parallel-agents"]);
}

#[when(expr = "конкурентно устанавливаются лимиты {int} и {int}")]
fn run_concurrent_sets(world: &mut ConfigWorld, first: usize, second: usize) {
    let root = world.root.as_ref().expect("scenario must define a root");
    let mut first_command = Command::new(env!("CARGO_BIN_EXE_orchestrator"));
    first_command
        .args(["config", "set", "max-parallel-agents", &first.to_string()])
        .env("ORC_HOME", root.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut second_command = Command::new(env!("CARGO_BIN_EXE_orchestrator"));
    second_command
        .args(["config", "set", "max-parallel-agents", &second.to_string()])
        .env("ORC_HOME", root.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let first_child = first_command
        .spawn()
        .expect("first orchestrator must start");
    let second_child = second_command
        .spawn()
        .expect("second orchestrator must start");
    world.concurrent_outputs = vec![
        first_child
            .wait_with_output()
            .expect("first orchestrator must finish"),
        second_child
            .wait_with_output()
            .expect("second orchestrator must finish"),
    ];
}

#[then(expr = "команда завершается с кодом {int}")]
fn exit_code(world: &mut ConfigWorld, expected: i32) {
    assert_eq!(world.output().status.code(), Some(expected));
}

#[then(expr = "stdout равен строке {string}")]
#[allow(clippy::needless_pass_by_value)]
fn stdout_equals(world: &mut ConfigWorld, expected: String) {
    assert_eq!(text(&world.output().stdout), format!("{expected}\n"));
}

#[then("stdout равен строкам default-workflow null, default-agent null и max-parallel-agents 5")]
fn stdout_contains_default_config(world: &mut ConfigWorld) {
    assert_eq!(
        text(&world.output().stdout),
        "default-workflow: null\ndefault-agent: null\nmax-parallel-agents: 5\n"
    );
}

#[then(
    "stdout равен строкам default-workflow delivery, default-agent codex-main и max-parallel-agents 7"
)]
fn stdout_contains_complete_config(world: &mut ConfigWorld) {
    assert_eq!(
        text(&world.output().stdout),
        "default-workflow: delivery\ndefault-agent: codex-main\nmax-parallel-agents: 7\n"
    );
}

#[then(expr = "stderr начинается с {string}")]
#[allow(clippy::needless_pass_by_value)]
fn stderr_starts_with(world: &mut ConfigWorld, prefix: String) {
    assert!(text(&world.output().stderr).starts_with(&prefix));
}

#[then(expr = "config get max-parallel-agents возвращает {string}")]
#[allow(clippy::needless_pass_by_value)]
fn config_get_limit_returns(world: &mut ConfigWorld, expected: String) {
    run(world, &["config", "get", "max-parallel-agents"]);
    assert_eq!(text(&world.output().stdout), format!("{expected}\n"));
}

#[then(expr = "config get default-workflow возвращает {string}")]
#[allow(clippy::needless_pass_by_value)]
fn config_get_default_workflow_returns(world: &mut ConfigWorld, expected: String) {
    run(world, &["config", "get", "default-workflow"]);
    assert_eq!(text(&world.output().stdout), format!("{expected}\n"));
}

#[then(expr = "config get default-agent возвращает {string}")]
#[allow(clippy::needless_pass_by_value)]
fn config_get_default_agent_returns(world: &mut ConfigWorld, expected: String) {
    run(world, &["config", "get", "default-agent"]);
    assert_eq!(text(&world.output().stdout), format!("{expected}\n"));
}

#[then("config.yaml остался побайтово неизменным")]
fn config_unchanged(world: &mut ConfigWorld) {
    let root = world.root.as_ref().expect("scenario must define a root");
    assert_eq!(
        fs::read(root.path().join("config.yaml")).expect("config must remain readable"),
        *world
            .initial_config
            .as_ref()
            .expect("scenario must capture initial config")
    );
}

#[then("остальные поля config.yaml сохранены")]
fn other_fields_preserved(world: &mut ConfigWorld) {
    let root = world.root.as_ref().expect("scenario must define a root");
    let contents = fs::read_to_string(root.path().join("config.yaml"))
        .expect("published config must be readable");
    assert!(contents.contains("default-workflow: delivery"));
    assert!(contents.contains("default-agent: codex-main"));
    assert!(contents.contains("codex-main:"));
    assert!(contents.contains("max-parallel-agents: 11"));
}

#[then(expr = "config.yaml сохраняет Agent и лимит при выборе workflow {word}")]
#[allow(clippy::needless_pass_by_value)]
fn agent_and_limit_preserved(world: &mut ConfigWorld, workflow_id: String) {
    let root = world.root.as_ref().expect("scenario must define a root");
    let contents = fs::read_to_string(root.path().join("config.yaml"))
        .expect("published config must be readable");
    assert!(contents.contains(&format!("default-workflow: {workflow_id}")));
    assert!(contents.contains("default-agent: codex-main"));
    assert!(contents.contains("max-parallel-agents: 7"));
    assert!(contents.contains("codex-main:"));
}

#[then("config.yaml сохраняет workflow, лимит и обоих Agents")]
fn config_fields_preserved_after_agent_selection(world: &mut ConfigWorld) {
    let root = world.root.as_ref().expect("scenario must define a root");
    let contents = fs::read_to_string(root.path().join("config.yaml"))
        .expect("published config must be readable");
    assert!(contents.contains("default-workflow: delivery"));
    assert!(contents.contains("default-agent: claude-main"));
    assert!(contents.contains("max-parallel-agents: 7"));
    assert!(contents.contains("codex-main:"));
    assert!(contents.contains("claude-main:"));
}

#[then("обе команды завершаются с кодом 0")]
fn both_commands_succeed(world: &mut ConfigWorld) {
    assert_eq!(world.concurrent_outputs.len(), 2);
    assert!(
        world
            .concurrent_outputs
            .iter()
            .all(|output| output.status.success())
    );
}

#[then("итоговый лимит равен 13 или 17")]
fn final_limit_is_one_candidate(world: &mut ConfigWorld) {
    run(world, &["config", "get", "max-parallel-agents"]);
    let stdout = text(&world.output().stdout);
    assert!(matches!(stdout.as_str(), "13\n" | "17\n"));
}

fn run(world: &mut ConfigWorld, arguments: &[&str]) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_orchestrator"));
    command.args(arguments);
    if let Some(home) = &world.home {
        command.env("HOME", home.path());
    }
    if world.relative_orc_home {
        command.env("ORC_HOME", "relative");
    } else if world.empty_orc_home {
        command.env("ORC_HOME", "");
    } else {
        let root = world.root.as_ref().expect("scenario must define a root");
        command.env("ORC_HOME", root.path());
    }
    world.output = Some(command.output().expect("orchestrator must run"));
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).expect("orchestrator output must be UTF-8")
}

impl ConfigWorld {
    fn output(&self) -> &Output {
        self.output
            .as_ref()
            .expect("scenario must execute a command")
    }
}

#[tokio::main]
async fn main() {
    ConfigWorld::run("features/config.feature").await;
}
