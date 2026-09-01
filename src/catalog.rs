//! Read-only catalogs source workflows, named Agents и prompt templates; модуль не materialize'ит workflows и не изменяет state root.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::agent::BuiltinAgentRegistry;
use crate::config::{CommandError, ProcessEnvironment, read_config, resolve_state_root};
use crate::run::InspectionFormat;
use crate::workflow::SymbolicId;

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
        InspectionFormat::Text => render_lines(entries.iter().map(|entry| {
            format!(
                "agent {}: type={} model={} reasoning={}",
                entry.agent, entry.agent_type, entry.model, entry.reasoning
            )
        })),
        InspectionFormat::Json => render_json(&entries, "agent list"),
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
