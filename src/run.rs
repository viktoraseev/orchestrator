//! Lifecycle run, storage и control boundary; модуль не разбирает CLI и не знает протокол конкретного Agent type.

use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use fs2::FileExt;
use serde::{Deserialize, Serialize};

use crate::agent::{AgentRegistry, AgentRunRequest, AttemptControl};
use crate::config::{CommandError, ProcessEnvironment, RawAgent, resolve_state_root};
use crate::workflow::{ValidateCommand, Workflow, materialize_for_lifecycle};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Команда публичного lifecycle API после разбора CLI.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LifecycleCommand {
    /// Создать новый run выбранного workflow.
    Start(ValidateCommand),
    /// Продолжить существующий run с обязательным ID.
    Resume(RunId),
}

impl LifecycleCommand {
    /// Создаёт `start` с явно выбранным workflow.
    ///
    /// # Errors
    ///
    /// Возвращает [`CommandError::Syntax`], если `WorkflowId` не соответствует kebab-case.
    pub fn start_explicit(value: &str) -> Result<Self, CommandError> {
        ValidateCommand::explicit(value)
            .map(Self::Start)
            .map_err(|error| match error {
                CommandError::Syntax { context } => CommandError::Syntax {
                    context: context.replacen("validate:", "start:", 1),
                },
                other => other,
            })
    }

    /// Создаёт `start` с workflow из config default.
    #[must_use]
    pub const fn start_configured_default() -> Self {
        Self::Start(ValidateCommand::ConfiguredDefault)
    }
}

/// Проверенный десятичный идентификатор run.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RunId(u64);

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

/// Приёмник стабильных lifecycle-сообщений.
pub trait LifecycleReporter {
    /// Публикует и немедленно делает наблюдаемой одну полную строку.
    ///
    /// `start` вызывает reporter после durable initial attempt и до запуска Agent, как задано Rule «Start обещает только durable run» в `features/lifecycle.feature`.
    ///
    /// # Errors
    ///
    /// Возвращает ошибку вывода; после неё lifecycle не запускает новую работу.
    fn line(&mut self, value: &str) -> Result<(), std::io::Error>;
}

/// Выполняет `start` или `resume` через одну lifecycle, storage и `AgentType` границу.
///
/// См. Rules в `features/lifecycle.feature`; storage commit attempt состоит в атомарной замене record, а `resume` читает только полностью опубликованную durable-модель.
///
/// # Errors
///
/// Возвращает категории 2/3/4 до создания run, 5 для занятого run и 1 для I/O, Agent failure или возврата без completion.
pub fn execute_lifecycle(
    command: &LifecycleCommand,
    environment: &ProcessEnvironment,
    registry: &dyn AgentRegistry,
    reporter: &mut dyn LifecycleReporter,
) -> Result<(), CommandError> {
    match command {
        LifecycleCommand::Start(selection) => start(selection, environment, registry, reporter),
        LifecycleCommand::Resume(run_id) => resume(*run_id, environment, registry, reporter),
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct MaterializedWorkflow {
    workflow_id: String,
    max_parallel_agents: usize,
    steps: Vec<MaterializedStep>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct MaterializedStep {
    id: String,
    agent: RawAgent,
    prompt: Option<String>,
    human: bool,
    depends_on: Vec<String>,
    outputs: Vec<String>,
}

impl From<Workflow> for MaterializedWorkflow {
    fn from(workflow: Workflow) -> Self {
        Self {
            workflow_id: workflow.id.as_str().to_owned(),
            max_parallel_agents: workflow.max_parallel_agents,
            steps: workflow
                .steps
                .into_iter()
                .map(|step| MaterializedStep {
                    id: step.id.as_str().to_owned(),
                    agent: step.agent,
                    prompt: step.prompt,
                    human: step.human,
                    depends_on: step
                        .depends_on
                        .into_iter()
                        .map(|id| id.as_str().to_owned())
                        .collect(),
                    outputs: step
                        .outputs
                        .into_iter()
                        .map(|id| id.as_str().to_owned())
                        .collect(),
                })
                .collect(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct AttemptRecord {
    input: Vec<u64>,
    events: Vec<AttemptEvent>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
enum AttemptEvent {
    SessionActivated {
        #[serde(rename = "session-id")]
        session_id: String,
    },
    Completed,
}

impl AttemptRecord {
    fn is_completed(&self) -> bool {
        matches!(self.events.last(), Some(AttemptEvent::Completed))
    }

    fn last_session(&self) -> Option<&str> {
        self.events.iter().rev().find_map(|event| match event {
            AttemptEvent::SessionActivated { session_id } => Some(session_id.as_str()),
            AttemptEvent::Completed => None,
        })
    }
}

struct RunGuard {
    directory: PathBuf,
    _lock: File,
}

impl RunGuard {
    fn acquire(directory: PathBuf, context: &str) -> Result<Self, CommandError> {
        let lock_path = directory.join("active.lock");
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|source| {
                runtime(
                    context,
                    format!("не удалось открыть {}", lock_path.display()),
                    source,
                )
            })?;
        FileExt::try_lock_exclusive(&lock).map_err(|source| {
            if source.kind() == std::io::ErrorKind::WouldBlock {
                CommandError::Busy {
                    context: format!("{context}: run занят другим supervisor"),
                }
            } else {
                runtime(
                    context,
                    format!("не удалось получить lock {}", lock_path.display()),
                    source,
                )
            }
        })?;
        Ok(Self {
            directory,
            _lock: lock,
        })
    }
}

fn start(
    selection: &ValidateCommand,
    environment: &ProcessEnvironment,
    registry: &dyn AgentRegistry,
    reporter: &mut dyn LifecycleReporter,
) -> Result<(), CommandError> {
    let root = resolve_state_root(environment, "start")?;
    let candidate = MaterializedWorkflow::from(materialize_for_lifecycle(
        selection,
        environment,
        registry,
        "start",
    )?);
    let (run_id, guard) = reserve_run(&root)?;
    publish_yaml(&guard.directory, "spec.yaml", &candidate, "start")?;
    let first = candidate
        .steps
        .first()
        .ok_or_else(|| CommandError::Invalid {
            context: "start: materialized workflow не содержит Steps".to_owned(),
        })?;
    let record = AttemptRecord {
        input: Vec::new(),
        events: Vec::new(),
    };
    publish_yaml(
        &guard.directory,
        &attempt_name(0, &first.id),
        &record,
        "start",
    )?;
    report(
        reporter,
        &format!("workflow: {}", candidate.workflow_id),
        "start",
    )?;
    report(reporter, &format!("Run {run_id}"), "start")?;
    let result = run_current_attempt(&guard, run_id, &candidate, 0, registry, &record).and_then(
        |completed| {
            if completed {
                report(reporter, &format!("run {run_id}: completed"), "start")?;
                Ok(())
            } else {
                Err(CommandError::Runtime {
                    context: format!("start: run {run_id}: Agent вернул управление без completion"),
                    source: std::io::Error::other("attempt остался незавершённым"),
                })
            }
        },
    );
    let exit_result = report(reporter, &format!("Run {run_id} exited"), "start");
    result.and(exit_result)
}

fn resume(
    run_id: RunId,
    environment: &ProcessEnvironment,
    registry: &dyn AgentRegistry,
    reporter: &mut dyn LifecycleReporter,
) -> Result<(), CommandError> {
    let root = resolve_state_root(environment, "resume")?;
    let directory = root.join("run").join(run_id.to_string());
    if !directory.is_dir() {
        return Err(CommandError::NotFound {
            context: format!("resume: run {run_id} не существует"),
        });
    }
    let guard = RunGuard::acquire(directory, &format!("resume: run {run_id}"))?;
    let candidate: MaterializedWorkflow = read_yaml(&guard.directory.join("spec.yaml"), "resume")?;
    validate_materialized(&candidate, registry, run_id)?;
    let first = candidate
        .steps
        .first()
        .ok_or_else(|| invalid_run(run_id, "spec не содержит Steps"))?;
    let record: AttemptRecord =
        read_yaml(&guard.directory.join(attempt_name(0, &first.id)), "resume")?;
    validate_record(&record, first, 0, &guard.directory, run_id)?;
    let result = if record.is_completed() {
        report(
            reporter,
            &format!("run {run_id}: already completed"),
            "resume",
        )
    } else {
        run_current_attempt(&guard, run_id, &candidate, 0, registry, &record).and_then(
            |completed| {
                if completed {
                    report(reporter, &format!("run {run_id}: completed"), "resume")
                } else {
                    Err(CommandError::Runtime {
                        context: format!(
                            "resume: run {run_id}: Agent вернул управление без completion"
                        ),
                        source: std::io::Error::other("attempt остался незавершённым"),
                    })
                }
            },
        )
    };
    let exit_result = report(reporter, &format!("Run {run_id} exited"), "resume");
    result.and(exit_result)
}

fn run_current_attempt(
    guard: &RunGuard,
    run_id: RunId,
    workflow: &MaterializedWorkflow,
    attempt: u64,
    registry: &dyn AgentRegistry,
    record: &AttemptRecord,
) -> Result<bool, CommandError> {
    let step = workflow
        .steps
        .first()
        .ok_or_else(|| invalid_run(run_id, "spec не содержит Steps"))?;
    if step.human {
        return Err(CommandError::Runtime {
            context: format!("run {run_id}: human attempt нельзя запустить без TTY"),
            source: std::io::Error::other("TTY недоступен"),
        });
    }
    let state = Arc::new(Mutex::new(ControlState {
        active: true,
        run_id,
        attempt,
        run_directory: guard.directory.clone(),
        step_id: step.id.clone(),
        outputs: step.outputs.clone(),
        record: record.clone(),
        candidate: None,
    }));
    let server = registry
        .uses_process_control()
        .then(|| ControlServer::start(&guard.directory, run_id, Arc::clone(&state)))
        .transpose()?;
    let endpoint = server
        .as_ref()
        .map_or_else(PathBuf::new, |server| server.path.clone());
    let mut control = ControlHandle {
        state: Arc::clone(&state),
    };
    let prompt = step.prompt.as_deref().unwrap_or("");
    let result = registry.run(
        &AgentRunRequest {
            type_id: &step.agent.r#type,
            model: &step.agent.model,
            reasoning: &step.agent.reasoning,
            prompt,
            resume_session: record.last_session(),
            run_id: &run_id.to_string(),
            attempt,
            control_endpoint: &endpoint,
        },
        &mut control,
    );
    if let Some(server) = server {
        server.stop()?;
    }
    let exit = result.map_err(|context| CommandError::Runtime {
        context: format!("run {run_id}: {context}"),
        source: std::io::Error::other("Agent adapter failure"),
    })?;
    let candidate = {
        let mut state = lock_state(&state)?;
        state.active = false;
        state.candidate.take()
    };
    let completed = if let Some(candidate) = candidate {
        finalize_completion(&guard.directory, attempt, step, candidate)?;
        true
    } else {
        false
    };
    if exit.code != 0 {
        return Err(CommandError::Runtime {
            context: format!("run {run_id}: Agent завершился с кодом {}", exit.code),
            source: std::io::Error::other("Agent process failure"),
        });
    }
    Ok(completed)
}

fn reserve_run(root: &Path) -> Result<(RunId, RunGuard), CommandError> {
    let run_root = root.join("run");
    fs::create_dir_all(&run_root).map_err(|source| {
        runtime(
            "start",
            format!("не удалось создать {}", run_root.display()),
            source,
        )
    })?;
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|source| CommandError::Invalid {
            context: format!("start: системное время раньше Unix epoch: {source}"),
        })?
        .as_millis();
    let initial = u64::try_from(millis).map_err(|_| CommandError::Invalid {
        context: "start: timestamp RunId не помещается в u64".to_owned(),
    })?;
    let mut candidate = initial;
    loop {
        let id = RunId(candidate);
        let directory = run_root.join(id.to_string());
        match fs::create_dir(&directory) {
            Ok(()) => {
                fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).map_err(
                    |source| {
                        runtime(
                            "start",
                            format!("не удалось защитить {}", directory.display()),
                            source,
                        )
                    },
                )?;
                let guard = RunGuard::acquire(directory, &format!("start: run {id}"))?;
                return Ok((id, guard));
            }
            Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {
                candidate = candidate
                    .checked_add(1)
                    .ok_or_else(|| CommandError::Runtime {
                        context: "start: исчерпано пространство RunId".to_owned(),
                        source: std::io::Error::other("RunId overflow"),
                    })?;
            }
            Err(source) => {
                return Err(runtime(
                    "start",
                    format!("не удалось зарезервировать {}", directory.display()),
                    source,
                ));
            }
        }
    }
}

#[derive(Debug)]
struct CompletionCandidate(BTreeMap<String, Vec<u8>>);

struct ControlState {
    active: bool,
    run_id: RunId,
    attempt: u64,
    run_directory: PathBuf,
    step_id: String,
    outputs: Vec<String>,
    record: AttemptRecord,
    candidate: Option<CompletionCandidate>,
}

struct ControlHandle {
    state: Arc<Mutex<ControlState>>,
}

impl AttemptControl for ControlHandle {
    fn activate_session(&mut self, session_id: &str) -> Result<(), String> {
        activate_session(&self.state, session_id).map_err(|error| error.to_string())
    }

    fn complete(&mut self, artifacts: &[(String, PathBuf)]) -> Result<(), String> {
        accept_completion(&self.state, artifacts).map_err(|error| error.to_string())
    }
}

fn activate_session(
    state: &Arc<Mutex<ControlState>>,
    session_id: &str,
) -> Result<(), CommandError> {
    let mut state = lock_state(state)?;
    ensure_active(&state)?;
    if state.record.last_session() == Some(session_id) {
        return Ok(());
    }
    state.record.events.push(AttemptEvent::SessionActivated {
        session_id: session_id.to_owned(),
    });
    publish_yaml(
        &state.run_directory,
        &attempt_name(state.attempt, &state.step_id),
        &state.record,
        "session activate",
    )
}

fn accept_completion(
    state: &Arc<Mutex<ControlState>>,
    artifacts: &[(String, PathBuf)],
) -> Result<(), CommandError> {
    let mut state = lock_state(state)?;
    ensure_active(&state)?;
    let expected: HashSet<&str> = state.outputs.iter().map(String::as_str).collect();
    let mut found = HashSet::with_capacity(artifacts.len());
    let mut bytes = BTreeMap::new();
    for (input_id, path) in artifacts {
        if !found.insert(input_id.as_str()) || !expected.contains(input_id.as_str()) {
            return Err(CommandError::Invalid {
                context: format!(
                    "attempt complete: невалидный или повторяющийся InputId '{input_id}'"
                ),
            });
        }
        if !path.is_absolute() {
            return Err(CommandError::Invalid {
                context: format!("attempt complete: path {} не абсолютный", path.display()),
            });
        }
        let metadata = fs::metadata(path).map_err(|source| CommandError::Invalid {
            context: format!(
                "attempt complete: artifact {} недоступен: {source}",
                path.display()
            ),
        })?;
        if !metadata.is_file() {
            return Err(CommandError::Invalid {
                context: format!(
                    "attempt complete: artifact {} не является regular file",
                    path.display()
                ),
            });
        }
        let content = fs::read(path).map_err(|source| {
            runtime(
                "attempt complete",
                format!("не удалось прочитать {}", path.display()),
                source,
            )
        })?;
        bytes.insert(input_id.clone(), content);
    }
    if found.len() != expected.len() {
        return Err(CommandError::Invalid {
            context: "attempt complete: набор InputIds не совпадает с outputs Step".to_owned(),
        });
    }
    state.candidate = Some(CompletionCandidate(bytes));
    Ok(())
}

fn finalize_completion(
    directory: &Path,
    attempt: u64,
    step: &MaterializedStep,
    candidate: CompletionCandidate,
) -> Result<(), CommandError> {
    for (input_id, bytes) in candidate.0 {
        publish_bytes(
            directory,
            &format!("{attempt}.{}.{input_id}.artifact", step.id),
            &bytes,
            "attempt complete",
        )?;
    }
    let path = directory.join(attempt_name(attempt, &step.id));
    let mut record: AttemptRecord = read_yaml(&path, "attempt complete")?;
    record.events.push(AttemptEvent::Completed);
    publish_yaml(
        directory,
        &attempt_name(attempt, &step.id),
        &record,
        "attempt complete",
    )
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
enum ControlRequest {
    SessionActivate {
        run_id: u64,
        attempt: u64,
        session_id: String,
    },
    AttemptComplete {
        run_id: u64,
        attempt: u64,
        artifacts: Vec<(String, PathBuf)>,
    },
    Shutdown,
}

#[derive(Debug, Deserialize, Serialize)]
struct ControlResponse {
    code: u8,
    message: String,
}

struct ControlServer {
    path: PathBuf,
    thread: thread::JoinHandle<Result<(), CommandError>>,
}

impl ControlServer {
    fn start(
        _directory: &Path,
        run_id: RunId,
        state: Arc<Mutex<ControlState>>,
    ) -> Result<Self, CommandError> {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let socket_root = if Path::new("/private/tmp").is_dir() {
            Path::new("/private/tmp")
        } else {
            Path::new("/tmp")
        };
        let path = socket_root.join(format!(
            "orc-control-{}-{sequence}.sock",
            std::process::id()
        ));
        let listener = UnixListener::bind(&path).map_err(|source| {
            runtime(
                "run",
                format!("не удалось создать control endpoint {}", path.display()),
                source,
            )
        })?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).map_err(|source| {
            runtime(
                "run",
                format!("не удалось защитить control endpoint {}", path.display()),
                source,
            )
        })?;
        let thread = thread::spawn(move || serve_control(listener, run_id, state));
        Ok(Self { path, thread })
    }

    fn stop(self) -> Result<(), CommandError> {
        let mut stream = UnixStream::connect(&self.path).map_err(|source| {
            runtime(
                "run",
                "не удалось закрыть control endpoint".to_owned(),
                source,
            )
        })?;
        write_frame(&mut stream, &ControlRequest::Shutdown).map_err(|source| {
            runtime(
                "run",
                "не удалось отправить shutdown control endpoint".to_owned(),
                source,
            )
        })?;
        let result = self.thread.join().map_err(|_| CommandError::Runtime {
            context: "run: control endpoint завершился panic".to_owned(),
            source: std::io::Error::other("control thread panic"),
        })?;
        let remove_result = fs::remove_file(&self.path).map_err(|source| {
            runtime(
                "run",
                format!(
                    "не удалось удалить control endpoint {}",
                    self.path.display()
                ),
                source,
            )
        });
        result?;
        remove_result
    }
}

#[allow(clippy::needless_pass_by_value)]
fn serve_control(
    listener: UnixListener,
    run_id: RunId,
    state: Arc<Mutex<ControlState>>,
) -> Result<(), CommandError> {
    for incoming in listener.incoming() {
        let mut stream = incoming
            .map_err(|source| runtime("control", "не удалось принять запрос".to_owned(), source))?;
        let request: ControlRequest = read_frame(&mut stream).map_err(|source| {
            runtime("control", "не удалось прочитать запрос".to_owned(), source)
        })?;
        if matches!(request, ControlRequest::Shutdown) {
            return Ok(());
        }
        let result = handle_request(&state, run_id, request);
        let response = match result {
            Ok(()) => ControlResponse {
                code: 0,
                message: String::new(),
            },
            Err(error) => ControlResponse {
                code: error.exit_code(),
                message: error.to_string(),
            },
        };
        write_frame(&mut stream, &response)
            .map_err(|source| runtime("control", "не удалось ответить".to_owned(), source))?;
    }
    Ok(())
}

fn handle_request(
    state: &Arc<Mutex<ControlState>>,
    served_run: RunId,
    request: ControlRequest,
) -> Result<(), CommandError> {
    match request {
        ControlRequest::SessionActivate {
            run_id,
            attempt,
            session_id,
        } => {
            validate_context(state, served_run, run_id, attempt)?;
            activate_session(state, &session_id)
        }
        ControlRequest::AttemptComplete {
            run_id,
            attempt,
            artifacts,
        } => {
            validate_context(state, served_run, run_id, attempt)?;
            accept_completion(state, &artifacts)
        }
        ControlRequest::Shutdown => Ok(()),
    }
}

fn validate_context(
    state: &Arc<Mutex<ControlState>>,
    served_run: RunId,
    run_id: u64,
    attempt: u64,
) -> Result<(), CommandError> {
    let state = lock_state(state)?;
    if served_run.0 != run_id
        || state.run_id.0 != run_id
        || state.attempt != attempt
        || !state.active
    {
        return Err(CommandError::Busy {
            context: "control: context текущей обработки attempt закрыт".to_owned(),
        });
    }
    Ok(())
}

/// Отправляет `session activate` parent supervisor из доверенного process context.
///
/// # Errors
///
/// Возвращает 5 для недоступного endpoint/context и ответ parent для невалидного события.
pub fn send_session_activation(
    endpoint: &Path,
    run_id: RunId,
    attempt: u64,
    session_id: String,
) -> Result<(), CommandError> {
    send_control(
        endpoint,
        &ControlRequest::SessionActivate {
            run_id: run_id.0,
            attempt,
            session_id,
        },
        "session activate",
    )
}

/// Отправляет `attempt complete` parent supervisor из доверенного process context.
///
/// # Errors
///
/// Возвращает 5 для недоступного endpoint/context и ответ parent для невалидного набора artifacts.
pub fn send_attempt_completion(
    endpoint: &Path,
    run_id: RunId,
    attempt: u64,
    artifacts: Vec<(String, PathBuf)>,
) -> Result<(), CommandError> {
    send_control(
        endpoint,
        &ControlRequest::AttemptComplete {
            run_id: run_id.0,
            attempt,
            artifacts,
        },
        "attempt complete",
    )
}

fn send_control(
    endpoint: &Path,
    request: &ControlRequest,
    context: &str,
) -> Result<(), CommandError> {
    let response: ControlResponse =
        send_wire(endpoint, request).map_err(|source| CommandError::Busy {
            context: format!("{context}: parent supervisor недоступен: {source}"),
        })?;
    match response.code {
        0 => Ok(()),
        2 => Err(CommandError::Syntax {
            context: response.message,
        }),
        3 => Err(CommandError::Invalid {
            context: response.message,
        }),
        4 => Err(CommandError::NotFound {
            context: response.message,
        }),
        5 => Err(CommandError::Busy {
            context: response.message,
        }),
        _ => Err(CommandError::Runtime {
            context: response.message,
            source: std::io::Error::other("parent runtime error"),
        }),
    }
}

fn send_wire<T: for<'de> Deserialize<'de>>(
    endpoint: &Path,
    request: &ControlRequest,
) -> Result<T, std::io::Error> {
    let mut stream = UnixStream::connect(endpoint)?;
    write_frame(&mut stream, request)?;
    read_frame(&mut stream)
}

fn write_frame(stream: &mut UnixStream, value: &impl Serialize) -> Result<(), std::io::Error> {
    let bytes = serde_yaml::to_string(value)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?
        .into_bytes();
    let length = u64::try_from(bytes.len()).map_err(std::io::Error::other)?;
    stream.write_all(&length.to_be_bytes())?;
    stream.write_all(&bytes)
}

fn read_frame<T: for<'de> Deserialize<'de>>(stream: &mut UnixStream) -> Result<T, std::io::Error> {
    let mut length = [0_u8; 8];
    stream.read_exact(&mut length)?;
    let length = usize::try_from(u64::from_be_bytes(length)).map_err(std::io::Error::other)?;
    let mut bytes = vec![0; length];
    stream.read_exact(&mut bytes)?;
    serde_yaml::from_slice(&bytes)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
}

fn validate_materialized(
    workflow: &MaterializedWorkflow,
    registry: &dyn AgentRegistry,
    run_id: RunId,
) -> Result<(), CommandError> {
    if workflow.max_parallel_agents == 0
        || workflow.steps.is_empty()
        || !valid_id(&workflow.workflow_id)
    {
        return Err(invalid_run(run_id, "невалидный materialized workflow"));
    }
    let mut ids = HashSet::with_capacity(workflow.steps.len());
    for step in &workflow.steps {
        let outputs: HashSet<&str> = step.outputs.iter().map(String::as_str).collect();
        if !valid_id(&step.id)
            || !ids.insert(step.id.as_str())
            || outputs.len() != step.outputs.len()
            || step.outputs.iter().any(|id| !valid_id(id))
        {
            return Err(invalid_run(
                run_id,
                "невалидные или повторяющиеся Step/Input IDs",
            ));
        }
        registry
            .validate(&step.agent.r#type, &step.agent.model, &step.agent.reasoning)
            .map_err(|context| invalid_run(run_id, &context))?;
    }
    for step in &workflow.steps {
        if step
            .depends_on
            .iter()
            .any(|dependency| !ids.contains(dependency.as_str()))
        {
            return Err(invalid_run(
                run_id,
                "depends-on ссылается на неизвестный Step",
            ));
        }
    }
    Ok(())
}

fn validate_record(
    record: &AttemptRecord,
    step: &MaterializedStep,
    attempt: u64,
    directory: &Path,
    run_id: RunId,
) -> Result<(), CommandError> {
    if !record.input.is_empty() {
        return Err(invalid_run(
            run_id,
            "initial attempt input должен быть пустым",
        ));
    }
    let mut last_session = None;
    for (index, event) in record.events.iter().enumerate() {
        match event {
            AttemptEvent::SessionActivated { session_id } => {
                if last_session == Some(session_id.as_str()) {
                    return Err(invalid_run(run_id, "повтор последней session activation"));
                }
                last_session = Some(session_id.as_str());
            }
            AttemptEvent::Completed if index + 1 != record.events.len() => {
                return Err(invalid_run(
                    run_id,
                    "completed не является последним событием",
                ));
            }
            AttemptEvent::Completed => {}
        }
    }
    if record.is_completed() {
        for output in &step.outputs {
            let artifact = directory.join(format!("{attempt}.{}.{output}.artifact", step.id));
            if !artifact.is_file() {
                return Err(invalid_run(
                    run_id,
                    &format!("отсутствует artifact {output}"),
                ));
            }
        }
        let prefix = format!("{attempt}.{}.", step.id);
        for entry in fs::read_dir(directory).map_err(|source| {
            runtime(
                "resume",
                format!("не удалось прочитать {}", directory.display()),
                source,
            )
        })? {
            let entry = entry.map_err(|source| {
                runtime("resume", "не удалось прочитать durable entry", source)
            })?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if let Some(input_id) = name
                .strip_prefix(&prefix)
                .and_then(|value| value.strip_suffix(".artifact"))
                && !step.outputs.iter().any(|output| output == input_id)
            {
                return Err(invalid_run(
                    run_id,
                    &format!("дополнительный artifact {input_id}"),
                ));
            }
        }
    }
    Ok(())
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
}

fn ensure_active(state: &ControlState) -> Result<(), CommandError> {
    if state.active {
        Ok(())
    } else {
        Err(CommandError::Busy {
            context: "control: context текущей обработки attempt закрыт".to_owned(),
        })
    }
}

fn lock_state(
    state: &Arc<Mutex<ControlState>>,
) -> Result<std::sync::MutexGuard<'_, ControlState>, CommandError> {
    state.lock().map_err(|_| CommandError::Runtime {
        context: "run: control state повреждён после panic".to_owned(),
        source: std::io::Error::other("poisoned control state"),
    })
}

fn attempt_name(attempt: u64, step_id: &str) -> String {
    format!("{attempt}.{step_id}.attempt.yaml")
}

fn publish_yaml(
    directory: &Path,
    name: &str,
    value: &impl Serialize,
    context: &str,
) -> Result<(), CommandError> {
    let bytes = serde_yaml::to_string(value).map_err(|source| CommandError::Invalid {
        context: format!("{context}: кандидат нельзя сериализовать: {source}"),
    })?;
    publish_bytes(directory, name, bytes.as_bytes(), context)
}

fn publish_bytes(
    directory: &Path,
    name: &str,
    bytes: &[u8],
    context: &str,
) -> Result<(), CommandError> {
    publish_bytes_with_hook(directory, name, bytes, context, || Ok(()))
}

fn publish_bytes_with_hook(
    directory: &Path,
    name: &str,
    bytes: &[u8],
    context: &str,
    before_commit: impl FnOnce() -> Result<(), std::io::Error>,
) -> Result<(), CommandError> {
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary = directory.join(format!(".{name}.{}.{sequence}.tmp", std::process::id()));
    let final_path = directory.join(name);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|source| {
            runtime(
                context,
                format!("не удалось создать {}", temporary.display()),
                source,
            )
        })?;
    let operation = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        before_commit()?;
        fs::rename(&temporary, &final_path)?;
        File::open(directory)?.sync_all()
    })();
    if operation.is_err() {
        drop(file);
        let _ = fs::remove_file(&temporary);
    }
    operation.map_err(|source| {
        runtime(
            context,
            format!("не удалось атомарно опубликовать {}", final_path.display()),
            source,
        )
    })
}

fn read_yaml<T: for<'de> Deserialize<'de>>(path: &Path, context: &str) -> Result<T, CommandError> {
    let bytes = fs::read(path).map_err(|source| {
        runtime(
            context,
            format!("не удалось прочитать {}", path.display()),
            source,
        )
    })?;
    serde_yaml::from_slice(&bytes).map_err(|source| CommandError::Invalid {
        context: format!(
            "{context}: невалидный durable файл {}: {source}",
            path.display()
        ),
    })
}

fn invalid_run(run_id: RunId, message: &str) -> CommandError {
    CommandError::Invalid {
        context: format!("resume: run {run_id}: {message}"),
    }
}

fn runtime(context: &str, message: impl std::fmt::Display, source: std::io::Error) -> CommandError {
    CommandError::Runtime {
        context: format!("{context}: {message}"),
        source,
    }
}

fn report(
    reporter: &mut dyn LifecycleReporter,
    line: &str,
    context: &str,
) -> Result<(), CommandError> {
    reporter.line(line).map_err(|source| {
        runtime(
            context,
            "не удалось вывести lifecycle-сообщение".to_owned(),
            source,
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn invalid_completion_does_not_replace_previous_candidate() {
        let root = TempDir::new().expect("test root must be created");
        let good = root.path().join("good");
        fs::write(&good, b"good").expect("artifact must be written");
        let state = Arc::new(Mutex::new(ControlState {
            active: true,
            run_id: RunId(1),
            attempt: 0,
            run_directory: root.path().to_owned(),
            step_id: "step".to_owned(),
            outputs: vec!["result".to_owned()],
            record: AttemptRecord {
                input: Vec::new(),
                events: Vec::new(),
            },
            candidate: None,
        }));
        accept_completion(&state, &[("result".to_owned(), good)])
            .expect("candidate must be accepted");
        assert!(
            accept_completion(&state, &[("extra".to_owned(), root.path().join("missing"))])
                .is_err()
        );
        assert!(
            lock_state(&state)
                .expect("state must be readable")
                .candidate
                .is_some()
        );
    }

    #[test]
    fn attempt_record_failure_before_commit_keeps_previous_durable_bytes() {
        let root = TempDir::new().expect("test root must be created");
        let name = "0.step.attempt.yaml";
        let path = root.path().join(name);
        fs::write(&path, b"input: []\nevents: []\n").expect("initial attempt must be written");

        let result = publish_bytes_with_hook(
            root.path(),
            name,
            b"input: []\nevents:\n- type: completed\n",
            "test",
            || Err(std::io::Error::other("injected before commit")),
        );

        assert!(result.is_err());
        assert_eq!(
            fs::read(&path).expect("previous record must remain readable"),
            b"input: []\nevents: []\n"
        );
    }
}
