//! Граница Agent type проверяет Agent, запускает его и распознаёт возврат; lifecycle не знает CLI или протокол конкретного type.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

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

/// Полностью materialized вход одного запуска или native resume Agent attempt.
#[derive(Clone, Debug)]
pub struct AgentRunRequest<'a> {
    /// ID Agent type из materialized Agent.
    pub type_id: &'a str,
    /// Model из materialized Agent.
    pub model: &'a str,
    /// Reasoning из materialized Agent.
    pub reasoning: &'a str,
    /// Сформированный UTF-8 prompt.
    pub prompt: &'a str,
    /// Последняя durable native session ID для resume либо `None` для создания session.
    pub resume_session: Option<&'a str>,
    /// Durable `RunId` обслуживаемого supervisor.
    pub run_id: &'a str,
    /// Глобальный номер attempt внутри run.
    pub attempt: u64,
    /// Volatile endpoint дочерних control-команд.
    pub control_endpoint: &'a Path,
}

/// Наблюдаемый supervisor результат возврата процесса агента.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AgentExit {
    /// Код возврата процесса; отсутствие Unix exit code отображается adapter-specific runtime error.
    pub code: i32,
}

/// Registry встроенных или тестовых Agent type.
pub trait AgentRegistry {
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
            .arg(request.type_id)
            .arg(request.model)
            .arg(request.reasoning)
            .arg(request.prompt)
            .env("ORC_CONTROL_ENDPOINT", request.control_endpoint)
            .env("ORC_RUN_ID", request.run_id)
            .env("ORC_ATTEMPT", request.attempt.to_string());
        if let Some(session_id) = request.resume_session {
            command.env("ORC_RESUME_SESSION", OsStr::new(session_id));
        }
        let status = command.output().map_err(|error| {
            format!(
                "не удалось запустить Agent type '{}': {error}",
                request.type_id
            )
        })?;
        status
            .status
            .code()
            .map(|code| AgentExit { code })
            .ok_or_else(|| format!("Agent type '{}' завершён сигналом", request.type_id))
    }
}
