//! Граница Agent type проверяет Agent, запускает его и распознаёт возврат; lifecycle не знает CLI или протокол конкретного type.

use std::fs;
use std::io::{BufRead, BufReader};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Управляемые recovery-critical события активной обработки attempt.
pub trait AttemptControl: Send {
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

/// Получатель volatile-событий представления активных Agent sessions.
pub trait AgentSessionObserver: Send + Sync {
    /// Добавляет активный attempt; `human` временно скрывает board остальных attempts.
    fn started(&self, attempt: u64, step_id: &str, human: bool);

    /// Публикует одно нормализованное сообщение non-human Agent.
    fn message(&self, attempt: u64, step_id: &str, message: &str);

    /// Удаляет вернувшийся attempt из volatile-представления.
    fn finished(&self, attempt: u64, step_id: &str, human: bool);
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
    /// Human attempt напрямую наследует terminal lifecycle-команды.
    pub human: bool,
}

#[derive(Debug, Default)]
struct CancellationState {
    signal: AtomicU8,
    escalate: AtomicBool,
}

/// Кооперативный сигнал остановки параллельных Agent attempts одного lifecycle-вызова.
#[derive(Clone, Debug, Default)]
pub struct AgentCancellation(Arc<CancellationState>);

impl AgentCancellation {
    /// Показывает, что supervisor уже наблюдал fail-fast outcome соседнего attempt.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.signal().is_some()
    }

    pub(crate) fn cancel(&self) {
        self.cancel_with(TerminationSignal::Terminate);
    }

    pub(crate) fn cancel_with(&self, signal: TerminationSignal) {
        let _ =
            self.0
                .signal
                .compare_exchange(0, signal.number(), Ordering::AcqRel, Ordering::Acquire);
    }

    pub(crate) fn escalate(&self) {
        self.0.escalate.store(true, Ordering::Release);
    }

    fn signal(&self) -> Option<TerminationSignal> {
        TerminationSignal::from_number(self.0.signal.load(Ordering::Acquire))
    }

    fn should_escalate(&self) -> bool {
        self.0.escalate.load(Ordering::Acquire)
    }
}

/// Поддерживаемый lifecycle termination signal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminationSignal {
    /// Закрытие управляющего терминала (`SIGHUP`).
    Hangup,
    /// Прерывание пользователя (`SIGINT`).
    Interrupt,
    /// Запрос штатного завершения (`SIGTERM`).
    Terminate,
}

impl TerminationSignal {
    /// Номер Unix signal.
    #[must_use]
    pub const fn number(self) -> u8 {
        match self {
            Self::Hangup => 1,
            Self::Interrupt => 2,
            Self::Terminate => 15,
        }
    }

    /// Документированный lifecycle exit code `128 + signal`.
    #[must_use]
    pub const fn exit_code(self) -> u8 {
        128 + self.number()
    }

    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Hangup => "HUP",
            Self::Interrupt => "INT",
            Self::Terminate => "TERM",
        }
    }

    pub(crate) const fn from_number(number: u8) -> Option<Self> {
        match number {
            1 => Some(Self::Hangup),
            2 => Some(Self::Interrupt),
            15 => Some(Self::Terminate),
            _ => None,
        }
    }
}

/// Распознанный Agent type результат возврата процесса агента.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentExit {
    /// Обычный возврат процесса с наблюдаемым exit code.
    Returned(i32),
    /// Human Agent распознал явную команду пользователя `/exit`.
    UserExit,
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
        process_agent_type(type_id)?;
        if model.is_empty() {
            return Err("model не может быть пустым".to_owned());
        }
        if !matches!(reasoning, "low" | "medium" | "high" | "xhigh" | "max") {
            return Err(format!("reasoning '{reasoning}' не поддерживается"));
        }
        Ok(())
    }
}

/// Process registry встроенных Agent type с единым абсолютным executable override.
#[derive(Clone)]
pub struct ProcessAgentRegistry {
    command: Option<PathBuf>,
    observer: Option<Arc<dyn AgentSessionObserver>>,
}

impl ProcessAgentRegistry {
    /// Создаёт process registry из абсолютного пути `ORC_AGENT_COMMAND`.
    ///
    /// # Errors
    ///
    /// Возвращает ошибку, если путь не абсолютный либо не указывает на executable regular file.
    pub fn new(command: impl Into<PathBuf>) -> Result<Self, String> {
        let command = command.into();
        if !command.is_absolute() {
            return Err("ORC_AGENT_COMMAND должен быть абсолютным путём".to_owned());
        }
        let metadata = fs::metadata(&command).map_err(|error| {
            format!(
                "ORC_AGENT_COMMAND '{}' недоступен: {error}",
                command.display()
            )
        })?;
        if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
            return Err(format!(
                "ORC_AGENT_COMMAND '{}' должен быть executable regular file",
                command.display()
            ));
        }
        Ok(Self {
            command: Some(command),
            observer: None,
        })
    }

    /// Создаёт registry со стандартным executable каждого встроенного Agent type.
    #[must_use]
    pub const fn system() -> Self {
        Self {
            command: None,
            observer: None,
        }
    }

    /// Подключает volatile observer, которому adapters передают только нормализованные события.
    #[must_use]
    pub fn with_session_observer(mut self, observer: Arc<dyn AgentSessionObserver>) -> Self {
        self.observer = Some(observer);
        self
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
        control: &mut dyn AttemptControl,
    ) -> Result<AgentExit, String> {
        let agent_type = process_agent_type(request.type_id)?;
        let executable = self
            .command
            .as_deref()
            .unwrap_or_else(|| Path::new(agent_type.executable()));
        let mut command = Command::new(executable);
        command
            .env("ORC_STEP_ID", request.step_id)
            .env("ORC_CONTROL_ENDPOINT", request.control_endpoint)
            .env("ORC_RUN_ID", request.run_id)
            .env("ORC_ATTEMPT", request.attempt.to_string());
        let inputs = serde_yaml::to_string(request.inputs)
            .map_err(|error| format!("не удалось сериализовать Agent inputs: {error}"))?;
        command.env("ORC_INPUT", inputs);
        if let Some(session_id) = request.resume_session {
            command.env("ORC_RESUME_SESSION", session_id);
        }
        agent_type.configure(&mut command, request);
        command.process_group(0);
        if request.human {
            command
                .stdin(Stdio::inherit())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit());
        } else {
            command
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null());
        }
        let mut child = command.spawn().map_err(|error| {
            format!(
                "не удалось запустить Agent type '{}': {error}",
                request.type_id
            )
        })?;
        let _observed_attempt = self.observer.as_deref().map(|observer| {
            observer.started(request.attempt, request.step_id, request.human);
            ObservedAttempt {
                observer,
                attempt: request.attempt,
                step_id: request.step_id,
                human: request.human,
            }
        });
        let observer = self.observer.as_deref();
        let status = if request.human {
            wait_for_process(&mut child, request)?
        } else {
            let stdout = child
                .stdout
                .take()
                .ok_or_else(|| format!("Agent type '{}' не предоставил stdout", request.type_id))?;
            thread::scope(|scope| {
                let reader = scope
                    .spawn(|| consume_protocol(agent_type, stdout, request, control, observer));
                let process_result = wait_for_process(&mut child, request);
                let protocol_result = reader.join().map_err(|_| {
                    format!("Agent type '{}' protocol reader panic", request.type_id)
                })?;
                protocol_result?;
                process_result
            })?
        };
        status
            .code()
            .map(AgentExit::Returned)
            .ok_or_else(|| format!("Agent type '{}' завершён сигналом", request.type_id))
    }
}

struct ObservedAttempt<'a> {
    observer: &'a dyn AgentSessionObserver,
    attempt: u64,
    step_id: &'a str,
    human: bool,
}

impl Drop for ObservedAttempt<'_> {
    fn drop(&mut self) {
        self.observer
            .finished(self.attempt, self.step_id, self.human);
    }
}

#[derive(Debug)]
enum AgentProtocolEvent {
    SessionStarted(String),
    Message(String),
}

trait ProcessAgentType: Sync {
    fn executable(&self) -> &'static str;
    fn configure(&self, command: &mut Command, request: &AgentRunRequest<'_>);
    fn parse_line(&self, line: &str) -> Result<Option<AgentProtocolEvent>, String>;
}

#[derive(Debug)]
struct CodexAgentType;

#[derive(Debug)]
struct ClaudeAgentType;

static CODEX_AGENT_TYPE: CodexAgentType = CodexAgentType;
static CLAUDE_AGENT_TYPE: ClaudeAgentType = ClaudeAgentType;

fn process_agent_type(type_id: &str) -> Result<&'static dyn ProcessAgentType, String> {
    match type_id {
        "codex" => Ok(&CODEX_AGENT_TYPE),
        "claude" => Ok(&CLAUDE_AGENT_TYPE),
        _ => Err(format!("неизвестный Agent type '{type_id}'")),
    }
}

impl ProcessAgentType for CodexAgentType {
    fn executable(&self) -> &'static str {
        "codex"
    }

    fn configure(&self, command: &mut Command, request: &AgentRunRequest<'_>) {
        let reasoning = format!("model_reasoning_effort=\"{}\"", request.reasoning);
        if request.human {
            if let Some(session_id) = request.resume_session {
                command.arg("resume");
                command.args(["--model", request.model, "--config", &reasoning]);
                command.args([session_id, request.prompt]);
            } else {
                command.args(["--model", request.model, "--config", &reasoning]);
                command.arg(request.prompt);
            }
        } else {
            command.arg("exec");
            if request.resume_session.is_some() {
                command.arg("resume");
            }
            command.args(["--json", "--model", request.model, "--config", &reasoning]);
            if let Some(session_id) = request.resume_session {
                command.arg(session_id);
            }
            command.arg(request.prompt);
        }
    }

    fn parse_line(&self, line: &str) -> Result<Option<AgentProtocolEvent>, String> {
        let event: CodexEvent = serde_json::from_str(line)
            .map_err(|error| format!("codex protocol содержит невалидный JSON: {error}"))?;
        Ok(match event {
            CodexEvent::ThreadStarted { thread_id } => {
                Some(AgentProtocolEvent::SessionStarted(thread_id))
            }
            CodexEvent::ItemCompleted {
                item: CodexItem::AgentMessage { text },
            } => Some(AgentProtocolEvent::Message(text)),
            CodexEvent::ItemCompleted {
                item: CodexItem::Other,
            }
            | CodexEvent::Other => None,
        })
    }
}

impl ProcessAgentType for ClaudeAgentType {
    fn executable(&self) -> &'static str {
        "claude"
    }

    fn configure(&self, command: &mut Command, request: &AgentRunRequest<'_>) {
        if !request.human {
            command.args(["--print", "--output-format", "stream-json", "--verbose"]);
        }
        command.args(["--model", request.model, "--effort", request.reasoning]);
        if let Some(session_id) = request.resume_session {
            command.args(["--resume", session_id]);
        }
        command.arg(request.prompt);
    }

    fn parse_line(&self, line: &str) -> Result<Option<AgentProtocolEvent>, String> {
        let event: ClaudeEvent = serde_json::from_str(line)
            .map_err(|error| format!("claude protocol содержит невалидный JSON: {error}"))?;
        Ok(match event {
            ClaudeEvent::System {
                subtype: ClaudeSystemSubtype::Init,
                session_id: Some(session_id),
            } => Some(AgentProtocolEvent::SessionStarted(session_id)),
            ClaudeEvent::Assistant { message } => {
                let text = message
                    .content
                    .into_iter()
                    .filter_map(|block| match block {
                        ClaudeContent::Text { text } => Some(text),
                        ClaudeContent::Other => None,
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                (!text.is_empty()).then_some(AgentProtocolEvent::Message(text))
            }
            ClaudeEvent::System { .. } | ClaudeEvent::Other => None,
        })
    }
}

fn consume_protocol(
    agent_type: &'static dyn ProcessAgentType,
    stdout: ChildStdout,
    request: &AgentRunRequest<'_>,
    control: &mut dyn AttemptControl,
    observer: Option<&dyn AgentSessionObserver>,
) -> Result<(), String> {
    let mut session_id: Option<String> = None;
    let mut failure = None;
    for line in BufReader::new(stdout).lines() {
        let result = line
            .map_err(|error| format!("не удалось прочитать Agent protocol: {error}"))
            .and_then(|line| agent_type.parse_line(&line));
        let event = match result {
            Ok(event) if failure.is_none() => event,
            Ok(_) => continue,
            Err(error) => {
                failure.get_or_insert(error);
                continue;
            }
        };
        let event_result = match event {
            Some(AgentProtocolEvent::SessionStarted(candidate)) => {
                if session_id
                    .as_deref()
                    .is_some_and(|current| current != candidate)
                {
                    Err(format!(
                        "Agent type '{}' сменил session ID внутри одного process",
                        request.type_id
                    ))
                } else if request
                    .resume_session
                    .is_some_and(|expected| expected != candidate)
                {
                    Err(format!(
                        "Agent type '{}' возобновил другую native session",
                        request.type_id
                    ))
                } else {
                    control.activate_session(&candidate)?;
                    session_id = Some(candidate);
                    Ok(())
                }
            }
            Some(AgentProtocolEvent::Message(message)) => {
                if let Some(observer) = observer {
                    let message = normalize_message(&message);
                    observer.message(request.attempt, request.step_id, &message);
                }
                Ok(())
            }
            None => continue,
        };
        if let Err(error) = event_result {
            failure.get_or_insert(error);
        }
    }
    if let Some(error) = failure {
        return Err(error);
    }
    session_id.ok_or_else(|| {
        format!(
            "Agent type '{}' не сообщил обязательный native session ID",
            request.type_id
        )
    })?;
    Ok(())
}

fn normalize_message(message: &str) -> String {
    message.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn wait_for_process(
    child: &mut Child,
    request: &AgentRunRequest<'_>,
) -> Result<ExitStatus, String> {
    let mut termination_deadline = None;
    loop {
        if let Some(status) = child.try_wait().map_err(|error| {
            format!(
                "не удалось наблюдать Agent type '{}': {error}",
                request.type_id
            )
        })? {
            return Ok(status);
        }
        if let Some(signal) = request.cancellation.signal()
            && termination_deadline.is_none()
        {
            signal_process_group(child.id(), signal.name())?;
            termination_deadline = Some(Instant::now() + Duration::from_secs(10));
        }
        if request.cancellation.should_escalate()
            || termination_deadline.is_some_and(|deadline| Instant::now() >= deadline)
        {
            signal_process_group(child.id(), "KILL")?;
            return child.wait().map_err(|error| {
                format!(
                    "не удалось дождаться Agent type '{}': {error}",
                    request.type_id
                )
            });
        }
        thread::park_timeout(Duration::from_millis(10));
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum CodexEvent {
    #[serde(rename = "thread.started")]
    ThreadStarted { thread_id: String },
    #[serde(rename = "item.completed")]
    ItemCompleted { item: CodexItem },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum CodexItem {
    #[serde(rename = "agent_message")]
    AgentMessage { text: String },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum ClaudeEvent {
    #[serde(rename = "system")]
    System {
        subtype: ClaudeSystemSubtype,
        #[serde(default)]
        session_id: Option<String>,
    },
    #[serde(rename = "assistant")]
    Assistant { message: ClaudeMessage },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ClaudeSystemSubtype {
    Init,
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
struct ClaudeMessage {
    content: Vec<ClaudeContent>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum ClaudeContent {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(other)]
    Other,
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
