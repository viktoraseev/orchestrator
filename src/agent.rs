//! Граница Agent type проверяет Agent, запускает его и распознаёт возврат; lifecycle не знает CLI или протокол конкретного type.

use std::ffi::OsStr;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;

/// Управляемые recovery-critical события активной обработки attempt.
pub trait AttemptControl {
    /// Durable-фиксирует активацию native session либо успешно игнорирует повтор последней ID.
    ///
    /// См. Rule «Session activation сохраняется в durable-порядке» в `features/lifecycle.feature`.
    ///
    /// # Errors
    ///
    /// Возвращает диагностику, если attempt уже не активен или событие нельзя durable-опубликовать.
    fn activate_session(&mut self, session_id: &str) -> Result<(), String>;

    /// Заменяет volatile-кандидат completion полным набором source paths.
    ///
    /// См. Rule «Completion становится durable только после возврата Agent» в `features/lifecycle.feature`.
    ///
    /// # Errors
    ///
    /// Возвращает диагностику для невалидного набора outputs, path или закрытого attempt context.
    fn complete(&mut self, artifacts: &[(String, PathBuf)]) -> Result<(), String>;
}

/// Один artifact неизменяемого input mapping текущего attempt.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct AgentInput {
    /// Source Step, выбранный dependency activation.
    pub step_id: String,
    /// Объявленный source Step идентификатор artifact.
    pub input_id: String,
    /// Абсолютный durable path выбранной версии artifact.
    pub path: PathBuf,
}

/// Полностью materialized вход одного запуска или native resume Agent attempt.
#[derive(Clone, Debug)]
pub struct AgentRunRequest<'a> {
    /// Materialized Step текущего attempt.
    pub step_id: &'a str,
    /// ID Agent type из materialized Agent.
    pub type_id: &'a str,
    /// Model из materialized Agent.
    pub model: &'a str,
    /// Reasoning из materialized Agent.
    pub reasoning: &'a str,
    /// Сформированный UTF-8 prompt.
    pub prompt: &'a str,
    /// Полный input mapping выбранных dependency attempts.
    pub inputs: &'a [AgentInput],
    /// Последняя durable native session ID для resume либо `None` для создания session.
    pub resume_session: Option<&'a str>,
    /// Durable `RunId` обслуживаемого supervisor.
    pub run_id: &'a str,
    /// Глобальный номер attempt внутри run.
    pub attempt: u64,
    /// Volatile endpoint дочерних control-команд.
    pub control_endpoint: &'a Path,
    /// Общий сигнал прекращения работы после fail-fast соседнего attempt.
    pub cancellation: &'a AgentCancellation,
}

/// Кооперативный сигнал остановки параллельных Agent attempts одного lifecycle-вызова.
#[derive(Clone, Debug, Default)]
pub struct AgentCancellation(Arc<AtomicBool>);

impl AgentCancellation {
    /// Показывает, что supervisor уже наблюдал fail-fast outcome соседнего attempt.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    pub(crate) fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
}

/// Наблюдаемый supervisor результат возврата процесса агента.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AgentExit {
    /// Код возврата процесса; отсутствие Unix exit code отображается adapter-specific runtime error.
    pub code: i32,
}

/// Registry встроенных или тестовых Agent type.
pub trait AgentRegistry: Sync {
    /// Проверяет существование type, совместимость model и reasoning и поддержку native resume.
    ///
    /// # Errors
    ///
    /// Возвращает диагностику несовместимости Agent; caller добавляет контекст config или Step.
    fn validate(&self, type_id: &str, model: &str, reasoning: &str) -> Result<(), String>;

    /// Указывает, что adapter запускает дочерний процесс, которому нужен Unix control endpoint.
    fn uses_process_control(&self) -> bool {
        false
    }

    /// Запускает новую или продолжает последнюю native session текущего attempt.
    ///
    /// Adapter обязан вернуть управление только после возврата agent process и не выводить сырой non-human поток в terminal orchestrator.
    ///
    /// # Errors
    ///
    /// Возвращает adapter-specific runtime error, если процесс нельзя построить, запустить или однозначно интерпретировать.
    fn run(
        &self,
        _request: &AgentRunRequest<'_>,
        _control: &mut dyn AttemptControl,
    ) -> Result<AgentExit, String> {
        Err("Agent type не предоставляет process adapter".to_owned())
    }
}

/// Registry type IDs, встроенных в бинарник orchestrator.
#[derive(Clone, Copy, Debug, Default)]
pub struct BuiltinAgentRegistry;

impl AgentRegistry for BuiltinAgentRegistry {
    fn validate(&self, type_id: &str, model: &str, reasoning: &str) -> Result<(), String> {
        if !matches!(type_id, "codex" | "claude") {
            return Err(format!("неизвестный Agent type '{type_id}'"));
        }
        if model.is_empty() {
            return Err("model не может быть пустым".to_owned());
        }
        if reasoning.is_empty() {
            return Err("reasoning не может быть пустым".to_owned());
        }
        Ok(())
    }
}

/// Process registry встроенных Agent type с единым абсолютным executable override.
#[derive(Clone, Debug)]
pub struct ProcessAgentRegistry {
    command: Option<PathBuf>,
}

impl ProcessAgentRegistry {
    /// Создаёт process registry из абсолютного пути `ORC_AGENT_COMMAND`.
    ///
    /// # Errors
    ///
    /// Возвращает ошибку, если путь не абсолютный.
    pub fn new(command: impl Into<PathBuf>) -> Result<Self, String> {
        let command = command.into();
        if !command.is_absolute() {
            return Err("ORC_AGENT_COMMAND должен быть абсолютным путём".to_owned());
        }
        Ok(Self {
            command: Some(command),
        })
    }

    /// Создаёт registry со стандартным executable каждого встроенного Agent type.
    #[must_use]
    pub const fn system() -> Self {
        Self { command: None }
    }
}

impl AgentRegistry for ProcessAgentRegistry {
    fn validate(&self, type_id: &str, model: &str, reasoning: &str) -> Result<(), String> {
        BuiltinAgentRegistry.validate(type_id, model, reasoning)
    }

    fn uses_process_control(&self) -> bool {
        true
    }

    fn run(
        &self,
        request: &AgentRunRequest<'_>,
        _control: &mut dyn AttemptControl,
    ) -> Result<AgentExit, String> {
        let executable = self
            .command
            .as_deref()
            .unwrap_or_else(|| Path::new(request.type_id));
        let mut command = Command::new(executable);
        command
            .env("ORC_STEP_ID", request.step_id)
            .arg(request.type_id)
            .arg(request.model)
            .arg(request.reasoning)
            .arg(request.prompt)
            .env("ORC_CONTROL_ENDPOINT", request.control_endpoint)
            .env("ORC_RUN_ID", request.run_id)
            .env("ORC_ATTEMPT", request.attempt.to_string());
        let inputs = serde_yaml::to_string(request.inputs)
            .map_err(|error| format!("не удалось сериализовать Agent inputs: {error}"))?;
        command.env("ORC_INPUT", inputs);
        if let Some(session_id) = request.resume_session {
            command.env("ORC_RESUME_SESSION", OsStr::new(session_id));
        }
        command
            .process_group(0)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut child = command.spawn().map_err(|error| {
            format!(
                "не удалось запустить Agent type '{}': {error}",
                request.type_id
            )
        })?;
        let mut termination_deadline = None;
        let status = loop {
            if let Some(status) = child.try_wait().map_err(|error| {
                format!(
                    "не удалось наблюдать Agent type '{}': {error}",
                    request.type_id
                )
            })? {
                break status;
            }
            if request.cancellation.is_cancelled() && termination_deadline.is_none() {
                signal_process_group(child.id(), "TERM")?;
                termination_deadline = Some(Instant::now() + Duration::from_secs(10));
            }
            if termination_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                signal_process_group(child.id(), "KILL")?;
                break child.wait().map_err(|error| {
                    format!(
                        "не удалось дождаться Agent type '{}': {error}",
                        request.type_id
                    )
                })?;
            }
            thread::park_timeout(Duration::from_millis(10));
        };
        status
            .code()
            .map(|code| AgentExit { code })
            .ok_or_else(|| format!("Agent type '{}' завершён сигналом", request.type_id))
    }
}

fn signal_process_group(process_id: u32, signal: &str) -> Result<(), String> {
    let target = format!("-{process_id}");
    let status = Command::new("/bin/kill")
        .args([format!("-{signal}"), target])
        .status()
        .map_err(|error| format!("не удалось послать SIG{signal} process group: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("SIG{signal} process group завершился с {status}"))
    }
}
