//! Cucumber-проверка публичного validate API и тех CLI-контрактов, которым необходим process driver.

use std::fs;
use std::process::Command;

use cucumber::{World, given, then, when};
use orchestrator::{
    AgentRegistry, BuiltinAgentRegistry, ProcessEnvironment, ValidateCommand,
    execute_validate_with_registry,
};
use tempfile::TempDir;

#[derive(Debug, Default, World)]
struct ValidationWorld {
    root: Option<TempDir>,
    outcome: Option<Observed>,
    second_outcome: Option<Observed>,
}

#[derive(Debug)]
struct Observed {
    exit_code: i32,
    stdout: String,
    stderr: String,
}

#[derive(Debug)]
struct FakeAgentRegistry;

impl AgentRegistry for FakeAgentRegistry {
    fn validate(&self, type_id: &str, model: &str, reasoning: &str) -> Result<(), String> {
        if type_id == "unsupported" {
            return Err("Agent type не поддерживает native resume".to_owned());
        }
        BuiltinAgentRegistry.validate(type_id, model, reasoning)
    }
}

#[given(expr = "подготовлен кандидат workflow {string}")]
#[allow(clippy::needless_pass_by_value)]
fn prepared_candidate(world: &mut ValidationWorld, candidate: String) {
    let root = TempDir::new().expect("test root must be created");
    prepare_candidate(root.path(), &candidate);
    world.root = Some(root);
}

#[given(expr = "подготовлен корень без workflow {word}")]
#[allow(clippy::needless_pass_by_value)]
fn root_without_workflow(world: &mut ValidationWorld, workflow_id: String) {
    drop(workflow_id);
    world.root = Some(TempDir::new().expect("test root must be created"));
}

#[when(expr = "workflow {word} проверяется через публичный API")]
#[allow(clippy::needless_pass_by_value)]
fn validate_through_api(world: &mut ValidationWorld, workflow_id: String) {
    let command =
        ValidateCommand::explicit(&workflow_id).expect("fixture WorkflowId must be valid");
    let environment = environment(world);
    world.outcome = Some(observe_api(&command, &environment));
}

#[when("workflow по умолчанию проверяется через публичный API")]
fn validate_default_through_api(world: &mut ValidationWorld) {
    let environment = environment(world);
    world.outcome = Some(observe_api(
        &ValidateCommand::configured_default(),
        &environment,
    ));
}

#[when(expr = "workflow {word} проверяется обеими формами публичного API")]
#[allow(clippy::needless_pass_by_value)]
fn validate_both_forms(world: &mut ValidationWorld, workflow_id: String) {
    let environment = environment(world);
    let explicit =
        ValidateCommand::explicit(&workflow_id).expect("fixture WorkflowId must be valid");
    world.outcome = Some(observe_api(&explicit, &environment));
    world.second_outcome = Some(observe_api(
        &ValidateCommand::configured_default(),
        &environment,
    ));
}

#[when(expr = "запускается orchestrator validate {word}")]
#[allow(clippy::needless_pass_by_value)]
fn validate_through_process(world: &mut ValidationWorld, workflow_id: String) {
    let root = world.root.as_ref().expect("scenario must define a root");
    let output = Command::new(env!("CARGO_BIN_EXE_orchestrator"))
        .args(["validate", &workflow_id])
        .env("ORC_HOME", root.path())
        .output()
        .expect("orchestrator must run");
    world.outcome = Some(observe_process(&output));
}

#[when("запускается orchestrator validate без аргумента")]
fn validate_default_through_process(world: &mut ValidationWorld) {
    let root = world.root.as_ref().expect("scenario must define a root");
    let output = Command::new(env!("CARGO_BIN_EXE_orchestrator"))
        .arg("validate")
        .env("ORC_HOME", root.path())
        .output()
        .expect("orchestrator must run");
    world.outcome = Some(observe_process(&output));
}

#[then(expr = "validation завершается с кодом {int}")]
fn validation_exit_code(world: &mut ValidationWorld, expected: i32) {
    assert_eq!(world.outcome().exit_code, expected);
}

#[then(expr = "результат validation равен {string}")]
#[allow(clippy::needless_pass_by_value)]
fn validation_output(world: &mut ValidationWorld, expected: String) {
    assert_eq!(world.outcome().stdout, expected);
}

#[then(expr = "diagnostics содержит {string}")]
#[allow(clippy::needless_pass_by_value)]
fn diagnostics_contains(world: &mut ValidationWorld, expected: String) {
    assert!(
        world.outcome().stderr.contains(&expected),
        "diagnostics {:?} do not contain {expected:?}",
        world.outcome().stderr
    );
}

#[then(expr = "diagnostics не содержит {string}")]
#[allow(clippy::needless_pass_by_value)]
fn diagnostics_does_not_contain(world: &mut ValidationWorld, unexpected: String) {
    assert!(!world.outcome().stderr.contains(&unexpected));
}

#[then(expr = "diagnostics начинается с {string}")]
#[allow(clippy::needless_pass_by_value)]
fn diagnostics_starts_with(world: &mut ValidationWorld, prefix: String) {
    assert!(world.outcome().stderr.starts_with(&prefix));
}

#[then("каталог run отсутствует")]
fn run_root_absent(world: &mut ValidationWorld) {
    let root = world.root.as_ref().expect("scenario must define a root");
    assert!(!root.path().join("run").exists());
}

#[then("обе формы validation возвращают одинаковый результат")]
fn both_validation_forms_match(world: &mut ValidationWorld) {
    let first = world.outcome();
    let second = world
        .second_outcome
        .as_ref()
        .expect("scenario must execute the second validation");
    assert_eq!(first.exit_code, second.exit_code);
    assert_eq!(first.stdout, second.stdout);
    assert_eq!(first.stderr, second.stderr);
}

fn observe_api(command: &ValidateCommand, environment: &ProcessEnvironment) -> Observed {
    match execute_validate_with_registry(command, environment, &FakeAgentRegistry) {
        Ok(stdout) => Observed {
            exit_code: 0,
            stdout,
            stderr: String::new(),
        },
        Err(error) => Observed {
            exit_code: i32::from(error.exit_code()),
            stdout: String::new(),
            stderr: error.to_string(),
        },
    }
}

fn observe_process(output: &std::process::Output) -> Observed {
    Observed {
        exit_code: output
            .status
            .code()
            .expect("orchestrator must exit normally"),
        stdout: text(&output.stdout).trim_end().to_owned(),
        stderr: text(&output.stderr).trim_end().to_owned(),
    }
}

fn environment(world: &ValidationWorld) -> ProcessEnvironment {
    let root = world.root.as_ref().expect("scenario must define a root");
    ProcessEnvironment {
        home: None,
        orc_home: Some(root.path().as_os_str().to_owned()),
    }
}

#[allow(clippy::too_many_lines)]
fn prepare_candidate(root: &std::path::Path, candidate: &str) {
    fs::create_dir_all(root.join("workflow")).expect("workflow root must be created");
    fs::create_dir_all(root.join("prompt")).expect("prompt root must be created");
    write_valid_config(root);

    match candidate {
        "линейный graph с обоими видами placeholders" => write_linear(root),
        "entry cycle без terminal Step" => write_workflow(root, entry_cycle()),
        "output без consumers" => write_workflow(root, output_without_consumers()),
        "выбранный workflow с невалидными невыбранными файлами" =>
        {
            write_linear(root);
            fs::write(root.join("workflow/broken-default.yaml"), "steps: [")
                .expect("unused workflow must be written");
            fs::write(root.join("prompt/unused.md"), [0xff])
                .expect("unused prompt must be written");
            let config = valid_config().replace(
                "default-agent: codex-main",
                "default-workflow: broken-default\ndefault-agent: codex-main",
            );
            fs::write(root.join("config.yaml"), config).expect("config must be written");
        }
        "default-workflow delivery" => {
            write_linear(root);
            set_default_workflow(root, "delivery");
        }
        "default-workflow missing" => {
            set_default_workflow(root, "missing");
        }
        "невалидный config с существующим workflow" => {
            write_linear(root);
            fs::write(root.join("config.yaml"), "unknown: true\n").expect("config must be written");
        }
        "default-workflow с недостижимым Step" => {
            write_workflow(
                root,
                "steps:\n  - id: plan\n    human: false\n    depends-on: []\n    outputs: []\n  - id: orphan\n    human: false\n    depends-on: []\n    outputs: []\n",
            );
            set_default_workflow(root, "delivery");
        }
        "пустой steps" => write_workflow(root, "steps: []\n"),
        "отсутствующее обязательное поле" => write_workflow(
            root,
            "steps:\n  - id: plan\n    depends-on: []\n    outputs: []\n",
        ),
        "неизвестное поле Step" => write_workflow(
            root,
            "steps:\n  - id: plan\n    human: false\n    depends-on: []\n    outputs: []\n    unknown: true\n",
        ),
        "повторяющийся StepId" => write_workflow(
            root,
            "steps:\n  - id: plan\n    human: false\n    depends-on: []\n    outputs: []\n  - id: plan\n    human: false\n    depends-on: [plan]\n    outputs: []\n",
        ),
        "повторяющийся depends-on" => write_workflow(
            root,
            "steps:\n  - id: plan\n    human: false\n    depends-on: []\n    outputs: []\n  - id: implement\n    human: false\n    depends-on: [plan, plan]\n    outputs: []\n",
        ),
        "повторяющийся output" => write_workflow(
            root,
            "steps:\n  - id: plan\n    human: false\n    depends-on: []\n    outputs: [spec, spec]\n",
        ),
        "невалидный StepId" => write_workflow(
            root,
            "steps:\n  - id: Bad-ID\n    human: false\n    depends-on: []\n    outputs: []\n",
        ),
        "синтаксически невалидный YAML" => {
            write_workflow(root, "steps: [\n");
        }
        "Agent не указан без default-agent" => {
            fs::write(root.join("config.yaml"), "agents: {}\n").expect("config must be written");
            write_workflow(root, single_step(None, None));
        }
        "явный Agent отсутствует" => {
            write_workflow(root, single_step(Some("missing"), None));
        }
        "невалидный AgentId" => {
            write_workflow(root, single_step(Some("Bad-ID"), None));
        }
        "Agent type без native resume" => {
            fs::write(
                root.join("config.yaml"),
                "default-agent: blocked\nagents:\n  blocked:\n    type: unsupported\n    model: model\n    reasoning: high\n",
            )
            .expect("config must be written");
            write_workflow(root, single_step(None, None));
        }
        "невалидный config" => {
            fs::write(root.join("config.yaml"), "unknown: true\n").expect("config must be written");
            write_workflow(root, single_step(None, None));
        }
        "отсутствующий Prompt" => {
            write_workflow(root, single_step(None, Some("missing")));
        }
        "placeholder первого Step" => {
            write_workflow(root, single_step(None, Some("plan")));
            fs::write(root.join("prompt/plan.md"), "{{path:plan:spec}}")
                .expect("prompt must be written");
        }
        "неизвестный вид placeholder" => {
            write_prompt_case(root, "{{unknown:plan:spec}}");
        }
        "пробел внутри placeholder" => {
            write_prompt_case(root, "{{path: plan:spec}}");
        }
        "незакрытый placeholder" => {
            write_prompt_case(root, "{{path:plan:spec");
        }
        "placeholder вне depends-on" => {
            write_prompt_case(root, "{{path:other:spec}}");
        }
        "placeholder неизвестного output" => {
            write_prompt_case(root, "{{path:plan:missing}}");
        }
        "Prompt не UTF-8" => {
            write_prompt_workflow(root);
            fs::write(root.join("prompt/implement.md"), [0xff]).expect("prompt must be written");
        }
        "Prompt не regular file" => {
            write_prompt_workflow(root);
            fs::create_dir(root.join("prompt/implement.md"))
                .expect("prompt directory must be created");
        }
        "неизвестный dependency" => write_workflow(
            root,
            "steps:\n  - id: plan\n    human: false\n    depends-on: []\n    outputs: []\n  - id: implement\n    human: false\n    depends-on: [missing]\n    outputs: []\n",
        ),
        "non-entry Step без dependencies" => write_workflow(
            root,
            "steps:\n  - id: plan\n    human: false\n    depends-on: []\n    outputs: []\n  - id: orphan\n    human: false\n    depends-on: []\n    outputs: []\n",
        ),
        "cycle без bootstrap-пути" => write_workflow(
            root,
            "steps:\n  - id: start\n    human: false\n    depends-on: []\n    outputs: []\n  - id: a\n    human: false\n    depends-on: [b]\n    outputs: []\n  - id: b\n    human: false\n    depends-on: [a]\n    outputs: []\n",
        ),
        "структурная и ссылочная ошибки" => write_workflow(
            root,
            "steps:\n  - id: plan\n    human: false\n    depends-on: []\n    outputs: [spec, spec]\n  - id: implement\n    human: false\n    depends-on: [missing]\n    outputs: []\n",
        ),
        other => panic!("unsupported candidate fixture: {other}"),
    }
}

fn write_valid_config(root: &std::path::Path) {
    fs::write(root.join("config.yaml"), valid_config()).expect("config must be written");
}

fn set_default_workflow(root: &std::path::Path, workflow_id: &str) {
    let config = valid_config().replacen(
        "default-agent: codex-main",
        &format!("default-workflow: {workflow_id}\ndefault-agent: codex-main"),
        1,
    );
    fs::write(root.join("config.yaml"), config).expect("config must be written");
}

fn valid_config() -> &'static str {
    "default-agent: codex-main\nmax-parallel-agents: 5\nagents:\n  codex-main:\n    type: codex\n    model: gpt-5-codex\n    reasoning: high\n  claude-main:\n    type: claude\n    model: claude-opus\n    reasoning: high\n"
}

fn write_linear(root: &std::path::Path) {
    write_workflow(
        root,
        "steps:\n  - id: plan\n    prompt: plan\n    human: false\n    depends-on: []\n    outputs: [spec, summary]\n  - id: implement\n    prompt: implement\n    human: false\n    depends-on: [plan]\n    outputs: [source]\n",
    );
    fs::write(root.join("prompt/plan.md"), "Составь план").expect("entry prompt must be written");
    fs::write(
        root.join("prompt/implement.md"),
        "{{path:plan:spec}} {{content:plan:summary}}",
    )
    .expect("dependent prompt must be written");
}

fn write_prompt_workflow(root: &std::path::Path) {
    write_workflow(
        root,
        "steps:\n  - id: plan\n    human: false\n    depends-on: []\n    outputs: [spec]\n  - id: implement\n    prompt: implement\n    human: false\n    depends-on: [plan]\n    outputs: []\n",
    );
}

fn write_prompt_case(root: &std::path::Path, prompt: &str) {
    write_prompt_workflow(root);
    fs::write(root.join("prompt/implement.md"), prompt).expect("prompt must be written");
}

fn write_workflow(root: &std::path::Path, contents: impl AsRef<[u8]>) {
    fs::write(root.join("workflow/delivery.yaml"), contents).expect("workflow must be written");
}

fn single_step(agent: Option<&str>, prompt: Option<&str>) -> String {
    let agent = agent.map_or(String::new(), |id| format!("    agent: {id}\n"));
    let prompt = prompt.map_or(String::new(), |id| format!("    prompt: {id}\n"));
    format!(
        "steps:\n  - id: plan\n{agent}{prompt}    human: false\n    depends-on: []\n    outputs: []\n"
    )
}

fn entry_cycle() -> &'static str {
    "steps:\n  - id: a\n    human: false\n    depends-on: [b]\n    outputs: []\n  - id: b\n    human: false\n    depends-on: [a]\n    outputs: []\n"
}

fn output_without_consumers() -> &'static str {
    "steps:\n  - id: plan\n    human: false\n    depends-on: []\n    outputs: [unused]\n"
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).expect("orchestrator output must be UTF-8")
}

impl ValidationWorld {
    fn outcome(&self) -> &Observed {
        self.outcome
            .as_ref()
            .expect("scenario must execute validation")
    }
}

#[tokio::main]
async fn main() {
    ValidationWorld::run("features/workflow_validation.feature").await;
    ValidationWorld::run("features/validate_cli.feature").await;
}
