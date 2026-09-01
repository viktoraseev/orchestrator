//! Чтение, проверка и атомарная публикация конфигурации; модуль не разбирает CLI и не изменяет состояние run.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::agent::{AgentRegistry, BuiltinAgentRegistry};

const DEFAULT_MAX_PARALLEL_AGENTS: usize = 5;
static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Переменные процесса, влияющие на разрешение корня состояния.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProcessEnvironment {
    /// Значение `HOME`, если оно присутствует в окружении.
    pub home: Option<OsString>,
    /// Значение `ORC_HOME`, если оно присутствует в окружении.
    pub orc_home: Option<OsString>,
}

/// Поддерживаемый ключ публичных config-команд.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigKey {
    /// Workflow, выбираемый lifecycle-командами без явного ID.
    DefaultWorkflow,
    /// Agent, выбираемый Step без явного Agent ID.
    DefaultAgent,
    /// Общий лимит одновременно работающих процессов агентов.
    MaxParallelAgents,
}

/// Операция над конфигурацией после разбора CLI.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConfigCommand {
    /// Прочитать одно эффективное значение.
    Get(ConfigKey),
    /// Прочитать все эффективные значения в стабильном порядке.
    List,
    /// Атомарно заменить общий лимит процессов агентов.
    SetMaxParallelAgents(NonZeroUsize),
    /// Атомарно выбрать существующий workflow template по умолчанию.
    SetDefaultWorkflow(WorkflowId),
    /// Атомарно выбрать существующего именованного Agent по умолчанию.
    SetDefaultAgent(AgentId),
}

impl ConfigCommand {
    /// Создаёт команду изменения лимита из CLI-значения.
    ///
    /// # Errors
    ///
    /// Возвращает [`CommandError::Invalid`], если значение не является положительным integer.
    pub fn set_max_parallel_agents(value: &str) -> Result<Self, CommandError> {
        let value = value
            .parse::<usize>()
            .ok()
            .and_then(NonZeroUsize::new)
            .ok_or_else(|| CommandError::Invalid {
                context:
                    "config set max-parallel-agents: значение должно быть положительным integer"
                        .to_owned(),
            })?;
        Ok(Self::SetMaxParallelAgents(value))
    }

    /// Создаёт команду выбора workflow из CLI-значения.
    ///
    /// # Errors
    ///
    /// Возвращает [`CommandError::Syntax`], если ID не соответствует kebab-case.
    pub fn set_default_workflow(value: &str) -> Result<Self, CommandError> {
        WorkflowId::parse(value).map(Self::SetDefaultWorkflow)
    }

    /// Создаёт команду выбора Agent из CLI-значения.
    ///
    /// # Errors
    ///
    /// Возвращает [`CommandError::Syntax`], если ID не соответствует kebab-case.
    pub fn set_default_agent(value: &str) -> Result<Self, CommandError> {
        AgentId::parse(value).map(Self::SetDefaultAgent)
    }
}

/// Проверенный файловый идентификатор workflow.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkflowId(String);

impl WorkflowId {
    fn parse(value: &str) -> Result<Self, CommandError> {
        validate_id("WorkflowId", value).map_err(|context| CommandError::Syntax {
            context: format!("config set default-workflow: {context}"),
        })?;
        Ok(Self(value.to_owned()))
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

/// Проверенный идентификатор именованного Agent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentId(String);

impl AgentId {
    fn parse(value: &str) -> Result<Self, CommandError> {
        validate_id("AgentId", value).map_err(|context| CommandError::Syntax {
            context: format!("config set default-agent: {context}"),
        })?;
        Ok(Self(value.to_owned()))
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

/// Ошибка config-команды с соответствующим публичному CLI кодом завершения.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CommandError {
    /// Аргумент или форма команды не входят в публичный синтаксис.
    #[error("{context}")]
    Syntax {
        /// Диагностика с контекстом команды.
        context: String,
    },
    /// Корень состояния или config не соответствует формату.
    #[error("{context}")]
    Invalid {
        /// Диагностика с контекстом команды.
        context: String,
    },
    /// Прямо указанный объект состояния не существует.
    #[error("{context}")]
    NotFound {
        /// Диагностика с контекстом команды.
        context: String,
    },
    /// Run lock занят либо volatile control context недоступен.
    #[error("{context}")]
    Busy {
        /// Диагностика с контекстом lifecycle или control-команды.
        context: String,
    },
    /// Lifecycle остановлен поддерживаемым Unix termination signal.
    #[error("{context}")]
    Interrupted {
        /// Диагностика с первым полученным сигналом.
        context: String,
        /// Документированный код `128 + signal`.
        exit_code: u8,
    },
    /// Состояние не удалось прочитать по причине ошибки ввода-вывода.
    #[error("{context}: {source}")]
    Runtime {
        /// Диагностика с контекстом команды.
        context: String,
        /// Исходная ошибка файловой системы.
        #[source]
        source: std::io::Error,
    },
}

impl CommandError {
    /// Возвращает exit code категории ошибки из Rule `Ошибки config-команд не маскируются значениями по умолчанию`.
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        match self {
            Self::Syntax { .. } => 2,
            Self::Invalid { .. } => 3,
            Self::NotFound { .. } => 4,
            Self::Busy { .. } => 5,
            Self::Interrupted { exit_code, .. } => *exit_code,
            Self::Runtime { .. } => 1,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub(crate) struct RawConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) default_workflow: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) default_agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) max_parallel_agents: Option<usize>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) agents: BTreeMap<String, RawAgent>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawAgent {
    pub(crate) r#type: String,
    pub(crate) model: String,
    pub(crate) reasoning: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Config {
    default_workflow: Option<String>,
    default_agent: Option<String>,
    max_parallel_agents: NonZeroUsize,
}

#[derive(Clone, Debug)]
pub(crate) struct ConfigDocument {
    pub(crate) raw: RawConfig,
    effective: Config,
}

impl ConfigDocument {
    pub(crate) const fn max_parallel_agents(&self) -> NonZeroUsize {
        self.effective.max_parallel_agents
    }
}

impl TryFrom<RawConfig> for Config {
    type Error = String;

    fn try_from(raw: RawConfig) -> Result<Self, Self::Error> {
        validate_optional_id("default-workflow", raw.default_workflow.as_deref())?;
        validate_optional_id("default-agent", raw.default_agent.as_deref())?;

        for (agent_id, agent) in &raw.agents {
            validate_id("AgentId", agent_id)?;
            validate_id("AgentTypeId", &agent.r#type)?;
        }

        if let Some(default_agent) = &raw.default_agent
            && !raw.agents.contains_key(default_agent)
        {
            return Err(format!(
                "default-agent '{default_agent}' отсутствует в agents"
            ));
        }

        let max_parallel_agents = raw
            .max_parallel_agents
            .map_or_else(
                || NonZeroUsize::new(DEFAULT_MAX_PARALLEL_AGENTS),
                NonZeroUsize::new,
            )
            .ok_or_else(|| "max-parallel-agents должен быть положительным integer".to_owned())?;

        Ok(Self {
            default_workflow: raw.default_workflow,
            default_agent: raw.default_agent,
            max_parallel_agents,
        })
    }
}

/// Выполняет config-команду относительно явно переданного окружения.
///
/// Функция не читает текущий каталог и никогда не изменяет runs; set-команды публикуют весь config только после проверки кандидата.
///
/// # Errors
///
/// Возвращает [`CommandError::Invalid`] для невалидного корня, config или значения, [`CommandError::NotFound`] для отсутствующего прямо указанного workflow и [`CommandError::Runtime`] для ошибки файловой системы.
pub fn execute_config(
    command: &ConfigCommand,
    environment: &ProcessEnvironment,
) -> Result<String, CommandError> {
    let root = resolve_state_root(environment, "config")?;
    let mut document = read_config(&root, "config", &BuiltinAgentRegistry)?;

    Ok(match command {
        ConfigCommand::Get(key) => render_value(&document.effective, *key),
        ConfigCommand::List => render_list(&document.effective),
        ConfigCommand::SetMaxParallelAgents(value) => {
            document.raw.max_parallel_agents = Some(value.get());
            document.effective = Config::try_from(document.raw.clone()).map_err(|context| {
                CommandError::Invalid {
                    context: format!("config set max-parallel-agents: {context}"),
                }
            })?;
            publish_config(&root, &document.raw)?;
            format!("max-parallel-agents: {value}")
        }
        ConfigCommand::SetDefaultWorkflow(workflow_id) => {
            require_workflow_template(&root, workflow_id)?;
            document.raw.default_workflow = Some(workflow_id.as_str().to_owned());
            document.effective = Config::try_from(document.raw.clone()).map_err(|context| {
                CommandError::Invalid {
                    context: format!("config set default-workflow: {context}"),
                }
            })?;
            publish_config(&root, &document.raw)?;
            format!("default-workflow: {}", workflow_id.as_str())
        }
        ConfigCommand::SetDefaultAgent(agent_id) => {
            if !document.raw.agents.contains_key(agent_id.as_str()) {
                return Err(CommandError::NotFound {
                    context: format!(
                        "config set default-agent: Agent '{}' не существует",
                        agent_id.as_str()
                    ),
                });
            }
            document.raw.default_agent = Some(agent_id.as_str().to_owned());
            document.effective = Config::try_from(document.raw.clone()).map_err(|context| {
                CommandError::Invalid {
                    context: format!("config set default-agent: {context}"),
                }
            })?;
            publish_config(&root, &document.raw)?;
            format!("default-agent: {}", agent_id.as_str())
        }
    })
}

fn require_workflow_template(root: &Path, workflow_id: &WorkflowId) -> Result<(), CommandError> {
    let path = root
        .join("workflow")
        .join(format!("{}.yaml", workflow_id.as_str()));
    match fs::metadata(&path) {
        Ok(metadata) if metadata.is_file() => Ok(()),
        Ok(_) => Err(CommandError::NotFound {
            context: format!(
                "config set default-workflow: workflow '{}' не является regular file",
                workflow_id.as_str()
            ),
        }),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            Err(CommandError::NotFound {
                context: format!(
                    "config set default-workflow: workflow '{}' не существует",
                    workflow_id.as_str()
                ),
            })
        }
        Err(source) => Err(CommandError::Runtime {
            context: format!(
                "config set default-workflow: не удалось проверить {}",
                path.display()
            ),
            source,
        }),
    }
}

pub(crate) fn resolve_state_root(
    environment: &ProcessEnvironment,
    command: &str,
) -> Result<PathBuf, CommandError> {
    if let Some(orc_home) = environment
        .orc_home
        .as_ref()
        .filter(|value| !value.is_empty())
    {
        let path = PathBuf::from(orc_home);
        if !path.is_absolute() {
            return Err(CommandError::Invalid {
                context: format!("{command}: ORC_HOME должен быть абсолютным путём"),
            });
        }
        return Ok(path);
    }

    let Some(home) = &environment.home else {
        return Err(CommandError::Invalid {
            context: format!("{command}: HOME не задан и ORC_HOME не переопределён"),
        });
    };
    let home = PathBuf::from(home);
    if !home.is_absolute() {
        return Err(CommandError::Invalid {
            context: format!("{command}: HOME должен разрешаться в абсолютный путь"),
        });
    }
    Ok(home.join(".orc"))
}

pub(crate) fn read_config(
    root: &Path,
    command: &str,
    registry: &dyn AgentRegistry,
) -> Result<ConfigDocument, CommandError> {
    let path = root.join("config.yaml");
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            let raw = RawConfig::default();
            let effective =
                Config::try_from(raw.clone()).map_err(|context| CommandError::Invalid {
                    context: format!("{command}: config: {context}"),
                })?;
            return Ok(ConfigDocument { raw, effective });
        }
        Err(source) => {
            return Err(CommandError::Runtime {
                context: format!("{command}: не удалось прочитать config {}", path.display()),
                source,
            });
        }
    };

    let raw: RawConfig =
        serde_yaml::from_slice(&bytes).map_err(|source| CommandError::Invalid {
            context: format!("{command}: невалидный config {}: {source}", path.display()),
        })?;
    let effective = Config::try_from(raw.clone()).map_err(|context| CommandError::Invalid {
        context: format!("{command}: невалидный config {}: {context}", path.display()),
    })?;
    validate_agents(&raw, registry).map_err(|context| CommandError::Invalid {
        context: format!("{command}: невалидный config {}: {context}", path.display()),
    })?;
    Ok(ConfigDocument { raw, effective })
}

fn publish_config(root: &Path, config: &RawConfig) -> Result<(), CommandError> {
    let bytes = serde_yaml::to_string(config).map_err(|source| CommandError::Invalid {
        context: format!("config set: кандидат нельзя сериализовать: {source}"),
    })?;
    publish_bytes(root, bytes.as_bytes(), || Ok(()))
}

fn publish_bytes(
    root: &Path,
    bytes: &[u8],
    before_commit: impl FnOnce() -> Result<(), std::io::Error>,
) -> Result<(), CommandError> {
    fs::create_dir_all(root).map_err(|source| CommandError::Runtime {
        context: format!("config set: не удалось создать {}", root.display()),
        source,
    })?;
    let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary_path = root.join(format!(
        ".config.yaml.{}.{sequence}.tmp",
        std::process::id()
    ));
    let final_path = root.join("config.yaml");
    let mut temporary = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary_path)
        .map_err(|source| CommandError::Runtime {
            context: format!(
                "config set: не удалось создать временный файл {}",
                temporary_path.display()
            ),
            source,
        })?;

    let operation = (|| {
        temporary.write_all(bytes)?;
        temporary.sync_all()?;
        before_commit()?;
        fs::rename(&temporary_path, &final_path)?;
        File::open(root)?.sync_all()?;
        Ok(())
    })();

    if operation.is_err() {
        drop(temporary);
        let _ = fs::remove_file(&temporary_path);
    }

    operation.map_err(|source| CommandError::Runtime {
        context: format!(
            "config set: не удалось атомарно опубликовать {}",
            final_path.display()
        ),
        source,
    })
}

fn validate_optional_id(field: &str, value: Option<&str>) -> Result<(), String> {
    value.map_or(Ok(()), |value| validate_id(field, value))
}

fn validate_id(kind: &str, value: &str) -> Result<(), String> {
    let valid = !value.is_empty()
        && value.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        });
    if valid {
        Ok(())
    } else {
        Err(format!("{kind} '{value}' не соответствует kebab-case"))
    }
}

fn validate_agents(config: &RawConfig, registry: &dyn AgentRegistry) -> Result<(), String> {
    for (agent_id, agent) in &config.agents {
        registry
            .validate(&agent.r#type, &agent.model, &agent.reasoning)
            .map_err(|context| format!("Agent '{agent_id}': {context}"))?;
    }
    Ok(())
}

fn render_value(config: &Config, key: ConfigKey) -> String {
    match key {
        ConfigKey::DefaultWorkflow => config
            .default_workflow
            .as_deref()
            .unwrap_or("null")
            .to_owned(),
        ConfigKey::DefaultAgent => config.default_agent.as_deref().unwrap_or("null").to_owned(),
        ConfigKey::MaxParallelAgents => config.max_parallel_agents.to_string(),
    }
}

fn render_list(config: &Config) -> String {
    format!(
        "default-workflow: {}\ndefault-agent: {}\nmax-parallel-agents: {}",
        config.default_workflow.as_deref().unwrap_or("null"),
        config.default_agent.as_deref().unwrap_or("null"),
        config.max_parallel_agents
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn symbolic_ids_follow_kebab_case_contract() {
        for valid in ["a", "a1", "one-two", "1-2"] {
            assert!(validate_id("ID", valid).is_ok(), "{valid}");
        }
        for invalid in ["", "-a", "a-", "a--b", "Upper", "with.dot", "кириллица"] {
            assert!(validate_id("ID", invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn config_rejects_unknown_fields() {
        let result = serde_yaml::from_str::<RawConfig>("unknown: true\n");
        assert!(result.is_err());
    }

    #[test]
    fn config_rejects_missing_default_agent() {
        let raw = RawConfig {
            default_agent: Some("missing".to_owned()),
            ..RawConfig::default()
        };
        assert!(Config::try_from(raw).is_err());
    }

    #[test]
    fn default_agent_command_parses_only_symbolic_ids() {
        assert!(ConfigCommand::set_default_agent("codex-main").is_ok());
        assert!(matches!(
            ConfigCommand::set_default_agent("Bad-ID"),
            Err(CommandError::Syntax { .. })
        ));
    }

    #[test]
    fn agent_validation_rejects_unknown_type_and_empty_fields() {
        for agent in [
            RawAgent {
                r#type: "unknown".to_owned(),
                model: "model".to_owned(),
                reasoning: "high".to_owned(),
            },
            RawAgent {
                r#type: "codex".to_owned(),
                model: String::new(),
                reasoning: "high".to_owned(),
            },
            RawAgent {
                r#type: "claude".to_owned(),
                model: "model".to_owned(),
                reasoning: String::new(),
            },
        ] {
            let raw = RawConfig {
                agents: BTreeMap::from([("agent".to_owned(), agent)]),
                ..RawConfig::default()
            };
            assert!(validate_agents(&raw, &BuiltinAgentRegistry).is_err());
        }
    }

    #[test]
    fn publication_failure_before_commit_keeps_previous_config() {
        let root = TempDir::new().expect("test root must be created");
        let path = root.path().join("config.yaml");
        fs::write(&path, b"max-parallel-agents: 7\n").expect("initial config must be written");

        let result = publish_bytes(root.path(), b"max-parallel-agents: 9\n", || {
            Err(std::io::Error::other("injected before commit"))
        });

        assert!(result.is_err());
        assert_eq!(
            fs::read(&path).expect("initial config must remain readable"),
            b"max-parallel-agents: 7\n"
        );
    }
}
