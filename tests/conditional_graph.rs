//! Acceptance условных выражений и повторений через публичный lifecycle API с управляемым in-memory Agent.

use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use cucumber::{World, given, then, when};
use orchestrator::{
    AgentExit, AgentRegistry, AgentRunRequest, AttemptControl, LifecycleCommand, LifecycleReporter,
    LifecycleSignals, ProcessEnvironment, RunId, TerminalMode, execute_lifecycle,
};
use tempfile::TempDir;

#[derive(Debug, Default, World)]
struct ConditionalWorld {
    root: Option<TempDir>,
    registry: Fake,
    run_id: Option<String>,
    code: u8,
    error: String,
    lines: Vec<String>,
    representations: Option<Representations>,
}

#[derive(Debug, Default)]
struct Fake {
    plans: Mutex<HashMap<String, VecDeque<Option<Vec<String>>>>>,
    calls: Mutex<Vec<(String, Vec<String>)>>,
    accepted: Mutex<Vec<bool>>,
}

#[derive(Debug)]
struct Representations {
    source_text: String,
    source_json: String,
    plan_text: String,
    plan_json: String,
}

impl AgentRegistry for Fake {
    fn validate(&self, type_id: &str, _model: &str, _reasoning: &str) -> Result<(), String> {
        if type_id == "codex" {
            Ok(())
        } else {
            Err("unknown fake".to_owned())
        }
    }

    fn run(
        &self,
        request: &AgentRunRequest<'_>,
        control: &mut dyn AttemptControl,
    ) -> Result<AgentExit, String> {
        self.calls.lock().unwrap().push((
            request.step_id.to_owned(),
            request
                .inputs
                .iter()
                .map(|input| format!("{}:{}", input.step_id, input.input_id))
                .collect(),
        ));
        let plan = self
            .plans
            .lock()
            .unwrap()
            .get_mut(request.step_id)
            .and_then(VecDeque::pop_front);
        let Some(Some(outputs)) = plan else {
            return Ok(AgentExit::Returned(0));
        };
        let staging = tempfile::tempdir().map_err(|error| error.to_string())?;
        let mut artifacts = Vec::new();
        for output in outputs {
            let path = staging.path().join(&output);
            fs::write(&path, request.prompt).map_err(|error| error.to_string())?;
            artifacts.push((output, path));
        }
        let result = control.complete(&artifacts);
        self.accepted.lock().unwrap().push(result.is_ok());
        Ok(AgentExit::Returned(0))
    }
}

#[derive(Default)]
struct Reporter(Vec<String>);
impl LifecycleReporter for Reporter {
    fn line(&mut self, line: &str) -> std::io::Result<()> {
        self.0.push(line.to_owned());
        Ok(())
    }
}

fn step(id: &str, dependencies: &str, outputs: &str) -> String {
    format!("- id: {id}\n  human: false\n  depends-on: {dependencies}\n  outputs: {outputs}\n")
}

fn prepare(world: &mut ConditionalWorld, steps: &[String]) {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("workflow")).unwrap();
    fs::create_dir(root.path().join("prompt")).unwrap();
    fs::write(root.path().join("config.yaml"), "default-agent: main\nmax-parallel-agents: 1\nagents:\n  main:\n    type: codex\n    model: model\n    reasoning: high\n").unwrap();
    fs::write(
        root.path().join("workflow/delivery.yaml"),
        format!("steps:\n{}", steps.concat()),
    )
    .unwrap();
    world.root = Some(root);
}

fn plan(world: &ConditionalWorld, id: &str, outputs: &[&str]) {
    world.registry.plans.lock().unwrap().insert(
        id.to_owned(),
        outputs.iter().map(|outputs| Some(split(outputs))).collect(),
    );
}

fn split(value: &str) -> Vec<String> {
    value
        .split(',')
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect()
}

#[given(expr = "outputs первого Step равны {string}")]
#[allow(
    clippy::needless_pass_by_value,
    reason = "Cucumber разбирает аргументы через FromStr"
)]
fn first_outputs(world: &mut ConditionalWorld, outputs: String) {
    prepare(world, &[step("source", "[]", &outputs)]);
}

#[when(expr = "Agent предлагает artifacts {string}")]
#[allow(
    clippy::needless_pass_by_value,
    reason = "Cucumber разбирает аргументы через FromStr"
)]
fn propose(world: &mut ConditionalWorld, artifacts: String) {
    plan(world, "source", &[&artifacts]);
    execute(world, false);
}

#[then(expr = "completion принят {string}")]
#[allow(
    clippy::needless_pass_by_value,
    reason = "Cucumber разбирает аргументы через FromStr"
)]
fn accepted(world: &mut ConditionalWorld, expected: String) {
    assert_eq!(
        *world.registry.accepted.lock().unwrap(),
        [expected == "yes"],
        "{}",
        world.error
    );
}

fn directory(world: &ConditionalWorld) -> PathBuf {
    world
        .root
        .as_ref()
        .unwrap()
        .path()
        .join("run")
        .join(world.run_id.as_ref().unwrap())
}

#[then(expr = "опубликованы artifacts {string}")]
#[allow(
    clippy::needless_pass_by_value,
    reason = "Cucumber разбирает аргументы через FromStr"
)]
fn artifacts(world: &mut ConditionalWorld, expected: String) {
    let mut actual = fs::read_dir(directory(world))
        .unwrap()
        .map(Result::unwrap)
        .filter_map(|entry| {
            entry
                .file_name()
                .to_str()?
                .strip_prefix("0.source.")?
                .strip_suffix(".artifact")
                .map(str::to_owned)
        })
        .collect::<Vec<_>>();
    actual.sort();
    let mut expected = split(&expected);
    expected.sort();
    assert_eq!(actual, expected);
}

#[given("подготовлен условный diamond с выбором done")]
fn conditional_diamond(world: &mut ConditionalWorld) {
    prepare(
        world,
        &[
            step("review", "[]", "[{one-of: [fix, done]}]"),
            step("fix", "[{step: review, output: fix}]", "[result]"),
            step("finish", "[{step: review, output: done}]", "[result]"),
            step("collect", "[{one-of: [fix, finish]}]", "[]"),
        ],
    );
    plan(world, "review", &["done"]);
    plan(world, "finish", &["result"]);
    plan(world, "collect", &[""]);
}

#[given("подготовлен workflow с двумя qualified ссылками на source")]
fn shared_source(world: &mut ConditionalWorld) {
    prepare(
        world,
        &[
            step(
                "source",
                "[]",
                "[report, {one-of: [{all: [fix, patch]}, done]}]",
            ),
            step(
                "target",
                "[{step: source, output: fix}, {step: source, output: report}]",
                "[]",
            ),
        ],
    );
    plan(world, "source", &["report,fix,patch"]);
    plan(world, "target", &[""]);
}

#[given("подготовлен diamond с one-of на join")]
fn choice_diamond(world: &mut ConditionalWorld) {
    prepare(
        world,
        &[
            step("root", "[]", "[]"),
            step("left", "[root]", "[]"),
            step("right", "[root]", "[]"),
            step("join", "[{one-of: [left, right]}]", "[]"),
        ],
    );
    for id in ["root", "left", "right", "join"] {
        plan(world, id, &[""]);
    }
}

#[given("подготовлен one-of с равными максимальными source numbers")]
fn tied_choice(world: &mut ConditionalWorld) {
    prepare(
        world,
        &[
            step("root", "[]", "[]"),
            step("left", "[root]", "[]"),
            step("right", "[root]", "[]"),
            step(
                "join",
                "[{one-of: [{all: [root, right]}, {all: [left, right]}]}]",
                "[]",
            ),
        ],
    );
    for id in ["root", "left", "right", "join"] {
        plan(world, id, &[""]);
    }
}

#[given("подготовлен условный цикл с постоянным контекстом")]
fn constant_cycle(world: &mut ConditionalWorld) {
    prepare(
        world,
        &[
            step("context", "[]", "[context]"),
            step(
                "a",
                "[context, {one-of: [{step: context, output: context}, {step: check, output: update}]}]",
                "[done]",
            ),
            step("left", "[a]", "[done]"),
            step("right", "[a]", "[done]"),
            step("check", "[left, right]", "[{one-of: [update, done]}]"),
            step("finish", "[{step: check, output: done}]", "[]"),
        ],
    );
    plan(world, "context", &["context"]);
    plan(world, "a", &["done", "done"]);
    plan(world, "left", &["done", "done"]);
    plan(world, "right", &["done", "done"]);
    plan(world, "check", &["update", "done"]);
    plan(world, "finish", &[""]);
}

#[given("подготовлен цикл с условной внутренней ветвью")]
fn branched_cycle(world: &mut ConditionalWorld) {
    prepare(
        world,
        &[
            step(
                "a",
                "[{step: check, output: update}]",
                "[{one-of: [left, right]}]",
            ),
            step("left", "[{step: a, output: left}]", "[done]"),
            step("right", "[{step: a, output: right}]", "[done]"),
            step(
                "check",
                "[{one-of: [left, right]}]",
                "[{one-of: [update, done]}]",
            ),
            step("finish", "[{step: check, output: done}]", "[]"),
        ],
    );
    plan(world, "a", &["right", "right"]);
    plan(world, "right", &["done", "done"]);
    plan(world, "check", &["update", "done"]);
    plan(world, "finish", &[""]);
}

#[given("подготовлен цикл с короткой и длинной ветвями")]
fn uneven_cycle(world: &mut ConditionalWorld) {
    prepare(
        world,
        &[
            step("a", "[{step: check, output: update}]", "[]"),
            step("left", "[a]", "[]"),
            step(
                "check",
                "[{one-of: [left, tail]}]",
                "[{one-of: [update, done]}]",
            ),
            step("right", "[a]", "[]"),
            step("tail", "[right]", "[]"),
            step("finish", "[{step: check, output: done}]", "[]"),
        ],
    );
    for id in ["a", "left", "right", "tail", "finish"] {
        plan(world, id, &[""]);
    }
    plan(world, "check", &["done"]);
}

fn environment(world: &ConditionalWorld) -> ProcessEnvironment {
    let root = world.root.as_ref().unwrap();
    ProcessEnvironment {
        orc_home: Some(root.path().to_owned().into_os_string()),
        current_dir: Some(root.path().to_owned()),
        ..ProcessEnvironment::default()
    }
}

#[when("проверяется source и materialized представление условий")]
fn representations(world: &mut ConditionalWorld) {
    use orchestrator::{InspectionFormat, execute_workflow_plan, execute_workflow_show};
    let environment = environment(world);
    world.representations = Some(Representations {
        source_text: execute_workflow_show("delivery", &environment, InspectionFormat::Text)
            .unwrap(),
        source_json: execute_workflow_show("delivery", &environment, InspectionFormat::Json)
            .unwrap(),
        plan_text: execute_workflow_plan("delivery", &environment, InspectionFormat::Text).unwrap(),
        plan_json: execute_workflow_plan("delivery", &environment, InspectionFormat::Json).unwrap(),
    });
}

#[then(
    "source show text содержит `outputs=report,one-of(all(fix,patch),done)` и `depends-on=source:fix,source:report`"
)]
fn source_show_text_preserves_expressions(world: &mut ConditionalWorld) {
    let representations = world
        .representations
        .as_ref()
        .expect("representations must be captured");
    assert_expression_text(&representations.source_text);
}

#[then(
    "source show JSON сохраняет nested one-of/all outputs report, fix, patch, done и qualified dependencies source:fix и source:report в mappings step/output"
)]
fn source_show_json_preserves_expressions(world: &mut ConditionalWorld) {
    let representations = world
        .representations
        .as_ref()
        .expect("representations must be captured");
    assert_expression_json(&representations.source_json);
}

#[then(
    "materialized plan text содержит `outputs=report,one-of(all(fix,patch),done)` и `depends-on=source:fix,source:report`"
)]
fn materialized_plan_text_preserves_expressions(world: &mut ConditionalWorld) {
    let representations = world
        .representations
        .as_ref()
        .expect("representations must be captured");
    assert_expression_text(&representations.plan_text);
}

#[then(
    "materialized plan JSON сохраняет nested one-of/all outputs report, fix, patch, done и qualified dependencies source:fix и source:report в mappings step/output"
)]
fn materialized_plan_json_preserves_expressions(world: &mut ConditionalWorld) {
    let representations = world
        .representations
        .as_ref()
        .expect("representations must be captured");
    assert_expression_json(&representations.plan_json);
}

#[then("run ещё не создан")]
fn no_run(world: &mut ConditionalWorld) {
    assert!(!world.root.as_ref().unwrap().path().join("run").exists());
}

fn assert_expression_text(text: &str) {
    assert!(
        text.contains("outputs=report,one-of(all(fix,patch),done)"),
        "{text}"
    );
    assert!(
        text.contains("depends-on=source:fix,source:report"),
        "{text}"
    );
}

fn assert_expression_json(json: &str) {
    let json: serde_json::Value = serde_json::from_str(json).expect("representation must be JSON");
    assert_eq!(
        json["steps"][0]["outputs"],
        serde_json::json!(["report", {"one-of": [{"all": ["fix", "patch"]}, "done"]}])
    );
    assert_eq!(
        json["steps"][1]["depends_on"],
        serde_json::json!([{"step": "source", "output": "fix"}, {"step": "source", "output": "report"}])
    );
}

#[then("inspection видит done и не видит fix")]
fn conditional_inspection(world: &mut ConditionalWorld) {
    use orchestrator::{inspect_run, open_run_artifact};
    let environment = environment(world);
    let run_id = RunId::parse(world.run_id.as_ref().unwrap()).unwrap();
    let inspected = inspect_run(run_id, &environment).unwrap();
    assert_eq!(inspected.artifacts().len(), 2);
    assert!(open_run_artifact(run_id, 0, "done", &environment).is_ok());
    assert_eq!(
        open_run_artifact(run_id, 0, "fix", &environment)
            .unwrap_err()
            .exit_code(),
        4
    );
}

#[when("условный workflow выполняется до завершения")]
fn run(world: &mut ConditionalWorld) {
    execute(world, false);
}

fn execute(world: &mut ConditionalWorld, resume: bool) {
    let root = world.root.as_ref().unwrap();
    let environment = ProcessEnvironment {
        home: None,
        orc_home: Some(root.path().to_owned().into_os_string()),
        current_dir: Some(root.path().to_owned()),
        path: None,
    };
    let command = if resume {
        LifecycleCommand::Resume(RunId::parse(world.run_id.as_ref().unwrap()).unwrap())
    } else {
        LifecycleCommand::start_explicit("delivery").unwrap()
    };
    let mut reporter = Reporter::default();
    let result = execute_lifecycle(
        &command,
        &environment,
        TerminalMode::Unavailable,
        &LifecycleSignals::default(),
        &world.registry,
        &mut reporter,
    );
    if let Some(id) = reporter.0.iter().find_map(|line| {
        line.strip_prefix("Run ")
            .filter(|id| id.bytes().all(|byte| byte.is_ascii_digit()))
    }) {
        world.run_id = Some(id.to_owned());
    }
    match result {
        Ok(()) => world.code = 0,
        Err(error) => {
            world.code = error.exit_code();
            world.error = error.to_string();
        }
    }
    world.lines = reporter.0;
}

#[when("выполнение прерывается на повторном a и продолжается через resume")]
fn resume_cycle(world: &mut ConditionalWorld) {
    world
        .registry
        .plans
        .lock()
        .unwrap()
        .get_mut("a")
        .unwrap()
        .insert(1, None);
    execute(world, false);
    assert_eq!(world.code, 1, "{}", world.error);
    let before = fs::read(directory(world).join("5.a.attempt.yaml")).unwrap();
    execute(world, true);
    let after: serde_yaml::Value =
        serde_yaml::from_slice(&fs::read(directory(world).join("5.a.attempt.yaml")).unwrap())
            .unwrap();
    let before: serde_yaml::Value = serde_yaml::from_slice(&before).unwrap();
    assert_eq!(before["input"], after["input"]);
}

#[when("в незавершённый run добавлено преждевременное решение")]
fn premature_decision(world: &mut ConditionalWorld) {
    world
        .registry
        .plans
        .lock()
        .unwrap()
        .insert("tail".to_owned(), VecDeque::from([None]));
    execute(world, false);
    assert_eq!(world.code, 1);
    let directory = directory(world);
    fs::write(
        directory.join("4.check.attempt.yaml"),
        "input: [1]\nevents: [{type: completed}]\n",
    )
    .unwrap();
    fs::write(directory.join("4.check.update.artifact"), "update").unwrap();
    world.registry.calls.lock().unwrap().clear();
    execute(world, true);
}

#[then("resume отклоняет run кодом 3 без запуска Agent")]
fn invalid_resume(world: &mut ConditionalWorld) {
    assert_eq!(world.code, 3, "{}", world.error);
    assert!(world.registry.calls.lock().unwrap().is_empty());
    assert!(
        world.error.contains("до завершения выбранных ветвей"),
        "{}",
        world.error
    );
}

#[then(expr = "выполнены Steps {string}")]
#[allow(
    clippy::needless_pass_by_value,
    reason = "Cucumber разбирает аргументы через FromStr"
)]
fn calls(world: &mut ConditionalWorld, expected: String) {
    assert_eq!(
        world
            .registry
            .calls
            .lock()
            .unwrap()
            .iter()
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>(),
        split(&expected),
        "{}",
        world.error
    );
}

#[then(expr = "результат run равен {string}")]
#[allow(
    clippy::needless_pass_by_value,
    reason = "Cucumber разбирает аргументы через FromStr"
)]
fn outcome(world: &mut ConditionalWorld, expected: String) {
    assert_eq!(expected, "completed");
    assert_eq!(world.code, 0, "{}", world.error);
    assert!(
        world.lines.iter().any(|line| line.contains("completed")),
        "{:?}",
        world.lines
    );
}

fn input_groups(world: &ConditionalWorld, id: &str) -> Vec<String> {
    let mut records = fs::read_dir(directory(world))
        .unwrap()
        .map(Result::unwrap)
        .filter_map(|entry| {
            let name = entry.file_name().to_str()?.to_owned();
            let number = name
                .strip_suffix(&format!(".{id}.attempt.yaml"))?
                .parse::<u64>()
                .ok()?;
            let record: serde_yaml::Value =
                serde_yaml::from_slice(&fs::read(entry.path()).unwrap()).unwrap();
            Some((
                number,
                record["input"]
                    .as_sequence()
                    .unwrap()
                    .iter()
                    .map(|number| number.as_u64().unwrap().to_string())
                    .collect::<Vec<_>>()
                    .join(","),
            ))
        })
        .collect::<Vec<_>>();
    records.sort_by_key(|(number, _)| *number);
    records.into_iter().map(|(_, input)| input).collect()
}

#[then(expr = "attempt {string} получает inputs {string}")]
#[allow(
    clippy::needless_pass_by_value,
    reason = "Cucumber разбирает аргументы через FromStr"
)]
fn input(world: &mut ConditionalWorld, step_id: String, expected: String) {
    assert_eq!(input_groups(world, &step_id), [expected], "{}", world.error);
}

#[then(expr = "attempts {string} получают input groups {string}")]
#[allow(
    clippy::needless_pass_by_value,
    reason = "Cucumber разбирает аргументы через FromStr"
)]
fn groups(world: &mut ConditionalWorld, step_id: String, expected: String) {
    assert_eq!(
        input_groups(world, &step_id),
        expected.split(';').map(str::to_owned).collect::<Vec<_>>()
    );
}

#[then(expr = "target получает artifacts {string}")]
#[allow(
    clippy::needless_pass_by_value,
    reason = "Cucumber разбирает аргументы через FromStr"
)]
fn target_artifacts(world: &mut ConditionalWorld, expected: String) {
    assert_eq!(
        world
            .registry
            .calls
            .lock()
            .unwrap()
            .iter()
            .find(|(step, _)| step == "target")
            .unwrap()
            .1,
        split(&expected)
    );
}

#[given(expr = "подготовлен невалидный условный workflow {string}")]
#[allow(
    clippy::needless_pass_by_value,
    reason = "Cucumber разбирает аргументы через FromStr"
)]
fn invalid(world: &mut ConditionalWorld, case: String) {
    let steps = match case.as_str() {
        "пустая группа" => vec![step("a", "[]", "[{one-of: []}]")],
        "повторный output" => vec![step("a", "[]", "[x, {all: [x]}]")],
        "неизвестный output" => vec![
            step("a", "[]", "[x]"),
            step("b", "[{step: a, output: missing}]", "[]"),
        ],
        "несовместимые outputs source" => vec![
            step("a", "[]", "[{one-of: [x, y]}]"),
            step("b", "[{step: a, output: x}, {step: a, output: y}]", "[]"),
        ],
        "несовместимые ветви" => vec![
            step("root", "[]", "[{one-of: [x, y]}]"),
            step("left", "[{step: root, output: x}]", "[]"),
            step("right", "[{step: root, output: y}]", "[]"),
            step("join", "[left, right]", "[]"),
        ],
        "негарантированный placeholder" => vec![
            step("a", "[]", "[{one-of: [x, y]}]"),
            step("b", "[a]", "[]").replace("  human:", "  prompt: conditional\n  human:"),
        ],
        "ранний возврат" => vec![
            step("a", "[b]", "[]"),
            step("b", "[a]", "[]"),
            step("c", "[a]", "[]"),
        ],
        "два независимых цикла" => vec![
            step("root", "[]", "[]"),
            step("a", "[{one-of: [root, b]}]", "[]"),
            step("b", "[a]", "[]"),
            step("c", "[{one-of: [root, d]}]", "[]"),
            step("d", "[c]", "[]"),
        ],
        "вложенный цикл" => vec![
            step("a", "[c]", "[]"),
            step("b", "[{one-of: [a, c]}]", "[]"),
            step("c", "[b]", "[]"),
        ],
        "поле fresh" => vec![
            step("a", "[]", "[x]"),
            step("b", "[{step: a, output: x, fresh: true}]", "[]"),
        ],
        _ => panic!("unknown fixture"),
    };
    prepare(world, &steps);
    fs::write(
        world
            .root
            .as_ref()
            .unwrap()
            .path()
            .join("prompt/conditional.md"),
        "{{path:a:x}}",
    )
    .unwrap();
}

#[then("условный lifecycle возвращает код 3 без run")]
fn invalid_result(world: &mut ConditionalWorld) {
    assert_eq!(world.code, 3, "{}", world.error);
    assert!(world.run_id.is_none());
    assert!(world.registry.calls.lock().unwrap().is_empty());
}

#[tokio::main]
async fn main() {
    ConditionalWorld::run("features/conditional_graph.feature").await;
}
