//! Durable run definition materialized from a source workflow; модуль не читает и не публикует состояние.

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::{Agent, SymbolicId, Workflow};
use crate::agent::AgentRegistry;
use crate::config::CommandError;

/// Проверенный десятичный идентификатор run.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RunId(pub(crate) u64);

impl RunId {
    /// Разбирает обязательный CLI `RunId`.
    ///
    /// # Errors
    ///
    /// Возвращает [`CommandError::Syntax`], если значение не является десятичным целым.
    pub fn parse(value: &str) -> Result<Self, CommandError> {
        value
            .parse::<u64>()
            .map(Self)
            .map_err(|_| CommandError::Syntax {
                context: format!("resume: RunId '{value}' должен быть десятичным integer"),
            })
    }
}

impl std::fmt::Display for RunId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub(crate) struct MaterializedWorkflow {
    pub(crate) workflow_id: String,
    pub(crate) max_parallel_agents: usize,
    #[serde(default)]
    pub(crate) parameters: BTreeMap<String, String>,
    pub(crate) steps: Vec<MaterializedStep>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub(crate) struct MaterializedStep {
    pub(crate) id: String,
    pub(crate) agent: Option<Agent>,
    pub(crate) prompt: Option<String>,
    pub(crate) human: bool,
    pub(crate) process: Option<MaterializedProcess>,
    pub(crate) depends_on: Vec<String>,
    pub(crate) outputs: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MaterializedProcess {
    pub(crate) executable: PathBuf,
    pub(crate) args: Vec<String>,
    pub(crate) cwd: PathBuf,
    pub(crate) stdout: Option<String>,
}

impl MaterializedWorkflow {
    pub(crate) fn from_workflow(workflow: Workflow, parameters: BTreeMap<String, String>) -> Self {
        Self {
            workflow_id: workflow.id.as_str().to_owned(),
            max_parallel_agents: workflow.max_parallel_agents,
            parameters,
            steps: workflow
                .steps
                .into_iter()
                .map(|step| MaterializedStep {
                    id: step.id.as_str().to_owned(),
                    agent: step.agent,
                    prompt: step.prompt,
                    human: step.human,
                    process: step.process.map(|process| MaterializedProcess {
                        executable: process.executable,
                        args: process.args,
                        cwd: process.cwd,
                        stdout: process.stdout.map(SymbolicId::into_string),
                    }),
                    depends_on: step
                        .depends_on
                        .into_iter()
                        .map(super::SymbolicId::into_string)
                        .collect(),
                    outputs: step
                        .outputs
                        .into_iter()
                        .map(super::SymbolicId::into_string)
                        .collect(),
                })
                .collect(),
        }
    }

    pub(crate) fn validate(&self, registry: &dyn AgentRegistry) -> Result<(), String> {
        if self.max_parallel_agents == 0
            || self.steps.is_empty()
            || !SymbolicId::is_valid(&self.workflow_id)
            || self
                .parameters
                .iter()
                .any(|(id, value)| !SymbolicId::is_valid(id) || value.contains('\0'))
        {
            return Err("невалидный materialized workflow".to_owned());
        }
        let mut ids = HashSet::with_capacity(self.steps.len());
        for step in &self.steps {
            let outputs: HashSet<&str> = step.outputs.iter().map(String::as_str).collect();
            if !SymbolicId::is_valid(&step.id)
                || !ids.insert(step.id.as_str())
                || outputs.len() != step.outputs.len()
                || step.outputs.iter().any(|id| !SymbolicId::is_valid(id))
            {
                return Err("невалидные или повторяющиеся Step/Input IDs".to_owned());
            }
            match (&step.agent, &step.process) {
                (Some(agent), None) => {
                    registry.validate(&agent.r#type, &agent.model, &agent.reasoning)?;
                }
                (None, Some(process)) => {
                    if step.human
                        || step.prompt.is_some()
                        || !process.executable.is_absolute()
                        || !process.cwd.is_absolute()
                        || process
                            .stdout
                            .as_ref()
                            .is_some_and(|id| !step.outputs.contains(id))
                    {
                        return Err("невалидный materialized Process Step".to_owned());
                    }
                }
                _ => {
                    return Err(
                        "Step должен содержать ровно один Agent или Process executor".to_owned(),
                    );
                }
            }
        }
        for step in &self.steps {
            let dependencies: HashSet<&str> = step.depends_on.iter().map(String::as_str).collect();
            if dependencies.len() != step.depends_on.len()
                || step
                    .depends_on
                    .iter()
                    .any(|dependency| !ids.contains(dependency.as_str()))
            {
                return Err("depends-on повторяется или ссылается на неизвестный Step".to_owned());
            }
            if let Some(process) = &step.process {
                for argument in &process.args {
                    self.validate_process_argument(step, argument)?;
                }
            }
        }
        Ok(())
    }

    fn validate_process_argument(
        &self,
        step: &MaterializedStep,
        argument: &str,
    ) -> Result<(), String> {
        if argument.contains('\0') {
            return Err("Process argv содержит NUL".to_owned());
        }
        if !argument.contains("{{") && !argument.contains("}}") {
            return Ok(());
        }
        let body = argument
            .strip_prefix("{{")
            .and_then(|value| value.strip_suffix("}}"))
            .ok_or_else(|| "Process placeholder не занимает весь argv element".to_owned())?;
        let parts = body.split(':').collect::<Vec<_>>();
        let valid = match parts.as_slice() {
            ["param", parameter] => self.parameters.contains_key(*parameter),
            ["path", source_step, input_id] => {
                step.depends_on
                    .iter()
                    .any(|dependency| dependency == source_step)
                    && self.steps.iter().any(|source| {
                        source.id == *source_step
                            && source.outputs.iter().any(|output| output == input_id)
                    })
            }
            ["output", output] => step.outputs.iter().any(|candidate| candidate == output),
            _ => false,
        };
        if valid {
            Ok(())
        } else {
            Err("невалидный materialized Process placeholder".to_owned())
        }
    }
}
