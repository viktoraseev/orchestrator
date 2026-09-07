//! Агрегат source workflow и его проверенные идентификаторы; модуль не читает config и template-файлы.

use std::path::PathBuf;

use crate::domain::{Dependencies, Outputs};

use super::Agent;

/// Проверенный идентификатор workflow, безопасный для построения пути template.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct WorkflowId(String);

impl WorkflowId {
    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        SymbolicId::parse("WorkflowId", value).map(|id| Self(id.into_string()))
    }

    pub(crate) fn from_validated(value: String) -> Self {
        Self(value)
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct SymbolicId(String);

impl SymbolicId {
    pub(crate) fn parse(kind: &str, value: &str) -> Result<Self, String> {
        if Self::is_valid(value) {
            Ok(Self(value.to_owned()))
        } else {
            Err(format!("{kind} '{value}' не соответствует kebab-case"))
        }
    }

    pub(crate) fn is_valid(value: &str) -> bool {
        !value.is_empty()
            && value.split('-').all(|part| {
                !part.is_empty()
                    && part
                        .bytes()
                        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
            })
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn into_string(self) -> String {
        self.0
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ProcessStep {
    pub(crate) executable: PathBuf,
    pub(crate) args: Vec<String>,
    pub(crate) cwd: PathBuf,
    pub(crate) stdout: Option<SymbolicId>,
}

#[derive(Clone, Debug)]
pub(crate) struct Step {
    pub(crate) id: SymbolicId,
    pub(crate) agent: Option<Agent>,
    pub(crate) prompt: Option<String>,
    pub(crate) human: bool,
    pub(crate) process: Option<ProcessStep>,
    pub(crate) depends_on: Dependencies,
    pub(crate) outputs: Outputs,
}

#[derive(Clone, Debug)]
pub(crate) struct Workflow {
    pub(crate) id: WorkflowId,
    pub(crate) max_parallel_agents: usize,
    pub(crate) parameters: Vec<SymbolicId>,
    pub(crate) steps: Vec<Step>,
}
