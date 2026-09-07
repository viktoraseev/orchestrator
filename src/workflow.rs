//! Materialization и validation workflow graph; модуль не планирует attempts и не записывает состояние run.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::domain::{Dependencies, Outputs};

use crate::agent::{AgentRegistry, BuiltinAgentRegistry};
use crate::config::{CommandError, ProcessEnvironment, RawConfig, read_config, resolve_state_root};
use crate::domain::{Agent, ProcessStep, Step, SymbolicId, Workflow};
use crate::run::InspectionFormat;

pub use crate::domain::WorkflowId;

/// Запрос полной проверки workflow после разбора CLI.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ValidateCommand {
    /// Проверить workflow с явно указанным ID.
    Explicit(WorkflowId),
    /// Проверить workflow, указанный полем `default-workflow` config.
    ConfiguredDefault,
}

impl ValidateCommand {
    /// Создаёт запрос проверки явно выбранного workflow.
    ///
    /// # Errors
    ///
    /// Возвращает [`CommandError::Syntax`], если `WorkflowId` не соответствует kebab-case.
    pub fn explicit(value: &str) -> Result<Self, CommandError> {
        WorkflowId::parse(value)
            .map(Self::Explicit)
            .map_err(|context| CommandError::Syntax {
                context: format!("validate: {context}"),
            })
    }

    /// Создаёт запрос проверки workflow из `default-workflow` config.
    #[must_use]
    pub const fn configured_default() -> Self {
        Self::ConfiguredDefault
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawWorkflow {
    #[serde(default)]
    pub(crate) parameters: BTreeMap<String, String>,
    pub(crate) steps: Vec<RawStep>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub(crate) struct RawStep {
    pub(crate) id: String,
    pub(crate) agent: Option<String>,
    pub(crate) prompt: Option<String>,
    pub(crate) process: Option<RawProcess>,
    pub(crate) human: bool,
    pub(crate) depends_on: Dependencies,
    pub(crate) outputs: Outputs,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawProcess {
    pub(crate) executable: String,
    pub(crate) args: Vec<String>,
    pub(crate) cwd: Option<String>,
    pub(crate) stdout: Option<String>,
}

/// Один результат полного bulk preflight source workflow.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct WorkflowValidation {
    workflow: String,
    valid: bool,
    diagnostics: Vec<String>,
}

impl WorkflowValidation {
    /// Возвращает проверенный `WorkflowId`.
    #[must_use]
    pub fn workflow(&self) -> &str {
        &self.workflow
    }

    /// Возвращает результат полного preflight.
    #[must_use]
    pub const fn is_valid(&self) -> bool {
        self.valid
    }

    /// Возвращает diagnostics невалидного кандидата.
    #[must_use]
    pub fn diagnostics(&self) -> &[String] {
        &self.diagnostics
    }
}

/// Полный deterministic report `validate --all`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct WorkflowValidationReport {
    workflows: Vec<WorkflowValidation>,
}

impl WorkflowValidationReport {
    /// Возвращает результаты в порядке `WorkflowId`.
    #[must_use]
    pub fn workflows(&self) -> &[WorkflowValidation] {
        &self.workflows
    }

    /// Возвращает `true`, когда каждый найденный workflow валиден.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.workflows.iter().all(WorkflowValidation::is_valid)
    }
}

/// Dependency edge source workflow.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct WorkflowGraphEdge {
    from: String,
    to: String,
}

impl WorkflowGraphEdge {
    /// Возвращает dependency `StepId`.
    #[must_use]
    pub fn from(&self) -> &str {
        &self.from
    }

    /// Возвращает dependent `StepId`.
    #[must_use]
    pub fn to(&self) -> &str {
        &self.to
    }
}

/// Typed validated source dependency graph.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct WorkflowGraph {
    workflow: String,
    path: String,
    bootstrap: String,
    nodes: Vec<String>,
    edges: Vec<WorkflowGraphEdge>,
}

impl WorkflowGraph {
    /// Возвращает `WorkflowId`.
    #[must_use]
    pub fn workflow(&self) -> &str {
        &self.workflow
    }

    /// Возвращает абсолютный source path.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Возвращает Step initial activation.
    #[must_use]
    pub fn bootstrap(&self) -> &str {
        &self.bootstrap
    }

    /// Возвращает `StepIds` в source order.
    #[must_use]
    pub fn nodes(&self) -> &[String] {
        &self.nodes
    }

    /// Возвращает dependency edges в source order.
    #[must_use]
    pub fn edges(&self) -> &[WorkflowGraphEdge] {
        &self.edges
    }
}

/// Effective Agent materialized Step.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct WorkflowPlanAgent {
    #[serde(rename = "type")]
    agent_type: String,
    model: String,
    reasoning: String,
}

/// Materialized Process executor одного Step.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct WorkflowPlanProcess {
    executable: String,
    args: Vec<String>,
    cwd: String,
    stdout: Option<String>,
}

impl WorkflowPlanProcess {
    /// Возвращает абсолютный executable path.
    #[must_use]
    pub fn executable(&self) -> &str {
        &self.executable
    }

    /// Возвращает argv templates в source order.
    #[must_use]
    pub fn args(&self) -> &[String] {
        &self.args
    }

    /// Возвращает абсолютный working directory.
    #[must_use]
    pub fn cwd(&self) -> &str {
        &self.cwd
    }

    /// Возвращает output, в который направляется stdout.
    #[must_use]
    pub fn stdout(&self) -> Option<&str> {
        self.stdout.as_deref()
    }
}

impl WorkflowPlanAgent {
    /// Возвращает built-in `AgentTypeId`.
    #[must_use]
    pub fn agent_type(&self) -> &str {
        &self.agent_type
    }

    /// Возвращает effective model.
    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Возвращает effective reasoning.
    #[must_use]
    pub fn reasoning(&self) -> &str {
        &self.reasoning
    }
}

/// Один Step materialized execution plan.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct WorkflowPlanStep {
    id: String,
    agent: Option<WorkflowPlanAgent>,
    prompt: Option<String>,
    human: bool,
    process: Option<WorkflowPlanProcess>,
    depends_on: Dependencies,
    outputs: Outputs,
}

impl WorkflowPlanStep {
    /// Возвращает `StepId`.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Возвращает materialized Agent только для Agent Step.
    #[must_use]
    pub const fn agent(&self) -> Option<&WorkflowPlanAgent> {
        self.agent.as_ref()
    }

    /// Возвращает точный materialized prompt content.
    #[must_use]
    pub fn prompt(&self) -> Option<&str> {
        self.prompt.as_deref()
    }

    /// Возвращает human flag.
    #[must_use]
    pub const fn is_human(&self) -> bool {
        self.human
    }

    /// Возвращает materialized Process executor для Process Step.
    #[must_use]
    pub const fn process(&self) -> Option<&WorkflowPlanProcess> {
        self.process.as_ref()
    }

    /// Возвращает dependencies в source order.
    #[must_use]
    pub fn depends_on(&self) -> &[String] {
        self.depends_on.sources()
    }

    /// Возвращает outputs в source order.
    #[must_use]
    pub fn outputs(&self) -> &[String] {
        self.outputs.ids()
    }
}

/// Typed полностью materialized кандидат до создания run.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct WorkflowPlan {
    workflow_id: String,
    max_parallel_agents: usize,
    parameters: Vec<String>,
    steps: Vec<WorkflowPlanStep>,
}

impl WorkflowPlan {
    /// Возвращает materialized `WorkflowId`.
    #[must_use]
    pub fn workflow_id(&self) -> &str {
        &self.workflow_id
    }

    /// Возвращает effective global parallel limit.
    #[must_use]
    pub const fn max_parallel_agents(&self) -> usize {
        self.max_parallel_agents
    }

    /// Возвращает объявленные `ParameterIds` в детерминированном порядке.
    #[must_use]
    pub fn parameters(&self) -> &[String] {
        &self.parameters
    }

    /// Возвращает materialized Steps в source order.
    #[must_use]
    pub fn steps(&self) -> &[WorkflowPlanStep] {
        &self.steps
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PlaceholderKind {
    Path,
    Content,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Placeholder {
    kind: PlaceholderKind,
    step_id: SymbolicId,
    input_id: SymbolicId,
}

/// Полностью materialize и проверяет выбранный workflow без записи состояния run.
///
/// См. Rule «Явно выбранный workflow полностью проверяется в памяти» в `features/workflow_validation.feature`.
///
/// # Errors
///
/// Возвращает [`CommandError::NotFound`] для отсутствующего template, [`CommandError::Invalid`] для невалидного config, workflow, Agent или prompt и [`CommandError::Runtime`] для ошибок файловой системы.
pub fn execute_validate(
    command: &ValidateCommand,
    environment: &ProcessEnvironment,
) -> Result<String, CommandError> {
    execute_validate_with_registry(command, environment, &BuiltinAgentRegistry)
}

/// Выполняет ту же полную validation с переданным registry Agent type.
///
/// Этот entrypoint позволяет Cucumber заменить только process-зависимость `AgentType`.
///
/// # Errors
///
/// Возвращает те же категории ошибок, что [`execute_validate`].
pub fn execute_validate_with_registry(
    command: &ValidateCommand,
    environment: &ProcessEnvironment,
    registry: &dyn AgentRegistry,
) -> Result<String, CommandError> {
    let root = resolve_state_root(environment, "validate")?;
    let config = read_config(&root, "validate", registry)?;
    let configured_workflow_id;
    let workflow_id = match command {
        ValidateCommand::Explicit(workflow_id) => workflow_id,
        ValidateCommand::ConfiguredDefault => {
            let Some(value) = &config.raw.default_workflow else {
                return Err(CommandError::Syntax {
                    context: "validate: передайте WorkflowId или настройте default-workflow"
                        .to_owned(),
                });
            };
            configured_workflow_id = WorkflowId::from_validated(value.clone());
            &configured_workflow_id
        }
    };
    let workflow = materialize(
        &root,
        workflow_id,
        &config.raw,
        config.max_parallel_agents().get(),
        environment,
    )?;
    validate_graph(&workflow)?;
    Ok(format!("workflow {}: valid", workflow.id.as_str()))
}

/// Проверяет полный source workflow catalog через built-in Agent registry.
///
/// # Errors
///
/// Возвращает ошибку catalog discovery либо runtime error отдельного preflight; validation errors кандидатов сохраняются в report.
pub fn validate_all_workflows(
    environment: &ProcessEnvironment,
) -> Result<WorkflowValidationReport, CommandError> {
    validate_all_workflows_with_registry(environment, &BuiltinAgentRegistry)
}

/// Проверяет полный source workflow catalog через переданный Agent registry.
///
/// # Errors
///
/// Возвращает те же ошибки, что [`validate_all_workflows`].
pub fn validate_all_workflows_with_registry(
    environment: &ProcessEnvironment,
    registry: &dyn AgentRegistry,
) -> Result<WorkflowValidationReport, CommandError> {
    let entries = crate::catalog::list_workflows(environment)
        .map_err(|error| recontextualize(error, "workflow list", "validate --all"))?;
    let mut workflows = Vec::with_capacity(entries.len());
    for entry in entries {
        let command = ValidateCommand::explicit(entry.workflow())?;
        match execute_validate_with_registry(&command, environment, registry) {
            Ok(_) => workflows.push(WorkflowValidation {
                workflow: entry.workflow().to_owned(),
                valid: true,
                diagnostics: Vec::new(),
            }),
            Err(
                CommandError::Invalid { context }
                | CommandError::NotFound { context }
                | CommandError::Syntax { context },
            ) => workflows.push(WorkflowValidation {
                workflow: entry.workflow().to_owned(),
                valid: false,
                diagnostics: vec![context],
            }),
            Err(error) => return Err(recontextualize(error, "validate", "validate --all")),
        }
    }
    Ok(WorkflowValidationReport { workflows })
}

/// Рендерит полный bulk validation report.
///
/// # Errors
///
/// Возвращает ошибки [`validate_all_workflows`] либо serialization.
pub fn execute_validate_all(
    environment: &ProcessEnvironment,
    format: InspectionFormat,
) -> Result<(String, WorkflowValidationReport), CommandError> {
    let report = validate_all_workflows(environment)?;
    let output = render_validation_report(&report, format)?;
    Ok((output, report))
}

/// Строит validated dependency graph выбранного source workflow без config и prompts.
///
/// # Errors
///
/// Возвращает `2` для невалидного ID, `4` для неизвестного workflow и `3` для невалидной source topology.
pub fn build_workflow_graph(
    workflow_id: &str,
    environment: &ProcessEnvironment,
) -> Result<WorkflowGraph, CommandError> {
    let source = crate::catalog::show_workflow(workflow_id, environment)
        .map_err(|error| recontextualize(error, "workflow show", "workflow graph"))?;
    let nodes = source
        .steps()
        .iter()
        .map(|step| step.id().to_owned())
        .collect::<Vec<_>>();
    let known = nodes.iter().map(String::as_str).collect::<HashSet<_>>();
    let mut edges = Vec::new();
    for step in source.steps() {
        for dependency in step.depends_on() {
            if !known.contains(dependency.as_str()) {
                return Err(invalid_graph(
                    source.workflow(),
                    step.id(),
                    &format!("depends-on ссылается на неизвестный Step '{dependency}'"),
                ));
            }
            edges.push(WorkflowGraphEdge {
                from: dependency.clone(),
                to: step.id().to_owned(),
            });
        }
    }
    validate_source_reachability(source.workflow(), source.steps())?;
    let bootstrap = source
        .steps()
        .first()
        .map(|step| step.id().to_owned())
        .ok_or_else(|| CommandError::Invalid {
            context: format!(
                "workflow graph: workflow '{}': steps должен быть непустым",
                source.workflow()
            ),
        })?;
    Ok(WorkflowGraph {
        workflow: source.workflow().to_owned(),
        path: source.path().to_owned(),
        bootstrap,
        nodes,
        edges,
    })
}

/// Рендерит validated source graph в text или JSON.
///
/// # Errors
///
/// Возвращает ошибки [`build_workflow_graph`] либо serialization.
pub fn execute_workflow_graph(
    workflow_id: &str,
    environment: &ProcessEnvironment,
    format: InspectionFormat,
) -> Result<String, CommandError> {
    let graph = build_workflow_graph(workflow_id, environment)?;
    render_workflow_graph(&graph, format)
}

/// Полностью materialize'ит выбранный workflow без создания run.
///
/// # Errors
///
/// Возвращает полные preflight errors config, workflow, Agent, prompt и graph validation.
pub fn build_workflow_plan(
    workflow_id: &str,
    environment: &ProcessEnvironment,
) -> Result<WorkflowPlan, CommandError> {
    build_workflow_plan_with_registry(workflow_id, environment, &BuiltinAgentRegistry)
}

/// Полностью materialize'ит выбранный workflow через переданный Agent registry.
///
/// # Errors
///
/// Возвращает те же ошибки, что [`build_workflow_plan`].
pub fn build_workflow_plan_with_registry(
    workflow_id: &str,
    environment: &ProcessEnvironment,
    registry: &dyn AgentRegistry,
) -> Result<WorkflowPlan, CommandError> {
    let selection = ValidateCommand::explicit(workflow_id)
        .map_err(|error| recontextualize(error, "validate", "workflow plan"))?;
    materialize_for_lifecycle(&selection, environment, registry, "workflow plan")
        .map(WorkflowPlan::from)
}

/// Рендерит materialized execution plan в text или JSON.
///
/// # Errors
///
/// Возвращает ошибки [`build_workflow_plan`] либо serialization.
pub fn execute_workflow_plan(
    workflow_id: &str,
    environment: &ProcessEnvironment,
    format: InspectionFormat,
) -> Result<String, CommandError> {
    let plan = build_workflow_plan(workflow_id, environment)?;
    render_workflow_plan(&plan, format)
}

impl From<Workflow> for WorkflowPlan {
    fn from(workflow: Workflow) -> Self {
        Self {
            workflow_id: workflow.id.as_str().to_owned(),
            max_parallel_agents: workflow.max_parallel_agents,
            parameters: workflow
                .parameters
                .into_iter()
                .map(SymbolicId::into_string)
                .collect(),
            steps: workflow
                .steps
                .into_iter()
                .map(|step| WorkflowPlanStep {
                    id: step.id.into_string(),
                    agent: step.agent.map(|agent| WorkflowPlanAgent {
                        agent_type: agent.r#type,
                        model: agent.model,
                        reasoning: agent.reasoning,
                    }),
                    prompt: step.prompt,
                    human: step.human,
                    process: step.process.map(|process| WorkflowPlanProcess {
                        executable: process.executable.to_string_lossy().into_owned(),
                        args: process.args,
                        cwd: process.cwd.to_string_lossy().into_owned(),
                        stdout: process.stdout.map(SymbolicId::into_string),
                    }),
                    depends_on: step.depends_on,
                    outputs: step.outputs,
                })
                .collect(),
        }
    }
}

fn validate_source_reachability(
    workflow: &str,
    steps: &[crate::catalog::SourceWorkflowStep],
) -> Result<(), CommandError> {
    crate::domain::Graph::validate(
        steps
            .iter()
            .map(|step| crate::domain::GraphStep {
                id: step.id(),
                depends_on: step.dependencies(),
                outputs: step.output_expression(),
            })
            .collect(),
    )
    .map(|_| ())
    .map_err(|message| invalid_graph(workflow, "graph", &message))
}

fn invalid_graph(workflow: &str, step: &str, message: &str) -> CommandError {
    CommandError::Invalid {
        context: format!("workflow graph: workflow '{workflow}': step '{step}': {message}"),
    }
}

fn render_validation_report(
    report: &WorkflowValidationReport,
    format: InspectionFormat,
) -> Result<String, CommandError> {
    match format {
        InspectionFormat::Text => {
            let mut output = String::new();
            for workflow in &report.workflows {
                if workflow.valid {
                    writeln!(output, "workflow {}: valid", workflow.workflow)
                        .map_err(workflow_formatting_error)?;
                } else {
                    for diagnostic in &workflow.diagnostics {
                        writeln!(
                            output,
                            "workflow {}: invalid: {diagnostic}",
                            workflow.workflow
                        )
                        .map_err(workflow_formatting_error)?;
                    }
                }
            }
            output.pop();
            Ok(output)
        }
        InspectionFormat::Json => {
            serde_json::to_string_pretty(report).map_err(|source| CommandError::Runtime {
                context: "validate --all: не удалось сериализовать JSON".to_owned(),
                source: std::io::Error::other(source),
            })
        }
    }
}

fn render_workflow_graph(
    graph: &WorkflowGraph,
    format: InspectionFormat,
) -> Result<String, CommandError> {
    match format {
        InspectionFormat::Text => {
            let mut output = String::new();
            writeln!(output, "workflow {}: path={}", graph.workflow, graph.path)
                .map_err(workflow_formatting_error)?;
            writeln!(output, "bootstrap: {}", graph.bootstrap)
                .map_err(workflow_formatting_error)?;
            for edge in &graph.edges {
                writeln!(output, "{} -> {}", edge.from, edge.to)
                    .map_err(workflow_formatting_error)?;
            }
            output.pop();
            Ok(output)
        }
        InspectionFormat::Json => {
            serde_json::to_string_pretty(graph).map_err(|source| CommandError::Runtime {
                context: "workflow graph: не удалось сериализовать JSON".to_owned(),
                source: std::io::Error::other(source),
            })
        }
    }
}

fn render_workflow_plan(
    plan: &WorkflowPlan,
    format: InspectionFormat,
) -> Result<String, CommandError> {
    match format {
        InspectionFormat::Text => {
            let mut output = String::new();
            writeln!(
                output,
                "workflow {}: max-parallel-agents={}",
                plan.workflow_id, plan.max_parallel_agents
            )
            .map_err(workflow_formatting_error)?;
            for step in &plan.steps {
                if let Some(agent) = &step.agent {
                    let prompt_bytes = step
                        .prompt
                        .as_ref()
                        .map_or_else(|| "-".to_owned(), |prompt| prompt.len().to_string());
                    writeln!(
                        output,
                        "step {}: type={} model={} reasoning={} prompt-bytes={} human={} depends-on={} outputs={}",
                        step.id,
                        agent.agent_type,
                        agent.model,
                        agent.reasoning,
                        prompt_bytes,
                        step.human,
                        step.depends_on.text(),
                        step.outputs.text()
                    )
                    .map_err(workflow_formatting_error)?;
                } else if let Some(process) = &step.process {
                    writeln!(
                        output,
                        "step {}: executor=process executable={} args={} cwd={} stdout={} depends-on={} outputs={}",
                        step.id,
                        process.executable,
                        process.args.len(),
                        process.cwd,
                        process.stdout.as_deref().unwrap_or("-"),
                        step.depends_on.text(),
                        step.outputs.text()
                    )
                    .map_err(workflow_formatting_error)?;
                }
            }
            output.pop();
            Ok(output)
        }
        InspectionFormat::Json => {
            serde_json::to_string_pretty(plan).map_err(|source| CommandError::Runtime {
                context: "workflow plan: не удалось сериализовать JSON".to_owned(),
                source: std::io::Error::other(source),
            })
        }
    }
}

fn workflow_formatting_error(source: std::fmt::Error) -> CommandError {
    CommandError::Runtime {
        context: "workflow tool: не удалось сформировать text output".to_owned(),
        source: std::io::Error::other(source),
    }
}

fn recontextualize(error: CommandError, from: &str, to: &str) -> CommandError {
    let replace = |value: String| value.replacen(&format!("{from}:"), &format!("{to}:"), 1);
    match error {
        CommandError::Syntax { context } => CommandError::Syntax {
            context: replace(context),
        },
        CommandError::Invalid { context } => CommandError::Invalid {
            context: replace(context),
        },
        CommandError::NotFound { context } => CommandError::NotFound {
            context: replace(context),
        },
        CommandError::Busy { context } => CommandError::Busy {
            context: replace(context),
        },
        CommandError::Interrupted { context, exit_code } => CommandError::Interrupted {
            context: replace(context),
            exit_code,
        },
        CommandError::Runtime { context, source } => CommandError::Runtime {
            context: replace(context),
            source,
        },
    }
}

fn materialize(
    root: &Path,
    workflow_id: &WorkflowId,
    config: &RawConfig,
    max_parallel_agents: usize,
    environment: &ProcessEnvironment,
) -> Result<Workflow, CommandError> {
    let path = workflow_path(root, workflow_id);
    require_regular_workflow(&path, workflow_id)?;
    let bytes = fs::read(&path).map_err(|source| CommandError::Runtime {
        context: format!("validate: не удалось прочитать workflow {}", path.display()),
        source,
    })?;
    let raw: RawWorkflow =
        serde_yaml::from_slice(&bytes).map_err(|source| CommandError::Invalid {
            context: format!(
                "validate: невалидный workflow '{}': {source}",
                workflow_id.as_str()
            ),
        })?;
    if raw.steps.is_empty() {
        return Err(invalid_workflow(workflow_id, "steps должен быть непустым"));
    }

    let parameters = parse_parameters(workflow_id, raw.parameters)?;

    let mut step_ids = HashSet::with_capacity(raw.steps.len());
    let mut steps = Vec::with_capacity(raw.steps.len());
    let mut prompt_cache = HashMap::<SymbolicId, String>::new();
    for (index, raw_step) in raw.steps.into_iter().enumerate() {
        let step_id = parse_step_id(workflow_id, index, &raw_step.id)?;
        if !step_ids.insert(step_id.clone()) {
            return Err(invalid_step(
                workflow_id,
                step_id.as_str(),
                "StepId повторяется",
            ));
        }
        let depends_on = raw_step.depends_on;
        let outputs = raw_step.outputs;
        let (agent, prompt, human, process) = if let Some(raw_process) = raw_step.process {
            if raw_step.agent.is_some() {
                return Err(invalid_step(
                    workflow_id,
                    step_id.as_str(),
                    "Process Step не допускает поле agent",
                ));
            }
            if raw_step.prompt.is_some() {
                return Err(invalid_step(
                    workflow_id,
                    step_id.as_str(),
                    "Process Step не допускает поле prompt",
                ));
            }
            if raw_step.human {
                return Err(invalid_step(
                    workflow_id,
                    step_id.as_str(),
                    "Process Step не может быть human",
                ));
            }
            let process = resolve_process(workflow_id, step_id.as_str(), raw_process, environment)?;
            (None, None, false, Some(process))
        } else {
            let agent = resolve_agent(
                workflow_id,
                step_id.as_str(),
                raw_step.agent.as_deref(),
                config,
            )?;
            let prompt = load_prompt(
                root,
                workflow_id,
                step_id.as_str(),
                raw_step.prompt,
                &mut prompt_cache,
            )?;
            (Some(agent), prompt, raw_step.human, None)
        };
        steps.push(Step {
            id: step_id,
            agent,
            prompt,
            human,
            process,
            depends_on,
            outputs,
        });
    }
    Ok(Workflow {
        id: workflow_id.clone(),
        max_parallel_agents,
        parameters,
        steps,
    })
}

fn parse_parameters(
    workflow_id: &WorkflowId,
    raw: BTreeMap<String, String>,
) -> Result<Vec<SymbolicId>, CommandError> {
    let mut parameters = Vec::with_capacity(raw.len());
    for (id, parameter_type) in raw {
        let id = SymbolicId::parse("ParameterId", &id)
            .map_err(|context| invalid_workflow(workflow_id, &context))?;
        if parameter_type != "string" {
            return Err(invalid_workflow(
                workflow_id,
                &format!(
                    "Parameter '{}' имеет неподдерживаемый type '{parameter_type}'",
                    id.as_str()
                ),
            ));
        }
        parameters.push(id);
    }
    Ok(parameters)
}

fn resolve_process(
    workflow_id: &WorkflowId,
    step_id: &str,
    raw: RawProcess,
    environment: &ProcessEnvironment,
) -> Result<ProcessStep, CommandError> {
    if raw.executable.is_empty()
        || raw.executable.contains('\0')
        || raw.executable.contains("{{")
        || raw.executable.contains("}}")
    {
        return Err(invalid_step(
            workflow_id,
            step_id,
            "Process executable должен быть непустой строкой без NUL и placeholders",
        ));
    }
    if raw.args.iter().any(|argument| argument.contains('\0')) {
        return Err(invalid_step(
            workflow_id,
            step_id,
            "Process args не могут содержать NUL",
        ));
    }
    let current_dir = environment.current_dir.clone().map_or_else(
        || {
            env::current_dir().map_err(|source| CommandError::Runtime {
                context: format!(
                    "validate: workflow '{}': не удалось определить current working directory",
                    workflow_id.as_str()
                ),
                source,
            })
        },
        Ok,
    )?;
    let cwd = raw.cwd.map_or_else(
        || Ok(current_dir.clone()),
        |value| {
            if value.is_empty()
                || value.contains('\0')
                || value.contains("{{")
                || value.contains("}}")
            {
                return Err(invalid_step(
                    workflow_id,
                    step_id,
                    "Process cwd должен быть непустой строкой без NUL и placeholders",
                ));
            }
            let path = PathBuf::from(value);
            Ok(if path.is_absolute() {
                path
            } else {
                current_dir.join(path)
            })
        },
    )?;
    let cwd = fs::canonicalize(&cwd).map_err(|source| CommandError::Runtime {
        context: format!(
            "validate: workflow '{}': Step '{step_id}': Process cwd '{}' недоступен",
            workflow_id.as_str(),
            cwd.display()
        ),
        source,
    })?;
    if !cwd.is_dir() {
        return Err(invalid_step(
            workflow_id,
            step_id,
            &format!("Process cwd '{}' не является directory", cwd.display()),
        ));
    }
    let executable = resolve_executable(
        workflow_id,
        step_id,
        &raw.executable,
        &cwd,
        environment.path.as_deref(),
    )?;
    let stdout = raw
        .stdout
        .map(|id| {
            SymbolicId::parse("InputId", &id)
                .map_err(|context| invalid_step(workflow_id, step_id, &context))
        })
        .transpose()?;
    Ok(ProcessStep {
        executable,
        args: raw.args,
        cwd,
        stdout,
    })
}

fn resolve_executable(
    workflow_id: &WorkflowId,
    step_id: &str,
    value: &str,
    cwd: &Path,
    configured_path: Option<&std::ffi::OsStr>,
) -> Result<PathBuf, CommandError> {
    let candidate = PathBuf::from(value);
    let candidate = if candidate.is_absolute() {
        Some(candidate)
    } else if value.contains('/') {
        Some(cwd.join(candidate))
    } else {
        let path = configured_path
            .map(std::ffi::OsStr::to_owned)
            .or_else(|| env::var_os("PATH"));
        path.and_then(|path| {
            env::split_paths(&path)
                .map(|directory| directory.join(value))
                .find(|path| is_executable_file(path))
        })
    };
    let Some(candidate) = candidate else {
        return Err(invalid_step(
            workflow_id,
            step_id,
            &format!("Process executable '{value}' не найден в PATH"),
        ));
    };
    if !is_executable_file(&candidate) {
        return Err(invalid_step(
            workflow_id,
            step_id,
            &format!(
                "Process executable '{}' не является executable regular file",
                candidate.display()
            ),
        ));
    }
    fs::canonicalize(&candidate).map_err(|source| CommandError::Runtime {
        context: format!(
            "validate: workflow '{}': Step '{step_id}': не удалось разрешить Process executable '{}'",
            workflow_id.as_str(),
            candidate.display()
        ),
        source,
    })
}

fn is_executable_file(path: &Path) -> bool {
    fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

pub(crate) fn materialize_for_lifecycle(
    command: &ValidateCommand,
    environment: &ProcessEnvironment,
    registry: &dyn AgentRegistry,
    context: &str,
) -> Result<Workflow, CommandError> {
    let root = resolve_state_root(environment, context)?;
    let config = read_config(&root, context, registry)?;
    let configured_workflow_id;
    let workflow_id = match command {
        ValidateCommand::Explicit(workflow_id) => workflow_id,
        ValidateCommand::ConfiguredDefault => {
            let Some(value) = &config.raw.default_workflow else {
                return Err(CommandError::Syntax {
                    context: format!(
                        "{context}: передайте WorkflowId или настройте default-workflow"
                    ),
                });
            };
            configured_workflow_id = WorkflowId::from_validated(value.clone());
            &configured_workflow_id
        }
    };
    materialize(
        &root,
        workflow_id,
        &config.raw,
        config.max_parallel_agents().get(),
        environment,
    )
    .and_then(|workflow| {
        validate_graph(&workflow)?;
        Ok(workflow)
    })
    .map_err(|error| replace_context(error, context))
}

fn replace_context(error: CommandError, context: &str) -> CommandError {
    recontextualize(error, "validate", context)
}

fn workflow_path(root: &Path, workflow_id: &WorkflowId) -> PathBuf {
    root.join("workflow")
        .join(format!("{}.yaml", workflow_id.as_str()))
}

fn require_regular_workflow(path: &Path, workflow_id: &WorkflowId) -> Result<(), CommandError> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(()),
        Ok(_) => Err(invalid_workflow(
            workflow_id,
            "template не является regular file",
        )),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            Err(CommandError::NotFound {
                context: format!(
                    "validate: workflow '{}' не существует",
                    workflow_id.as_str()
                ),
            })
        }
        Err(source) => Err(CommandError::Runtime {
            context: format!("validate: не удалось проверить workflow {}", path.display()),
            source,
        }),
    }
}

fn parse_step_id(
    workflow_id: &WorkflowId,
    index: usize,
    value: &str,
) -> Result<SymbolicId, CommandError> {
    SymbolicId::parse("StepId", value).map_err(|context| {
        invalid_workflow(
            workflow_id,
            &format!("step {}: {context}", index.saturating_add(1)),
        )
    })
}

fn resolve_agent(
    workflow_id: &WorkflowId,
    step_id: &str,
    explicit_agent: Option<&str>,
    config: &RawConfig,
) -> Result<Agent, CommandError> {
    let agent_id = explicit_agent
        .or(config.default_agent.as_deref())
        .ok_or_else(|| {
            invalid_step(
                workflow_id,
                step_id,
                "Agent не указан и default-agent не задан",
            )
        })?;
    SymbolicId::parse("AgentId", agent_id)
        .map_err(|context| invalid_step(workflow_id, step_id, &context))?;
    config.agents.get(agent_id).cloned().ok_or_else(|| {
        invalid_step(
            workflow_id,
            step_id,
            &format!("Agent '{agent_id}' отсутствует в config"),
        )
    })
}

fn load_prompt(
    root: &Path,
    workflow_id: &WorkflowId,
    step_id: &str,
    prompt_id: Option<String>,
    cache: &mut HashMap<SymbolicId, String>,
) -> Result<Option<String>, CommandError> {
    let Some(prompt_id) = prompt_id else {
        return Ok(None);
    };
    let prompt_id = SymbolicId::parse("PromptId", &prompt_id)
        .map_err(|context| invalid_step(workflow_id, step_id, &context))?;
    if let Some(prompt) = cache.get(&prompt_id) {
        return Ok(Some(prompt.clone()));
    }
    let path = root
        .join("prompt")
        .join(format!("{}.md", prompt_id.as_str()));
    let metadata = fs::metadata(&path).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            invalid_step(
                workflow_id,
                step_id,
                &format!("Prompt '{}' не существует", prompt_id.as_str()),
            )
        } else {
            CommandError::Runtime {
                context: format!("validate: не удалось проверить prompt {}", path.display()),
                source,
            }
        }
    })?;
    if !metadata.is_file() {
        return Err(invalid_step(
            workflow_id,
            step_id,
            &format!("Prompt '{}' не является regular file", prompt_id.as_str()),
        ));
    }
    let prompt = fs::read_to_string(&path).map_err(|source| {
        if source.kind() == std::io::ErrorKind::InvalidData {
            invalid_step(
                workflow_id,
                step_id,
                &format!("Prompt '{}' не является UTF-8", prompt_id.as_str()),
            )
        } else {
            CommandError::Runtime {
                context: format!("validate: не удалось прочитать prompt {}", path.display()),
                source,
            }
        }
    })?;
    cache.insert(prompt_id, prompt.clone());
    Ok(Some(prompt))
}

fn validate_graph(workflow: &Workflow) -> Result<(), CommandError> {
    let by_id: HashMap<&str, &Step> = workflow
        .steps
        .iter()
        .map(|step| (step.id.as_str(), step))
        .collect();
    for (index, step) in workflow.steps.iter().enumerate() {
        for dependency in &step.depends_on {
            if !by_id.contains_key(dependency.as_str()) {
                return Err(invalid_step(
                    &workflow.id,
                    step.id.as_str(),
                    &format!(
                        "depends-on ссылается на неизвестный Step '{}'",
                        dependency.as_str()
                    ),
                ));
            }
        }
        if let Some(prompt) = &step.prompt {
            let placeholders = parse_placeholders(&workflow.id, step.id.as_str(), prompt)?;
            if index == 0 && !placeholders.is_empty() {
                return Err(invalid_step(
                    &workflow.id,
                    step.id.as_str(),
                    "prompt первого Step содержит placeholder",
                ));
            }
            validate_placeholders(workflow, step, &by_id, &placeholders)?;
        }
        if let Some(process) = &step.process {
            validate_process(workflow, step, &by_id, process)?;
        }
    }
    validate_reachability(workflow)
}

fn validate_process(
    workflow: &Workflow,
    step: &Step,
    by_id: &HashMap<&str, &Step>,
    process: &ProcessStep,
) -> Result<(), CommandError> {
    if let Some(stdout) = &process.stdout
        && !step.outputs.iter().any(|id| id == stdout.as_str())
    {
        return Err(invalid_step(
            &workflow.id,
            step.id.as_str(),
            &format!(
                "Process stdout ссылается на неизвестный output '{}'",
                stdout.as_str()
            ),
        ));
    }
    for argument in &process.args {
        validate_process_argument(workflow, step, by_id, argument)?;
    }
    Ok(())
}

fn validate_process_argument(
    workflow: &Workflow,
    step: &Step,
    by_id: &HashMap<&str, &Step>,
    argument: &str,
) -> Result<(), CommandError> {
    if !argument.contains("{{") && !argument.contains("}}") {
        return Ok(());
    }
    let Some(body) = argument
        .strip_prefix("{{")
        .and_then(|value| value.strip_suffix("}}"))
    else {
        return Err(invalid_step(
            &workflow.id,
            step.id.as_str(),
            "Process placeholder должен занимать весь argv element",
        ));
    };
    if body.contains("{{")
        || body.contains("}}")
        || body.bytes().any(|byte| byte.is_ascii_whitespace())
    {
        return Err(invalid_step(
            &workflow.id,
            step.id.as_str(),
            &format!("невалидный Process placeholder '{{{{{body}}}}}'"),
        ));
    }
    let parts = body.split(':').collect::<Vec<_>>();
    match parts.as_slice() {
        ["param", parameter_id] => {
            let parameter_id = SymbolicId::parse("ParameterId", parameter_id)
                .map_err(|context| invalid_step(&workflow.id, step.id.as_str(), &context))?;
            if !workflow.parameters.contains(&parameter_id) {
                return Err(invalid_step(
                    &workflow.id,
                    step.id.as_str(),
                    &format!(
                        "Process placeholder ссылается на неизвестный parameter '{}'",
                        parameter_id.as_str()
                    ),
                ));
            }
        }
        ["path", source_step, input_id] => {
            let placeholder = Placeholder {
                kind: PlaceholderKind::Path,
                step_id: SymbolicId::parse("StepId", source_step)
                    .map_err(|context| invalid_step(&workflow.id, step.id.as_str(), &context))?,
                input_id: SymbolicId::parse("InputId", input_id)
                    .map_err(|context| invalid_step(&workflow.id, step.id.as_str(), &context))?,
            };
            validate_placeholders(workflow, step, by_id, &[placeholder])?;
        }
        ["output", input_id] => {
            let input_id = SymbolicId::parse("InputId", input_id)
                .map_err(|context| invalid_step(&workflow.id, step.id.as_str(), &context))?;
            if !step.outputs.iter().any(|id| id == input_id.as_str()) {
                return Err(invalid_step(
                    &workflow.id,
                    step.id.as_str(),
                    &format!(
                        "Process placeholder ссылается на неизвестный output '{}'",
                        input_id.as_str()
                    ),
                ));
            }
        }
        ["content", ..] => {
            return Err(invalid_step(
                &workflow.id,
                step.id.as_str(),
                "content placeholder запрещён в Process args",
            ));
        }
        _ => {
            return Err(invalid_step(
                &workflow.id,
                step.id.as_str(),
                &format!("невалидный Process placeholder '{{{{{body}}}}}'"),
            ));
        }
    }
    Ok(())
}

fn parse_placeholders(
    workflow_id: &WorkflowId,
    step_id: &str,
    prompt: &str,
) -> Result<Vec<Placeholder>, CommandError> {
    let mut remaining = prompt;
    let mut placeholders = Vec::new();
    while let Some(start) = remaining.find("{{") {
        remaining = &remaining[start + 2..];
        let Some(end) = remaining.find("}}") else {
            return Err(invalid_step(
                workflow_id,
                step_id,
                "prompt содержит незакрытый placeholder",
            ));
        };
        let body = &remaining[..end];
        let parts: Vec<&str> = body.split(':').collect();
        if parts.len() != 3 || body.bytes().any(|byte| byte.is_ascii_whitespace()) {
            return Err(invalid_step(
                workflow_id,
                step_id,
                &format!("невалидный placeholder '{{{{{body}}}}}'"),
            ));
        }
        let kind = match parts[0] {
            "path" => PlaceholderKind::Path,
            "content" => PlaceholderKind::Content,
            _ => {
                return Err(invalid_step(
                    workflow_id,
                    step_id,
                    &format!("неизвестный вид placeholder '{}'", parts[0]),
                ));
            }
        };
        let source_step = SymbolicId::parse("StepId", parts[1])
            .map_err(|context| invalid_step(workflow_id, step_id, &context))?;
        let input_id = SymbolicId::parse("InputId", parts[2])
            .map_err(|context| invalid_step(workflow_id, step_id, &context))?;
        placeholders.push(Placeholder {
            kind,
            step_id: source_step,
            input_id,
        });
        remaining = &remaining[end + 2..];
    }
    Ok(placeholders)
}

fn validate_placeholders(
    workflow: &Workflow,
    step: &Step,
    by_id: &HashMap<&str, &Step>,
    placeholders: &[Placeholder],
) -> Result<(), CommandError> {
    for placeholder in placeholders {
        if !step
            .depends_on
            .iter()
            .any(|id| id == placeholder.step_id.as_str())
        {
            return Err(invalid_step(
                &workflow.id,
                step.id.as_str(),
                &format!(
                    "placeholder ссылается на Step '{}' вне depends-on",
                    placeholder.step_id.as_str()
                ),
            ));
        }
        let source = by_id.get(placeholder.step_id.as_str()).ok_or_else(|| {
            invalid_step(
                &workflow.id,
                step.id.as_str(),
                &format!(
                    "placeholder ссылается на неизвестный Step '{}'",
                    placeholder.step_id.as_str()
                ),
            )
        })?;
        if !source
            .outputs
            .iter()
            .any(|id| id == placeholder.input_id.as_str())
        {
            return Err(invalid_step(
                &workflow.id,
                step.id.as_str(),
                &format!(
                    "placeholder ссылается на неизвестный output '{}:{}'",
                    placeholder.step_id.as_str(),
                    placeholder.input_id.as_str()
                ),
            ));
        }
        let graph = crate::domain::Graph::validate(
            workflow
                .steps
                .iter()
                .map(|candidate| crate::domain::GraphStep {
                    id: candidate.id.as_str(),
                    depends_on: &candidate.depends_on,
                    outputs: &candidate.outputs,
                })
                .collect(),
        )
        .map_err(|message| invalid_workflow(&workflow.id, &message))?;
        if !graph.guarantees(
            &step.depends_on,
            placeholder.step_id.as_str(),
            placeholder.input_id.as_str(),
        ) {
            return Err(invalid_step(
                &workflow.id,
                step.id.as_str(),
                "placeholder не гарантирован выбранными dependencies",
            ));
        }
    }
    Ok(())
}

fn validate_reachability(workflow: &Workflow) -> Result<(), CommandError> {
    crate::domain::Graph::validate(
        workflow
            .steps
            .iter()
            .map(|step| crate::domain::GraphStep {
                id: step.id.as_str(),
                depends_on: &step.depends_on,
                outputs: &step.outputs,
            })
            .collect(),
    )
    .map(|_| ())
    .map_err(|message| invalid_workflow(&workflow.id, &message))
}

fn invalid_workflow(workflow_id: &WorkflowId, message: &str) -> CommandError {
    CommandError::Invalid {
        context: format!("validate: workflow '{}': {message}", workflow_id.as_str()),
    }
}

fn invalid_step(workflow_id: &WorkflowId, step_id: &str, message: &str) -> CommandError {
    invalid_workflow(workflow_id, &format!("step '{step_id}': {message}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_parser_accepts_both_documented_kinds() {
        let workflow_id = WorkflowId::parse("delivery").expect("workflow ID must be valid");
        let placeholders = parse_placeholders(
            &workflow_id,
            "implement",
            "{{path:plan:spec}} {{content:plan:summary}}",
        )
        .expect("placeholders must be valid");
        assert_eq!(placeholders.len(), 2);
        assert_eq!(placeholders[0].kind, PlaceholderKind::Path);
        assert_eq!(placeholders[1].kind, PlaceholderKind::Content);
    }

    #[test]
    fn placeholder_parser_rejects_unknown_unclosed_and_spaced_forms() {
        let workflow_id = WorkflowId::parse("delivery").expect("workflow ID must be valid");
        for prompt in [
            "{{unknown:plan:spec}}",
            "{{path:plan:spec",
            "{{path: plan:spec}}",
        ] {
            assert!(parse_placeholders(&workflow_id, "implement", prompt).is_err());
        }
    }
}
