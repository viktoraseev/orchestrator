//! Lock, чтение, проверка и атомарная публикация durable run; модуль не планирует и не запускает attempts.

use std::fs::{self, File, OpenOptions};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};

use fs2::FileExt;
use serde::{Deserialize, Serialize};

use super::executor::prepare_agent_input;
use super::{INSPECTION_SNAPSHOT_ATTEMPTS, TEMP_SEQUENCE, invalid_run, runtime};
use crate::agent::{AgentRegistry, BuiltinAgentRegistry};
use crate::config::CommandError;
use crate::domain::{AttemptRecord, DurableAttempt, MaterializedStep, MaterializedWorkflow, RunId};

/// Удерживает exclusive kernel lock run до завершения supervisor; см. Rule «Start обещает только durable run» в `features/lifecycle.feature`.
pub(super) struct RunGuard {
    pub(super) directory: PathBuf,
    _lock: File,
}

pub(super) struct RunSnapshot {
    pub(super) run_id: RunId,
    pub(super) directory: PathBuf,
    pub(super) workflow: MaterializedWorkflow,
    pub(super) attempts: Vec<DurableAttempt>,
}

impl RunGuard {
    fn acquire(directory: PathBuf, context: &str) -> Result<Self, CommandError> {
        Self::open(directory, context, true, None)
    }

    pub(super) fn acquire_existing(
        directory: PathBuf,
        run_id: RunId,
    ) -> Result<Self, CommandError> {
        Self::open(
            directory,
            &format!("resume: run {run_id}"),
            false,
            Some(run_id),
        )
    }

    fn open(
        directory: PathBuf,
        context: &str,
        create: bool,
        run_id: Option<RunId>,
    ) -> Result<Self, CommandError> {
        let lock_path = directory.join("active.lock");
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(create)
            .truncate(false)
            .open(&lock_path)
            .map_err(|source| {
                run_id.map_or_else(
                    || {
                        runtime(
                            context,
                            format!("не удалось открыть {}", lock_path.display()),
                            source,
                        )
                    },
                    |id| invalid_run(id, "active.lock отсутствует или недоступен"),
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

/// Возвращает 0 для run без зарезервированных attempts, иначе монотонно увеличивает максимальный номер из durable и crash-leftover имён; см. Rule «Номера attempts глобальны и не переиспользуются» в `features/recovery.feature`.
pub(super) fn next_available_attempt_number(
    directory: &Path,
    attempts: &[DurableAttempt],
    run_id: RunId,
) -> Result<u64, CommandError> {
    let mut maximum = attempts.iter().map(|attempt| attempt.number).max();
    for entry in fs::read_dir(directory).map_err(|source| {
        runtime(
            "resume",
            format!("не удалось прочитать {}", directory.display()),
            source,
        )
    })? {
        let entry = entry
            .map_err(|source| runtime("resume", "не удалось прочитать durable entry", source))?;
        let name = entry.file_name();
        if let Some(number) = reserved_attempt_number(&name.to_string_lossy()) {
            maximum = Some(maximum.map_or(number, |current| current.max(number)));
        }
    }
    let Some(maximum) = maximum else {
        return Ok(0);
    };
    maximum
        .checked_add(1)
        .ok_or_else(|| invalid_run(run_id, "attempt number overflow"))
}

fn reserved_attempt_number(name: &str) -> Option<u64> {
    let durable = name.ends_with(".attempt.yaml") || name.ends_with(".artifact");
    let temporary = name.starts_with('.')
        && name.strip_suffix(".tmp").is_some()
        && (name.contains(".attempt.yaml.") || name.contains(".artifact."));
    if !durable && !temporary {
        return None;
    }
    name.trim_start_matches('.')
        .split('.')
        .next()
        .and_then(|number| number.parse().ok())
}

pub(super) fn reserve_run(root: &Path) -> Result<(RunId, RunGuard), CommandError> {
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

pub(super) fn validate_materialized(
    workflow: &MaterializedWorkflow,
    registry: &dyn AgentRegistry,
    run_id: RunId,
) -> Result<(), CommandError> {
    workflow
        .validate(registry)
        .map_err(|message| invalid_run(run_id, &message))
}

/// Загружает и проверяет полную durable-модель attempts; пустая модель допустима для recovery после crash до публикации initial attempt, а частичная модель обязана начинаться с непротиворечивого attempt 0; см. Rules в `features/recovery.feature`.
pub(super) fn load_attempts(
    directory: &Path,
    workflow: &MaterializedWorkflow,
    registry: &dyn AgentRegistry,
    run_id: RunId,
) -> Result<Vec<DurableAttempt>, CommandError> {
    validate_materialized(workflow, registry, run_id)?;
    let mut attempts = Vec::new();
    for entry in fs::read_dir(directory).map_err(|source| {
        runtime(
            "resume",
            format!("не удалось прочитать {}", directory.display()),
            source,
        )
    })? {
        let entry = entry
            .map_err(|source| runtime("resume", "не удалось прочитать durable entry", source))?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || !name.ends_with(".attempt.yaml") {
            continue;
        }
        let stem = name
            .strip_suffix(".attempt.yaml")
            .ok_or_else(|| invalid_run(run_id, "невалидное имя attempt record"))?;
        let (number, step_id) = stem
            .split_once('.')
            .ok_or_else(|| invalid_run(run_id, "невалидное имя attempt record"))?;
        let number = number
            .parse::<u64>()
            .map_err(|_| invalid_run(run_id, "невалидный номер attempt"))?;
        let step_index = workflow
            .steps
            .iter()
            .position(|step| step.id == step_id)
            .ok_or_else(|| invalid_run(run_id, "attempt ссылается на неизвестный Step"))?;
        if !entry
            .file_type()
            .map_err(|source| runtime("resume", "не удалось прочитать тип durable entry", source))?
            .is_file()
        {
            return Err(invalid_run(
                run_id,
                "attempt record не является regular file",
            ));
        }
        let record: AttemptRecord = read_yaml(&entry.path(), "resume")?;
        attempts.push(DurableAttempt {
            number,
            step_index,
            record,
        });
    }
    attempts.sort_by_key(|attempt| attempt.number);
    if attempts.first().is_some_and(|initial| {
        initial.number != 0 || initial.step_index != 0 || !initial.record.input().is_empty()
    }) {
        return Err(invalid_run(
            run_id,
            "initial attempt 0 отсутствует или противоречив",
        ));
    }
    for pair in attempts.windows(2) {
        if pair[0].number == pair[1].number {
            return Err(invalid_run(run_id, "глобальный attempt number повторяется"));
        }
    }
    for step_index in 0..workflow.steps.len() {
        if attempts
            .iter()
            .filter(|attempt| attempt.step_index == step_index && !attempt.record.is_completed())
            .count()
            > 1
        {
            return Err(invalid_run(
                run_id,
                "Step имеет несколько незавершённых attempts",
            ));
        }
    }
    for attempt in &attempts {
        let step = &workflow.steps[attempt.step_index];
        validate_record(&attempt.record, step, attempt.number, directory, run_id)?;
        validate_attempt_input(workflow, &attempts, attempt, run_id)?;
        prepare_agent_input(directory, workflow, attempt, run_id)?;
    }
    Ok(attempts)
}

fn validate_attempt_input(
    workflow: &MaterializedWorkflow,
    attempts: &[DurableAttempt],
    attempt: &DurableAttempt,
    run_id: RunId,
) -> Result<(), CommandError> {
    if attempt.number == 0 {
        return Ok(());
    }
    let step = &workflow.steps[attempt.step_index];
    if attempt.record.input().len() != step.depends_on.len() {
        return Err(invalid_run(
            run_id,
            "attempt input не совпадает с depends-on",
        ));
    }
    let previous = attempts
        .iter()
        .filter(|candidate| {
            candidate.step_index == attempt.step_index && candidate.number < attempt.number
        })
        .max_by_key(|candidate| candidate.number);
    for (dependency_index, (dependency, source_number)) in step
        .depends_on
        .iter()
        .zip(attempt.record.input())
        .enumerate()
    {
        let source_index = workflow
            .steps
            .iter()
            .position(|candidate| candidate.id == *dependency)
            .ok_or_else(|| invalid_run(run_id, "unknown dependency"))?;
        let source = attempts
            .iter()
            .find(|candidate| candidate.number == *source_number)
            .ok_or_else(|| {
                invalid_run(run_id, "attempt input ссылается на отсутствующий source")
            })?;
        if source.step_index != source_index
            || !source.record.is_completed()
            || source.number >= attempt.number
        {
            return Err(invalid_run(
                run_id,
                "attempt input ссылается на неготовый source",
            ));
        }
        let latest = attempts
            .iter()
            .filter(|candidate| {
                candidate.step_index == source_index
                    && candidate.record.is_completed()
                    && candidate.number < attempt.number
            })
            .map(|candidate| candidate.number)
            .max();
        if latest != Some(*source_number) {
            return Err(invalid_run(
                run_id,
                "attempt input выбрал не последнюю source version",
            ));
        }
        if previous
            .and_then(|candidate| candidate.record.input().get(dependency_index))
            .is_some_and(|bound| source_number <= bound)
        {
            return Err(invalid_run(
                run_id,
                "attempt input повторно использует старую source version",
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
    record
        .validate_history()
        .map_err(|message| invalid_run(run_id, message))?;
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

pub(super) fn attempt_name(attempt: u64, step_id: &str) -> String {
    format!("{attempt}.{step_id}.attempt.yaml")
}

pub(super) fn publish_yaml(
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

/// Публикует durable файл атомарной заменой и sync каталога; только после успеха `resume` может наблюдать новые bytes.
pub(super) fn publish_bytes(
    directory: &Path,
    name: &str,
    bytes: &[u8],
    context: &str,
) -> Result<(), CommandError> {
    publish_bytes_with_hook(directory, name, bytes, context, || Ok(()))
}

pub(super) fn publish_bytes_with_hook(
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

pub(super) fn read_yaml<T: for<'de> Deserialize<'de>>(
    path: &Path,
    context: &str,
) -> Result<T, CommandError> {
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

pub(super) fn list_run_ids(root: &Path, context: &str) -> Result<Vec<RunId>, CommandError> {
    let run_root = root.join("run");
    let entries = match fs::read_dir(&run_root) {
        Ok(entries) => entries,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(runtime(
                context,
                format!("не удалось прочитать {}", run_root.display()),
                source,
            ));
        }
    };
    let mut run_ids = Vec::new();
    for entry in entries {
        let entry = entry
            .map_err(|source| runtime(context, "не удалось прочитать durable entry", source))?;
        if entry
            .file_type()
            .map_err(|source| runtime(context, "не удалось прочитать тип run entry", source))?
            .is_dir()
            && let Some(name) = entry.file_name().to_str()
            && let Ok(value) = name.parse::<u64>()
        {
            run_ids.push(RunId(value));
        }
    }
    run_ids.sort_by_key(|run_id| run_id.0);
    Ok(run_ids)
}

/// Читает validated snapshot без Run lock и возвращает его только при одинаковом durable fingerprint до и после чтения; см. Rule «Run inspection читает согласованный snapshot» в `features/run_inspection.feature`.
pub(super) fn load_run_snapshot(
    root: &Path,
    run_id: RunId,
    context: &str,
) -> Result<RunSnapshot, CommandError> {
    let directory = root.join("run").join(run_id.to_string());
    if !directory.is_dir() {
        return Err(CommandError::NotFound {
            context: format!("{context}: run {run_id} не существует"),
        });
    }
    load_run_snapshot_with_hook(&directory, run_id, context, || {})
}

pub(super) fn load_run_snapshot_with_hook(
    directory: &Path,
    run_id: RunId,
    context: &str,
    mut after_fingerprint: impl FnMut(),
) -> Result<RunSnapshot, CommandError> {
    let mut last_snapshot_error = None;
    for _ in 0..INSPECTION_SNAPSHOT_ATTEMPTS {
        let before = match durable_fingerprint(directory, context) {
            Ok(value) => value,
            Err(error) => {
                last_snapshot_error = Some(error);
                continue;
            }
        };
        after_fingerprint();
        let result = load_run_snapshot_once(directory, run_id, context);
        let after = match durable_fingerprint(directory, context) {
            Ok(value) => value,
            Err(error) => {
                last_snapshot_error = Some(error);
                continue;
            }
        };
        if before == after {
            return result;
        }
    }
    let detail = last_snapshot_error.map_or_else(
        || "durable snapshot непрерывно изменяется".to_owned(),
        |error| format!("durable snapshot непрерывно изменяется: {error}"),
    );
    Err(CommandError::Runtime {
        context: format!("{context}: run {run_id}: {detail}"),
        source: std::io::Error::other("исчерпан лимит чтения согласованного snapshot"),
    })
}

fn load_run_snapshot_once(
    directory: &Path,
    run_id: RunId,
    context: &str,
) -> Result<RunSnapshot, CommandError> {
    let spec_path = directory.join("spec.yaml");
    if !spec_path.is_file() {
        return Err(CommandError::Invalid {
            context: format!(
                "{context}: run {run_id}: spec.yaml отсутствует или не является regular file"
            ),
        });
    }
    let workflow: MaterializedWorkflow = read_yaml(&spec_path, context)?;
    let attempts = load_attempts(directory, &workflow, &BuiltinAgentRegistry, run_id)
        .map_err(|error| recontextualize(error, context))?;
    Ok(RunSnapshot {
        run_id,
        directory: directory.to_owned(),
        workflow,
        attempts,
    })
}

fn durable_fingerprint(directory: &Path, context: &str) -> Result<u64, CommandError> {
    let mut paths = Vec::new();
    for entry in fs::read_dir(directory).map_err(|source| {
        runtime(
            context,
            format!("не удалось прочитать {}", directory.display()),
            source,
        )
    })? {
        let entry = entry
            .map_err(|source| runtime(context, "не удалось прочитать durable entry", source))?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == "spec.yaml" || name.ends_with(".attempt.yaml") || name.ends_with(".artifact") {
            paths.push(entry.path());
        }
    }
    paths.sort();
    let mut hasher = DefaultHasher::new();
    for path in paths {
        path.file_name().hash(&mut hasher);
        let metadata = fs::symlink_metadata(&path).map_err(|source| {
            runtime(
                context,
                format!("не удалось прочитать metadata {}", path.display()),
                source,
            )
        })?;
        metadata.file_type().is_file().hash(&mut hasher);
        metadata.file_type().is_symlink().hash(&mut hasher);
        if metadata.file_type().is_file() {
            fs::read(&path)
                .map_err(|source| {
                    runtime(
                        context,
                        format!("не удалось прочитать {}", path.display()),
                        source,
                    )
                })?
                .hash(&mut hasher);
        }
    }
    Ok(hasher.finish())
}

pub(super) fn artifact_size(path: &Path, context: &str) -> Result<u64, CommandError> {
    path.metadata()
        .map_err(|source| {
            runtime(
                context,
                format!("не удалось прочитать metadata {}", path.display()),
                source,
            )
        })
        .map(|metadata| metadata.len())
}

pub(super) fn open_artifact(path: &Path) -> Result<File, CommandError> {
    File::open(path).map_err(|source| {
        runtime(
            "run artifact",
            format!("не удалось открыть {}", path.display()),
            source,
        )
    })
}

fn recontextualize(error: CommandError, context: &str) -> CommandError {
    fn replace(value: String, context: &str) -> String {
        if let Some(suffix) = value.strip_prefix("resume:") {
            format!("{context}:{suffix}")
        } else {
            value
        }
    }
    match error {
        CommandError::Syntax { context: value } => CommandError::Syntax {
            context: replace(value, context),
        },
        CommandError::Invalid { context: value } => CommandError::Invalid {
            context: replace(value, context),
        },
        CommandError::NotFound { context: value } => CommandError::NotFound {
            context: replace(value, context),
        },
        CommandError::Busy { context: value } => CommandError::Busy {
            context: replace(value, context),
        },
        CommandError::Interrupted {
            context: value,
            exit_code,
        } => CommandError::Interrupted {
            context: replace(value, context),
            exit_code,
        },
        CommandError::Runtime {
            context: value,
            source,
        } => CommandError::Runtime {
            context: replace(value, context),
            source,
        },
    }
}
