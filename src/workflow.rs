//! Materialization и validation workflow graph; модуль не планирует attempts и не записывает состояние run.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::agent::{AgentRegistry, BuiltinAgentRegistry};
use crate::config::{
    CommandError, ProcessEnvironment, RawAgent, RawConfig, read_config, resolve_state_root,
};

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
        WorkflowId::parse(value).map(Self::Explicit)
    }

    /// Создаёт запрос проверки workflow из `default-workflow` config.
    #[must_use]
    pub const fn configured_default() -> Self {
        Self::ConfiguredDefault
    }
}

/// Проверенный идентификатор workflow, безопасный для построения пути template.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct WorkflowId(String);

impl WorkflowId {
    fn parse(value: &str) -> Result<Self, CommandError> {
        SymbolicId::parse("WorkflowId", value)
            .map(|id| Self(id.into_string()))
            .map_err(|context| CommandError::Syntax {
                context: format!("validate: {context}"),
            })
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct SymbolicId(String);

impl SymbolicId {
    fn parse(kind: &str, value: &str) -> Result<Self, String> {
        let valid = !value.is_empty()
            && value.split('-').all(|part| {
                !part.is_empty()
                    && part
                        .bytes()
                        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
            });
        if valid {
            Ok(Self(value.to_owned()))
        } else {
            Err(format!("{kind} '{value}' не соответствует kebab-case"))
        }
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    fn into_string(self) -> String {
        self.0
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawWorkflow {
    steps: Vec<RawStep>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct RawStep {
    id: String,
    agent: Option<String>,
    prompt: Option<String>,
    human: bool,
    depends_on: Vec<String>,
    outputs: Vec<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct Step {
    pub(crate) id: SymbolicId,
    pub(crate) agent: RawAgent,
    pub(crate) prompt: Option<String>,
    pub(crate) human: bool,
    pub(crate) depends_on: Vec<SymbolicId>,
    pub(crate) outputs: Vec<SymbolicId>,
}

#[derive(Clone, Debug)]
pub(crate) struct Workflow {
    pub(crate) id: WorkflowId,
    pub(crate) max_parallel_agents: usize,
    pub(crate) steps: Vec<Step>,
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
            configured_workflow_id = WorkflowId(value.clone());
            &configured_workflow_id
        }
    };
    let workflow = materialize(
        &root,
        workflow_id,
        &config.raw,
        config.max_parallel_agents().get(),
    )?;
    validate_graph(&workflow)?;
    Ok(format!("workflow {}: valid", workflow.id.as_str()))
}

fn materialize(
    root: &Path,
    workflow_id: &WorkflowId,
    config: &RawConfig,
    max_parallel_agents: usize,
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
        let depends_on = parse_unique_ids(
            workflow_id,
            step_id.as_str(),
            "depends-on",
            raw_step.depends_on,
        )?;
        let outputs = parse_unique_ids(workflow_id, step_id.as_str(), "outputs", raw_step.outputs)?;
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
        steps.push(Step {
            id: step_id,
            agent,
            prompt,
            human: raw_step.human,
            depends_on,
            outputs,
        });
    }
    Ok(Workflow {
        id: workflow_id.clone(),
        max_parallel_agents,
        steps,
    })
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
            configured_workflow_id = WorkflowId(value.clone());
            &configured_workflow_id
        }
    };
    materialize(
        &root,
        workflow_id,
        &config.raw,
        config.max_parallel_agents().get(),
    )
    .and_then(|workflow| {
        validate_graph(&workflow)?;
        Ok(workflow)
    })
    .map_err(|error| replace_context(error, context))
}

fn replace_context(error: CommandError, context: &str) -> CommandError {
    let replace = |value: String| value.replacen("validate:", &format!("{context}:"), 1);
    match error {
        CommandError::Syntax { context: value } => CommandError::Syntax {
            context: replace(value),
        },
        CommandError::Invalid { context: value } => CommandError::Invalid {
            context: replace(value),
        },
        CommandError::NotFound { context: value } => CommandError::NotFound {
            context: replace(value),
        },
        CommandError::Busy { context: value } => CommandError::Busy {
            context: replace(value),
        },
        CommandError::Interrupted {
            context: value,
            exit_code,
        } => CommandError::Interrupted {
            context: replace(value),
            exit_code,
        },
        CommandError::Runtime {
            context: value,
            source,
        } => CommandError::Runtime {
            context: replace(value),
            source,
        },
    }
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

fn parse_unique_ids(
    workflow_id: &WorkflowId,
    step_id: &str,
    field: &str,
    values: Vec<String>,
) -> Result<Vec<SymbolicId>, CommandError> {
    let mut seen = HashSet::with_capacity(values.len());
    let mut parsed = Vec::with_capacity(values.len());
    for value in values {
        let id = SymbolicId::parse(field, &value)
            .map_err(|context| invalid_step(workflow_id, step_id, &context))?;
        if !seen.insert(id.clone()) {
            return Err(invalid_step(
                workflow_id,
                step_id,
                &format!("{field} содержит повтор '{}'", id.as_str()),
            ));
        }
        parsed.push(id);
    }
    Ok(parsed)
}

fn resolve_agent(
    workflow_id: &WorkflowId,
    step_id: &str,
    explicit_agent: Option<&str>,
    config: &RawConfig,
) -> Result<RawAgent, CommandError> {
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
        let _ = (&step.agent, step.human);
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
    }
    validate_reachability(workflow)
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
        if !step.depends_on.contains(&placeholder.step_id) {
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
        if !source.outputs.contains(&placeholder.input_id) {
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
    }
    Ok(())
}

fn validate_reachability(workflow: &Workflow) -> Result<(), CommandError> {
    let mut reachable = HashSet::with_capacity(workflow.steps.len());
    reachable.insert(workflow.steps[0].id.clone());
    loop {
        let previous_len = reachable.len();
        for step in workflow.steps.iter().skip(1) {
            if !step.depends_on.is_empty()
                && step
                    .depends_on
                    .iter()
                    .all(|dependency| reachable.contains(dependency))
            {
                reachable.insert(step.id.clone());
            }
        }
        if reachable.len() == previous_len {
            break;
        }
    }
    if let Some(step) = workflow
        .steps
        .iter()
        .find(|step| !reachable.contains(&step.id))
    {
        return Err(invalid_step(
            &workflow.id,
            step.id.as_str(),
            "Step статически недостижим из initial activation",
        ));
    }
    Ok(())
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
