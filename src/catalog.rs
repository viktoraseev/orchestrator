//! Read-only catalogs source workflows, named Agents и prompt templates; модуль не materialize'ит workflows и не изменяет state root.

use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::agent::BuiltinAgentRegistry;
use crate::config::{CommandError, ProcessEnvironment, read_config, resolve_state_root};
use crate::run::InspectionFormat;
use crate::workflow::{RawProcess, RawStep, RawWorkflow, SymbolicId};

/// Descriptor source workflow template.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct WorkflowCatalogEntry {
    workflow: String,
    path: String,
}

impl WorkflowCatalogEntry {
    /// Возвращает validated `WorkflowId` из basename.
    #[must_use]
    pub fn workflow(&self) -> &str {
        &self.workflow
    }

    /// Возвращает абсолютный path template.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }
}

/// Typed source workflow до materialization config и prompt templates.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct SourceWorkflow {
    workflow: String,
    path: String,
    parameters: Vec<String>,
    steps: Vec<SourceWorkflowStep>,
}

impl SourceWorkflow {
    /// Возвращает validated `WorkflowId`.
    #[must_use]
    pub fn workflow(&self) -> &str {
        &self.workflow
    }

    /// Возвращает абсолютный path source template.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Возвращает объявленные `ParameterIds` в детерминированном порядке.
    #[must_use]
    pub fn parameters(&self) -> &[String] {
        &self.parameters
    }

    /// Возвращает Steps в исходном YAML order.
    #[must_use]
    pub fn steps(&self) -> &[SourceWorkflowStep] {
        &self.steps
    }
}

/// Source Step без разрешения Agent и prompt references.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct SourceWorkflowStep {
    id: String,
    agent: Option<String>,
    prompt: Option<String>,
    human: bool,
    process: Option<SourceWorkflowProcess>,
    depends_on: Vec<String>,
    outputs: Vec<String>,
}

/// Source Process executor без разрешения executable и placeholders.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct SourceWorkflowProcess {
    executable: String,
    args: Vec<String>,
    cwd: Option<String>,
    stdout: Option<String>,
}

impl SourceWorkflowProcess {
    /// Возвращает source executable.
    #[must_use]
    pub fn executable(&self) -> &str {
        &self.executable
    }

    /// Возвращает source argv templates.
    #[must_use]
    pub fn args(&self) -> &[String] {
        &self.args
    }

    /// Возвращает optional source cwd.
    #[must_use]
    pub fn cwd(&self) -> Option<&str> {
        self.cwd.as_deref()
    }

    /// Возвращает optional stdout output.
    #[must_use]
    pub fn stdout(&self) -> Option<&str> {
        self.stdout.as_deref()
    }
}

impl SourceWorkflowStep {
    /// Возвращает validated `StepId`.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Возвращает явный source `AgentId`, не применяя default.
    #[must_use]
    pub fn agent(&self) -> Option<&str> {
        self.agent.as_deref()
    }

    /// Возвращает source `PromptId`.
    #[must_use]
    pub fn prompt(&self) -> Option<&str> {
        self.prompt.as_deref()
    }

    /// Возвращает source human flag.
    #[must_use]
    pub const fn is_human(&self) -> bool {
        self.human
    }

    /// Возвращает source Process executor.
    #[must_use]
    pub const fn process(&self) -> Option<&SourceWorkflowProcess> {
        self.process.as_ref()
    }

    /// Возвращает source dependencies в исходном order.
    #[must_use]
    pub fn depends_on(&self) -> &[String] {
        &self.depends_on
    }

    /// Возвращает source outputs в исходном order.
    #[must_use]
    pub fn outputs(&self) -> &[String] {
        &self.outputs
    }
}

/// Descriptor named Agent из validated config.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct AgentCatalogEntry {
    agent: String,
    #[serde(rename = "type")]
    agent_type: String,
    model: String,
    reasoning: String,
}

impl AgentCatalogEntry {
    /// Возвращает validated `AgentId`.
    #[must_use]
    pub fn agent(&self) -> &str {
        &self.agent
    }

    /// Возвращает `AgentTypeId`.
    #[must_use]
    pub fn agent_type(&self) -> &str {
        &self.agent_type
    }

    /// Возвращает model из config.
    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Возвращает reasoning из config.
    #[must_use]
    pub fn reasoning(&self) -> &str {
        &self.reasoning
    }
}

/// Descriptor UTF-8 prompt template.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct PromptCatalogEntry {
    prompt: String,
    bytes: u64,
    path: String,
}

impl PromptCatalogEntry {
    /// Возвращает validated `PromptId` из basename.
    #[must_use]
    pub fn prompt(&self) -> &str {
        &self.prompt
    }

    /// Возвращает размер полностью прочитанного UTF-8 template в bytes.
    #[must_use]
    pub const fn bytes(&self) -> u64 {
        self.bytes
    }

    /// Возвращает абсолютный path template.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }
}

/// Полностью прочитанный UTF-8 prompt template.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct PromptTemplate {
    prompt: String,
    bytes: u64,
    path: String,
    content: String,
}

impl PromptTemplate {
    /// Возвращает validated `PromptId`.
    #[must_use]
    pub fn prompt(&self) -> &str {
        &self.prompt
    }

    /// Возвращает размер content в UTF-8 bytes.
    #[must_use]
    pub const fn bytes(&self) -> u64 {
        self.bytes
    }

    /// Возвращает абсолютный path template.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Возвращает точное UTF-8 содержимое template.
    #[must_use]
    pub fn content(&self) -> &str {
        &self.content
    }
}

/// Строит typed catalog source workflow templates без чтения их содержимого или config.
///
/// # Errors
///
/// Возвращает [`CommandError::Invalid`] для contract file с невалидным `WorkflowId` или non-regular path и [`CommandError::Runtime`] для ошибки файловой системы.
pub fn list_workflows(
    environment: &ProcessEnvironment,
) -> Result<Vec<WorkflowCatalogEntry>, CommandError> {
    let root = resolve_state_root(environment, "workflow list")?;
    catalog_paths(&root, "workflow", "yaml", "WorkflowId", "workflow list")?
        .into_iter()
        .map(|(workflow, path)| {
            Ok(WorkflowCatalogEntry {
                workflow,
                path: catalog_path_string(&path, "workflow list")?,
            })
        })
        .collect()
}

/// Рендерит полный workflow catalog в text или JSON.
///
/// # Errors
///
/// Возвращает ошибки [`list_workflows`] либо serialization.
pub fn execute_workflow_list(
    environment: &ProcessEnvironment,
    format: InspectionFormat,
) -> Result<String, CommandError> {
    let entries = list_workflows(environment)?;
    match format {
        InspectionFormat::Text => render_lines(entries.iter().map(|entry| entry.workflow.as_str())),
        InspectionFormat::Json => render_json(&entries, "workflow list"),
    }
}

/// Читает и структурно проверяет один source workflow без materialization и graph validation.
///
/// # Errors
///
/// Возвращает `2` для невалидного `WorkflowId`, `4` для отсутствующего template, `3` для non-regular, non-UTF-8 или невалидного YAML/schema и runtime error для I/O.
pub fn show_workflow(
    workflow_id: &str,
    environment: &ProcessEnvironment,
) -> Result<SourceWorkflow, CommandError> {
    let workflow = parse_selected_id(workflow_id, "WorkflowId", "workflow show")?;
    let root = resolve_state_root(environment, "workflow show")?;
    let path = selected_catalog_path(&root, "workflow", "yaml", &workflow, "workflow show")?;
    let bytes = fs::read(&path).map_err(|source| CommandError::Runtime {
        context: format!("workflow show: не удалось прочитать {}", path.display()),
        source,
    })?;
    std::str::from_utf8(&bytes).map_err(|_| CommandError::Invalid {
        context: format!("workflow show: workflow '{workflow}' не является UTF-8"),
    })?;
    let raw: RawWorkflow =
        serde_yaml::from_slice(&bytes).map_err(|source| CommandError::Invalid {
            context: format!("workflow show: невалидный workflow '{workflow}': {source}"),
        })?;
    let parameters = validate_source_parameters(&workflow, raw.parameters)?;
    let steps = validate_source_steps(&workflow, raw.steps)?;
    Ok(SourceWorkflow {
        workflow,
        path: catalog_path_string(&path, "workflow show")?,
        parameters,
        steps,
    })
}

/// Рендерит один source workflow в text или JSON.
///
/// # Errors
///
/// Возвращает ошибки [`show_workflow`] либо serialization.
pub fn execute_workflow_show(
    workflow_id: &str,
    environment: &ProcessEnvironment,
    format: InspectionFormat,
) -> Result<String, CommandError> {
    let workflow = show_workflow(workflow_id, environment)?;
    match format {
        InspectionFormat::Text => render_source_workflow(&workflow),
        InspectionFormat::Json => render_json(&workflow, "workflow show"),
    }
}

/// Строит typed catalog named Agents после полной validation config.
///
/// # Errors
///
/// Возвращает ошибки state root, чтения или полной config/Agent validation.
pub fn list_agents(
    environment: &ProcessEnvironment,
) -> Result<Vec<AgentCatalogEntry>, CommandError> {
    let root = resolve_state_root(environment, "agent list")?;
    let document = read_config(&root, "agent list", &BuiltinAgentRegistry)?;
    Ok(document
        .raw
        .agents
        .into_iter()
        .map(|(agent, value)| AgentCatalogEntry {
            agent,
            agent_type: value.r#type,
            model: value.model,
            reasoning: value.reasoning,
        })
        .collect())
}

/// Рендерит полный Agent catalog в text или JSON.
///
/// # Errors
///
/// Возвращает ошибки [`list_agents`] либо serialization.
pub fn execute_agent_list(
    environment: &ProcessEnvironment,
    format: InspectionFormat,
) -> Result<String, CommandError> {
    let entries = list_agents(environment)?;
    match format {
        InspectionFormat::Text => render_lines(entries.iter().map(render_agent)),
        InspectionFormat::Json => render_json(&entries, "agent list"),
    }
}

/// Выбирает одного named Agent после полной validation config.
///
/// # Errors
///
/// Возвращает `2` для невалидного `AgentId`, `4` для отсутствующего Agent и ошибки полной config/Agent validation.
pub fn show_agent(
    agent_id: &str,
    environment: &ProcessEnvironment,
) -> Result<AgentCatalogEntry, CommandError> {
    let agent = parse_selected_id(agent_id, "AgentId", "agent show")?;
    let root = resolve_state_root(environment, "agent show")?;
    let document = read_config(&root, "agent show", &BuiltinAgentRegistry)?;
    let value = document
        .raw
        .agents
        .get(&agent)
        .ok_or_else(|| CommandError::NotFound {
            context: format!("agent show: Agent '{agent}' не существует"),
        })?;
    Ok(AgentCatalogEntry {
        agent,
        agent_type: value.r#type.clone(),
        model: value.model.clone(),
        reasoning: value.reasoning.clone(),
    })
}

/// Рендерит одного named Agent в text или JSON.
///
/// # Errors
///
/// Возвращает ошибки [`show_agent`] либо serialization.
pub fn execute_agent_show(
    agent_id: &str,
    environment: &ProcessEnvironment,
    format: InspectionFormat,
) -> Result<String, CommandError> {
    let agent = show_agent(agent_id, environment)?;
    match format {
        InspectionFormat::Text => Ok(render_agent(&agent)),
        InspectionFormat::Json => render_json(&agent, "agent show"),
    }
}

/// Строит typed catalog source prompt templates после проверки regular file и UTF-8.
///
/// # Errors
///
/// Возвращает [`CommandError::Invalid`] для невалидного `PromptId`, non-regular или non-UTF-8 template и [`CommandError::Runtime`] для ошибки файловой системы.
pub fn list_prompts(
    environment: &ProcessEnvironment,
) -> Result<Vec<PromptCatalogEntry>, CommandError> {
    let root = resolve_state_root(environment, "prompt list")?;
    catalog_paths(&root, "prompt", "md", "PromptId", "prompt list")?
        .into_iter()
        .map(|(prompt, path)| {
            let bytes = fs::read(&path).map_err(|source| CommandError::Runtime {
                context: format!("prompt list: не удалось прочитать {}", path.display()),
                source,
            })?;
            std::str::from_utf8(&bytes).map_err(|_| CommandError::Invalid {
                context: format!("prompt list: Prompt '{prompt}' не является UTF-8"),
            })?;
            Ok(PromptCatalogEntry {
                prompt,
                bytes: u64::try_from(bytes.len()).map_err(|source| CommandError::Runtime {
                    context: "prompt list: размер template не помещается в u64".to_owned(),
                    source: std::io::Error::other(source),
                })?,
                path: catalog_path_string(&path, "prompt list")?,
            })
        })
        .collect()
}

/// Рендерит полный prompt catalog в text или JSON.
///
/// # Errors
///
/// Возвращает ошибки [`list_prompts`] либо serialization.
pub fn execute_prompt_list(
    environment: &ProcessEnvironment,
    format: InspectionFormat,
) -> Result<String, CommandError> {
    let entries = list_prompts(environment)?;
    match format {
        InspectionFormat::Text => render_lines(entries.iter().map(|entry| {
            format!(
                "prompt {}: bytes={} path={}",
                entry.prompt, entry.bytes, entry.path
            )
        })),
        InspectionFormat::Json => render_json(&entries, "prompt list"),
    }
}

/// Читает один source prompt template целиком.
///
/// # Errors
///
/// Возвращает `2` для невалидного `PromptId`, `4` для отсутствующего template, `3` для non-regular или non-UTF-8 файла и runtime error для I/O.
pub fn show_prompt(
    prompt_id: &str,
    environment: &ProcessEnvironment,
) -> Result<PromptTemplate, CommandError> {
    let prompt = parse_selected_id(prompt_id, "PromptId", "prompt show")?;
    let root = resolve_state_root(environment, "prompt show")?;
    let path = selected_catalog_path(&root, "prompt", "md", &prompt, "prompt show")?;
    let bytes = fs::read(&path).map_err(|source| CommandError::Runtime {
        context: format!("prompt show: не удалось прочитать {}", path.display()),
        source,
    })?;
    let byte_count = u64::try_from(bytes.len()).map_err(|source| CommandError::Runtime {
        context: "prompt show: размер template не помещается в u64".to_owned(),
        source: std::io::Error::other(source),
    })?;
    let content = String::from_utf8(bytes).map_err(|_| CommandError::Invalid {
        context: format!("prompt show: Prompt '{prompt}' не является UTF-8"),
    })?;
    Ok(PromptTemplate {
        prompt,
        bytes: byte_count,
        path: catalog_path_string(&path, "prompt show")?,
        content,
    })
}

/// Рендерит один prompt template как точный text content или JSON descriptor.
///
/// # Errors
///
/// Возвращает ошибки [`show_prompt`] либо serialization.
pub fn execute_prompt_show(
    prompt_id: &str,
    environment: &ProcessEnvironment,
    format: InspectionFormat,
) -> Result<String, CommandError> {
    let prompt = show_prompt(prompt_id, environment)?;
    match format {
        InspectionFormat::Text => Ok(prompt.content),
        InspectionFormat::Json => render_json(&prompt, "prompt show"),
    }
}

fn validate_source_steps(
    workflow: &str,
    raw_steps: Vec<RawStep>,
) -> Result<Vec<SourceWorkflowStep>, CommandError> {
    if raw_steps.is_empty() {
        return Err(invalid_source_workflow(
            workflow,
            "steps должен быть непустым",
        ));
    }
    let mut step_ids = HashSet::with_capacity(raw_steps.len());
    let mut steps = Vec::with_capacity(raw_steps.len());
    for (index, raw) in raw_steps.into_iter().enumerate() {
        let id = parse_source_id(workflow, index, "StepId", &raw.id)?;
        if !step_ids.insert(id.clone()) {
            return Err(invalid_source_step(workflow, &id, "StepId повторяется"));
        }
        let agent = raw
            .agent
            .map(|value| parse_source_id(workflow, index, "AgentId", &value))
            .transpose()?;
        let prompt = raw
            .prompt
            .map(|value| parse_source_id(workflow, index, "PromptId", &value))
            .transpose()?;
        let process = raw
            .process
            .map(|process| validate_source_process(workflow, &id, process))
            .transpose()?;
        if process.is_some() && (agent.is_some() || prompt.is_some() || raw.human) {
            return Err(invalid_source_step(
                workflow,
                &id,
                "Process Step несовместим с agent, prompt и human: true",
            ));
        }
        let depends_on =
            parse_source_ids(workflow, index, &id, "depends-on", "StepId", raw.depends_on)?;
        let outputs = parse_source_ids(workflow, index, &id, "outputs", "InputId", raw.outputs)?;
        if let Some(stdout) = process.as_ref().and_then(SourceWorkflowProcess::stdout)
            && !outputs.iter().any(|output| output == stdout)
        {
            return Err(invalid_source_step(
                workflow,
                &id,
                &format!("Process stdout ссылается на неизвестный output '{stdout}'"),
            ));
        }
        steps.push(SourceWorkflowStep {
            id,
            agent,
            prompt,
            human: raw.human,
            process,
            depends_on,
            outputs,
        });
    }
    Ok(steps)
}

fn validate_source_parameters(
    workflow: &str,
    parameters: BTreeMap<String, String>,
) -> Result<Vec<String>, CommandError> {
    parameters
        .into_iter()
        .map(|(id, parameter_type)| {
            let id = SymbolicId::parse("ParameterId", &id)
                .map_err(|message| invalid_source_workflow(workflow, &message))?;
            if parameter_type != "string" {
                return Err(invalid_source_workflow(
                    workflow,
                    &format!(
                        "Parameter '{}' имеет неподдерживаемый type '{parameter_type}'",
                        id.as_str()
                    ),
                ));
            }
            Ok(id.into_string())
        })
        .collect()
}

fn validate_source_process(
    workflow: &str,
    step_id: &str,
    process: RawProcess,
) -> Result<SourceWorkflowProcess, CommandError> {
    if process.executable.is_empty()
        || process.executable.contains('\0')
        || process.executable.contains("{{")
        || process.executable.contains("}}")
    {
        return Err(invalid_source_step(
            workflow,
            step_id,
            "Process executable должен быть непустой строкой без NUL и placeholders",
        ));
    }
    if process.cwd.as_ref().is_some_and(|cwd| {
        cwd.is_empty() || cwd.contains('\0') || cwd.contains("{{") || cwd.contains("}}")
    }) {
        return Err(invalid_source_step(
            workflow,
            step_id,
            "Process cwd должен быть непустой строкой без NUL и placeholders",
        ));
    }
    if process.args.iter().any(|argument| argument.contains('\0')) {
        return Err(invalid_source_step(
            workflow,
            step_id,
            "Process args не могут содержать NUL",
        ));
    }
    for argument in &process.args {
        validate_source_process_argument(workflow, step_id, argument)?;
    }
    let stdout = process
        .stdout
        .map(|value| SymbolicId::parse("InputId", &value).map(SymbolicId::into_string))
        .transpose()
        .map_err(|message| invalid_source_step(workflow, step_id, &message))?;
    Ok(SourceWorkflowProcess {
        executable: process.executable,
        args: process.args,
        cwd: process.cwd,
        stdout,
    })
}

fn validate_source_process_argument(
    workflow: &str,
    step_id: &str,
    argument: &str,
) -> Result<(), CommandError> {
    if !argument.contains("{{") && !argument.contains("}}") {
        return Ok(());
    }
    let body = argument
        .strip_prefix("{{")
        .and_then(|value| value.strip_suffix("}}"))
        .ok_or_else(|| {
            invalid_source_step(
                workflow,
                step_id,
                "Process placeholder должен занимать весь argv element",
            )
        })?;
    let parts = body.split(':').collect::<Vec<_>>();
    let ids_are_valid = match parts.as_slice() {
        ["param" | "output", parameter] => SymbolicId::parse("InputId", parameter).is_ok(),
        ["path", step, input] => {
            SymbolicId::parse("StepId", step).is_ok() && SymbolicId::parse("InputId", input).is_ok()
        }
        _ => false,
    };
    if ids_are_valid {
        Ok(())
    } else {
        Err(invalid_source_step(
            workflow,
            step_id,
            &format!("невалидный Process placeholder '{{{{{body}}}}}'"),
        ))
    }
}

fn parse_source_ids(
    workflow: &str,
    index: usize,
    step_id: &str,
    field: &str,
    kind: &str,
    values: Vec<String>,
) -> Result<Vec<String>, CommandError> {
    let mut unique = HashSet::with_capacity(values.len());
    let mut parsed = Vec::with_capacity(values.len());
    for value in values {
        let value = parse_source_id(workflow, index, kind, &value)?;
        if !unique.insert(value.clone()) {
            return Err(invalid_source_step(
                workflow,
                step_id,
                &format!("{field} содержит повтор '{value}'"),
            ));
        }
        parsed.push(value);
    }
    Ok(parsed)
}

fn parse_source_id(
    workflow: &str,
    index: usize,
    kind: &str,
    value: &str,
) -> Result<String, CommandError> {
    SymbolicId::parse(kind, value)
        .map(SymbolicId::into_string)
        .map_err(|message| invalid_source_workflow(workflow, &format!("Step #{index}: {message}")))
}

fn invalid_source_workflow(workflow: &str, message: &str) -> CommandError {
    CommandError::Invalid {
        context: format!("workflow show: workflow '{workflow}': {message}"),
    }
}

fn invalid_source_step(workflow: &str, step_id: &str, message: &str) -> CommandError {
    invalid_source_workflow(workflow, &format!("Step '{step_id}': {message}"))
}

fn parse_selected_id(value: &str, kind: &str, context: &str) -> Result<String, CommandError> {
    SymbolicId::parse(kind, value)
        .map(SymbolicId::into_string)
        .map_err(|message| CommandError::Syntax {
            context: format!("{context}: {message}"),
        })
}

fn selected_catalog_path(
    root: &Path,
    directory: &str,
    extension: &str,
    id: &str,
    context: &str,
) -> Result<PathBuf, CommandError> {
    let path = root.join(directory).join(format!("{id}.{extension}"));
    match fs::metadata(&path) {
        Ok(metadata) if metadata.is_file() => Ok(path),
        Ok(_) => Err(CommandError::Invalid {
            context: format!("{context}: {} не является regular file", path.display()),
        }),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            Err(CommandError::NotFound {
                context: format!("{context}: '{id}' не существует"),
            })
        }
        Err(source) => Err(CommandError::Runtime {
            context: format!("{context}: не удалось проверить {}", path.display()),
            source,
        }),
    }
}

fn render_source_workflow(workflow: &SourceWorkflow) -> Result<String, CommandError> {
    let mut output = String::new();
    writeln!(
        output,
        "workflow {}: path={}",
        workflow.workflow, workflow.path
    )
    .map_err(text_format_error)?;
    for step in &workflow.steps {
        if let Some(process) = &step.process {
            writeln!(
                output,
                "step {}: process={} args={} cwd={} stdout={} depends-on={} outputs={}",
                step.id,
                process.executable,
                process.args.len(),
                process.cwd.as_deref().unwrap_or("-"),
                process.stdout.as_deref().unwrap_or("-"),
                joined_or_dash(&step.depends_on),
                joined_or_dash(&step.outputs)
            )
            .map_err(text_format_error)?;
        } else {
            writeln!(
                output,
                "step {}: agent={} prompt={} human={} depends-on={} outputs={}",
                step.id,
                step.agent.as_deref().unwrap_or("-"),
                step.prompt.as_deref().unwrap_or("-"),
                step.human,
                joined_or_dash(&step.depends_on),
                joined_or_dash(&step.outputs)
            )
            .map_err(text_format_error)?;
        }
    }
    output.pop();
    Ok(output)
}

fn render_agent(agent: &AgentCatalogEntry) -> String {
    format!(
        "agent {}: type={} model={} reasoning={}",
        agent.agent, agent.agent_type, agent.model, agent.reasoning
    )
}

fn joined_or_dash(values: &[String]) -> String {
    if values.is_empty() {
        "-".to_owned()
    } else {
        values.join(",")
    }
}

fn text_format_error(source: std::fmt::Error) -> CommandError {
    CommandError::Runtime {
        context: "source catalog: не удалось сформировать text output".to_owned(),
        source: std::io::Error::other(source),
    }
}

fn catalog_paths(
    root: &Path,
    directory_name: &str,
    extension: &str,
    id_kind: &str,
    context: &str,
) -> Result<Vec<(String, PathBuf)>, CommandError> {
    let directory = root.join(directory_name);
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(CommandError::Runtime {
                context: format!("{context}: не удалось прочитать {}", directory.display()),
                source,
            });
        }
    };
    let mut result = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| CommandError::Runtime {
            context: format!("{context}: не удалось прочитать catalog entry"),
            source,
        })?;
        let path = entry.path();
        let Some(file_name) = path.file_name() else {
            continue;
        };
        if file_name.as_encoded_bytes().starts_with(b".")
            || path.extension() != Some(extension.as_ref())
        {
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(|value| value.to_str())
            .ok_or_else(|| CommandError::Invalid {
                context: format!(
                    "{context}: имя contract file {} не является UTF-8",
                    path.display()
                ),
            })?;
        let id = SymbolicId::parse(id_kind, stem).map_err(|message| CommandError::Invalid {
            context: format!("{context}: {message}"),
        })?;
        let metadata = fs::metadata(&path).map_err(|source| CommandError::Runtime {
            context: format!("{context}: не удалось проверить {}", path.display()),
            source,
        })?;
        if !metadata.is_file() {
            return Err(CommandError::Invalid {
                context: format!("{context}: {} не является regular file", path.display()),
            });
        }
        result.push((id.into_string(), path));
    }
    result.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(result)
}

fn catalog_path_string(path: &Path, context: &str) -> Result<String, CommandError> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| CommandError::Invalid {
            context: format!("{context}: path {} не является UTF-8", path.display()),
        })
}

fn render_lines(values: impl IntoIterator<Item = impl AsRef<str>>) -> Result<String, CommandError> {
    let mut output = String::new();
    for value in values {
        writeln!(output, "{}", value.as_ref()).map_err(|source| CommandError::Runtime {
            context: "source catalog: не удалось сформировать text output".to_owned(),
            source: std::io::Error::other(source),
        })?;
    }
    if output.ends_with('\n') {
        output.pop();
    }
    Ok(output)
}

fn render_json(value: &impl Serialize, context: &str) -> Result<String, CommandError> {
    serde_json::to_string_pretty(value).map_err(|source| CommandError::Runtime {
        context: format!("{context}: не удалось сериализовать JSON"),
        source: std::io::Error::other(source),
    })
}
