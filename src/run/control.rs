//! Control endpoint активного attempt; модуль не планирует Steps и не владеет чтением run snapshot.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::thread;

use serde::{Deserialize, Serialize};

use crate::domain::Outputs;

use super::storage::{attempt_name, publish_bytes, publish_yaml, read_yaml};
use super::{TEMP_SEQUENCE, runtime};
use crate::agent::AttemptControl;
use crate::config::CommandError;
use crate::domain::{AttemptRecord, MaterializedStep, RunId};

#[derive(Debug)]
pub(super) struct CompletionCandidate(pub(super) BTreeMap<String, Vec<u8>>);

pub(super) struct ControlState {
    pub(super) active: bool,
    pub(super) run_id: RunId,
    pub(super) attempt: u64,
    pub(super) run_directory: PathBuf,
    pub(super) step_id: String,
    pub(super) outputs: Outputs,
    pub(super) record: AttemptRecord,
    pub(super) candidate: Option<CompletionCandidate>,
    pub(super) storage: Arc<Mutex<()>>,
}

type ActiveContexts = HashMap<u64, Arc<Mutex<ControlState>>>;

pub(super) struct ControlHub {
    pub(super) run_id: RunId,
    pub(super) contexts: Mutex<ActiveContexts>,
    pub(super) storage: Arc<Mutex<()>>,
}

pub(super) struct ControlHandle {
    pub(super) state: Arc<Mutex<ControlState>>,
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
    state.record.activate_session(session_id.to_owned());
    let _storage = lock_storage(&state.storage)?;
    publish_yaml(
        &state.run_directory,
        &attempt_name(state.attempt, &state.step_id),
        &state.record,
        "session activate",
    )
}

pub(super) fn accept_completion(
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
    if !state.outputs.accepts(found.iter().copied()) {
        return Err(CommandError::Invalid {
            context: "attempt complete: набор InputIds не совпадает с outputs Step".to_owned(),
        });
    }
    state.candidate = Some(CompletionCandidate(bytes));
    Ok(())
}

/// Под storage lock заменяет неподтверждённые artifacts выбранным snapshot и последним публикует completed; до этой точки resume игнорирует файлы attempt, см. Rule «Completion выбирает допустимый полный набор outputs» в `features/conditional_graph.feature`.
pub(super) fn finalize_completion(
    directory: &Path,
    attempt: u64,
    step: &MaterializedStep,
    candidate: CompletionCandidate,
) -> Result<(), CommandError> {
    for output in &step.outputs {
        if !candidate.0.contains_key(output) {
            let path = directory.join(format!("{attempt}.{}.{output}.artifact", step.id));
            match fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(source) => {
                    return Err(runtime(
                        "attempt complete",
                        "не удалось удалить неопубликованный artifact",
                        source,
                    ));
                }
            }
        }
    }
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
    record.complete();
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

pub(super) struct ControlServer {
    pub(super) path: PathBuf,
    thread: thread::JoinHandle<Result<(), CommandError>>,
}

impl ControlServer {
    pub(super) fn start(directory: &Path, hub: Arc<ControlHub>) -> Result<Self, CommandError> {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let socket_root = if Path::new("/private/tmp").is_dir() {
            Path::new("/private/tmp")
        } else {
            Path::new("/tmp")
        };
        let namespace = control_namespace(directory);
        let prefix = format!("orc-control-{namespace:016x}-");
        remove_stale_endpoints(socket_root, &prefix)?;
        let path = socket_root.join(format!("{prefix}{}-{sequence}.sock", std::process::id()));
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
        let thread = thread::spawn(move || serve_control(listener, hub));
        Ok(Self { path, thread })
    }

    pub(super) fn stop(self) -> Result<(), CommandError> {
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

fn control_namespace(directory: &Path) -> u64 {
    let mut hasher = DefaultHasher::new();
    directory.hash(&mut hasher);
    hasher.finish()
}

fn remove_stale_endpoints(socket_root: &Path, prefix: &str) -> Result<(), CommandError> {
    let entries = fs::read_dir(socket_root).map_err(|source| {
        runtime(
            "run",
            format!(
                "не удалось проверить stale control endpoints в {}",
                socket_root.display()
            ),
            source,
        )
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| {
            runtime(
                "run",
                format!(
                    "не удалось прочитать stale control endpoint в {}",
                    socket_root.display()
                ),
                source,
            )
        })?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name.starts_with(prefix)
            && Path::new(name)
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("sock"))
        {
            fs::remove_file(entry.path()).map_err(|source| {
                runtime(
                    "run",
                    format!(
                        "не удалось удалить stale control endpoint {}",
                        entry.path().display()
                    ),
                    source,
                )
            })?;
        }
    }
    Ok(())
}

#[allow(clippy::needless_pass_by_value)]
fn serve_control(listener: UnixListener, hub: Arc<ControlHub>) -> Result<(), CommandError> {
    for incoming in listener.incoming() {
        let mut stream = incoming
            .map_err(|source| runtime("control", "не удалось принять запрос".to_owned(), source))?;
        let request: ControlRequest = read_frame(&mut stream).map_err(|source| {
            runtime("control", "не удалось прочитать запрос".to_owned(), source)
        })?;
        if matches!(request, ControlRequest::Shutdown) {
            return Ok(());
        }
        let result = handle_request(&hub, request);
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

fn handle_request(hub: &ControlHub, request: ControlRequest) -> Result<(), CommandError> {
    match request {
        ControlRequest::SessionActivate {
            run_id,
            attempt,
            session_id,
        } => {
            let state = context_for_request(hub, run_id, attempt)?;
            activate_session(&state, &session_id)
        }
        ControlRequest::AttemptComplete {
            run_id,
            attempt,
            artifacts,
        } => {
            let state = context_for_request(hub, run_id, attempt)?;
            accept_completion(&state, &artifacts)
        }
        ControlRequest::Shutdown => Ok(()),
    }
}

fn context_for_request(
    hub: &ControlHub,
    run_id: u64,
    attempt: u64,
) -> Result<Arc<Mutex<ControlState>>, CommandError> {
    if hub.run_id.0 != run_id {
        return Err(CommandError::Busy {
            context: "control: context текущей обработки attempt закрыт".to_owned(),
        });
    }
    let contexts = lock_contexts(&hub.contexts)?;
    let state = contexts
        .get(&attempt)
        .cloned()
        .ok_or_else(|| CommandError::Busy {
            context: "control: context текущей обработки attempt закрыт".to_owned(),
        })?;
    drop(contexts);
    {
        let context = lock_state(&state)?;
        if context.run_id.0 != run_id || context.attempt != attempt || !context.active {
            return Err(CommandError::Busy {
                context: "control: context текущей обработки attempt закрыт".to_owned(),
            });
        }
    }
    Ok(state)
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

fn ensure_active(state: &ControlState) -> Result<(), CommandError> {
    if state.active {
        Ok(())
    } else {
        Err(CommandError::Busy {
            context: "control: context текущей обработки attempt закрыт".to_owned(),
        })
    }
}

pub(super) fn lock_state(
    state: &Arc<Mutex<ControlState>>,
) -> Result<std::sync::MutexGuard<'_, ControlState>, CommandError> {
    state.lock().map_err(|_| CommandError::Runtime {
        context: "run: control state повреждён после panic".to_owned(),
        source: std::io::Error::other("poisoned control state"),
    })
}

pub(super) fn lock_storage(
    storage: &Arc<Mutex<()>>,
) -> Result<std::sync::MutexGuard<'_, ()>, CommandError> {
    storage.lock().map_err(|_| CommandError::Runtime {
        context: "run: storage boundary повреждён после panic".to_owned(),
        source: std::io::Error::other("poisoned storage boundary"),
    })
}

fn lock_contexts(
    contexts: &Mutex<ActiveContexts>,
) -> Result<std::sync::MutexGuard<'_, ActiveContexts>, CommandError> {
    contexts.lock().map_err(|_| CommandError::Runtime {
        context: "run: control contexts повреждены после panic".to_owned(),
        source: std::io::Error::other("poisoned control contexts"),
    })
}

pub(super) fn register_context(
    hub: &ControlHub,
    attempt: u64,
    state: Arc<Mutex<ControlState>>,
) -> Result<(), CommandError> {
    if lock_contexts(&hub.contexts)?
        .insert(attempt, state)
        .is_some()
    {
        return Err(CommandError::Runtime {
            context: format!("run {}: attempt {attempt} уже выполняется", hub.run_id),
            source: std::io::Error::other("duplicate active attempt context"),
        });
    }
    Ok(())
}

pub(super) fn unregister_context(hub: &ControlHub, attempt: u64) -> Result<(), CommandError> {
    lock_contexts(&hub.contexts)?.remove(&attempt);
    Ok(())
}
