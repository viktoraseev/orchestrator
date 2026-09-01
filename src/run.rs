//! Lifecycle run, storage и control boundary; модуль не разбирает CLI и не знает протокол конкретного Agent type.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as FmtWrite;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use fs2::FileExt;
use serde::{Deserialize, Serialize};

use crate::agent::{
    AgentCancellation, AgentInput, AgentRegistry, AgentRunRequest, AttemptControl,
    BuiltinAgentRegistry, TerminationSignal,
};
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

/// Доступность терминала lifecycle-команды для прямого запуска human Agent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalMode {
    /// Lifecycle запущен без интерактивного терминала.
    Unavailable,
    /// Stdin и stdout lifecycle-команды подключены к терминалу.
    Available,
}

impl TerminalMode {
    const fn is_available(self) -> bool {
        matches!(self, Self::Available)
    }
}

#[derive(Debug, Default)]
struct SignalState {
    first: AtomicU8,
    count: AtomicUsize,
}

/// Управляемый источник termination signals для lifecycle supervisor.
#[derive(Clone, Debug, Default)]
pub struct LifecycleSignals(Arc<SignalState>);

impl LifecycleSignals {
    /// Регистрирует полученный signal; первый определяет итоговый exit code, повторный требует немедленной эскалации.
    pub fn notify(&self, signal: TerminationSignal) {
        let _ =
            self.0
                .first
                .compare_exchange(0, signal.number(), Ordering::AcqRel, Ordering::Acquire);
        self.0.count.fetch_add(1, Ordering::AcqRel);
    }

    fn observation(&self) -> Option<(TerminationSignal, usize)> {
        let signal = TerminationSignal::from_number(self.0.first.load(Ordering::Acquire))?;
        Some((signal, self.0.count.load(Ordering::Acquire)))
    }
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
    terminal: TerminalMode,
    signals: &LifecycleSignals,
    registry: &dyn AgentRegistry,
    reporter: &mut dyn LifecycleReporter,
) -> Result<(), CommandError> {
    match command {
        LifecycleCommand::Start(selection) => start(
            selection,
            environment,
            terminal,
            signals,
            registry,
            reporter,
        ),
        LifecycleCommand::Resume(run_id) => {
            resume(*run_id, environment, terminal, signals, registry, reporter)
        }
    }
}

/// Перечисляет validated durable runs в порядке возрастания [`RunId`].
///
/// Операция не получает Run lock и ничего не создаёт и не изменяет; см. Rule «Run list вычисляет состояние без побочных эффектов» в `features/run_inspection.feature`.
///
/// # Errors
///
/// Возвращает ошибку корня состояния, чтения либо первой противоречивой durable-модели; до успеха caller не получает частичный вывод.
pub fn execute_run_list(environment: &ProcessEnvironment) -> Result<String, CommandError> {
    let root = resolve_state_root(environment, "run list")?;
    let run_root = root.join("run");
    let entries = match fs::read_dir(&run_root) {
        Ok(entries) => entries,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(String::new()),
        Err(source) => {
            return Err(runtime(
                "run list",
                format!("не удалось прочитать {}", run_root.display()),
                source,
            ));
        }
    };
    let mut run_ids = Vec::new();
    for entry in entries {
        let entry = entry
            .map_err(|source| runtime("run list", "не удалось прочитать durable entry", source))?;
        if !entry
            .file_type()
            .map_err(|source| runtime("run list", "не удалось прочитать тип run entry", source))?
            .is_dir()
        {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if let Ok(value) = name.parse::<u64>() {
            run_ids.push(RunId(value));
        }
    }
    run_ids.sort_by_key(|run_id| run_id.0);
    let runs = run_ids
        .into_iter()
        .map(|run_id| load_inspected_run(&root, run_id, "run list"))
        .collect::<Result<Vec<_>, _>>()?;
    let mut output = String::new();
    for run in &runs {
        writeln!(
            output,
            "run {}: workflow={} state={}",
            run.run_id,
            run.workflow.workflow_id,
            run.state().as_str()
        )
        .map_err(formatting_error)?;
    }
    if output.ends_with('\n') {
        output.pop();
    }
    Ok(output)
}

/// Показывает materialized Steps, attempts и frontier одного validated durable run.
///
/// Операция не получает Run lock и не меняет состояние; см. Rule «Run show отображает validated read model» в `features/run_inspection.feature`.
///
/// # Errors
///
/// Возвращает `4` для неизвестного run и ошибку чтения или validation для противоречивой durable-модели.
pub fn execute_run_show(
    run_id: RunId,
    environment: &ProcessEnvironment,
) -> Result<String, CommandError> {
    let root = resolve_state_root(environment, "run show")?;
    let run = load_inspected_run(&root, run_id, "run show")?;
    let mut output = String::new();
    writeln!(
        output,
        "run {}: workflow={} state={}",
        run.run_id,
        run.workflow.workflow_id,
        run.state().as_str()
    )
    .map_err(formatting_error)?;
    for (step_index, step) in run.workflow.steps.iter().enumerate() {
        let attempts = run
            .attempts
            .iter()
            .filter(|attempt| attempt.step_index == step_index)
            .map(|attempt| attempt.number.to_string())
            .collect::<Vec<_>>();
        writeln!(
            output,
            "step {}: attempts={}",
            step.id,
            joined_or_dash(&attempts)
        )
        .map_err(formatting_error)?;
    }
    for attempt in &run.attempts {
        let step = &run.workflow.steps[attempt.step_index];
        let inputs = attempt
            .record
            .input
            .iter()
            .map(u64::to_string)
            .collect::<Vec<_>>();
        writeln!(
            output,
            "attempt {}: step={} state={} session={} input={}",
            attempt.number,
            step.id,
            if attempt.record.is_completed() {
                "completed"
            } else {
                "active"
            },
            attempt.record.last_session().unwrap_or("-"),
            joined_or_dash(&inputs)
        )
        .map_err(formatting_error)?;
    }
    let ready = run
        .frontier
        .ready
        .iter()
        .map(|step_index| run.workflow.steps[*step_index].id.clone())
        .collect::<Vec<_>>();
    writeln!(
        output,
        "frontier: ready={} missing={}",
        joined_or_dash(&ready),
        joined_or_dash(&run.frontier.missing)
    )
    .map_err(formatting_error)?;
    output.pop();
    Ok(output)
}

/// Открывает точную durable-версию artifact для потокового копирования caller'ом.
///
/// Файл открывается только после полной validation run; см. Rule «Run artifact выбирает точную durable версию» в `features/run_inspection.feature`.
///
/// # Errors
///
/// Возвращает `2` для невалидного `InputId`, `4` для неизвестного run, attempt, output или незавершённого attempt и ошибку validation либо I/O для противоречивого состояния.
pub fn open_run_artifact(
    run_id: RunId,
    attempt_number: u64,
    input_id: &str,
    environment: &ProcessEnvironment,
) -> Result<File, CommandError> {
    if !valid_id(input_id) {
        return Err(CommandError::Syntax {
            context: format!("run artifact: InputId '{input_id}' не соответствует kebab-case"),
        });
    }
    let root = resolve_state_root(environment, "run artifact")?;
    let run = load_inspected_run(&root, run_id, "run artifact")?;
    let attempt = run
        .attempts
        .iter()
        .find(|attempt| attempt.number == attempt_number)
        .ok_or_else(|| CommandError::NotFound {
            context: format!("run artifact: run {run_id}: attempt {attempt_number} не существует"),
        })?;
    let step = &run.workflow.steps[attempt.step_index];
    if !attempt.record.is_completed() {
        return Err(CommandError::NotFound {
            context: format!(
                "run artifact: run {run_id}: attempt {attempt_number} не опубликовал artifacts"
            ),
        });
    }
    if !step.outputs.iter().any(|output| output == input_id) {
        return Err(CommandError::NotFound {
            context: format!(
                "run artifact: run {run_id}: InputId '{input_id}' не объявлен attempt {attempt_number}"
            ),
        });
    }
    let path = run
        .directory
        .join(format!("{attempt_number}.{}.{input_id}.artifact", step.id));
    File::open(&path).map_err(|source| {
        runtime(
            "run artifact",
            format!("не удалось открыть {}", path.display()),
            source,
        )
    })
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
        Self::open(directory, context, true, None)
    }

    fn acquire_existing(directory: PathBuf, run_id: RunId) -> Result<Self, CommandError> {
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

fn start(
    selection: &ValidateCommand,
    environment: &ProcessEnvironment,
    terminal: TerminalMode,
    signals: &LifecycleSignals,
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
    let result = execute_scheduler(
        &guard, run_id, &candidate, terminal, signals, registry, "start",
    )
    .and_then(|outcome| report_scheduler(outcome, run_id, reporter, "start"));
    let exit_result = report(reporter, &format!("Run {run_id} exited"), "start");
    result.and(exit_result)
}

fn resume(
    run_id: RunId,
    environment: &ProcessEnvironment,
    terminal: TerminalMode,
    signals: &LifecycleSignals,
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
    let guard = RunGuard::acquire_existing(directory, run_id)?;
    let spec_path = guard.directory.join("spec.yaml");
    if !spec_path.is_file() {
        return Err(invalid_run(
            run_id,
            "spec.yaml отсутствует или не является regular file",
        ));
    }
    let candidate: MaterializedWorkflow = read_yaml(&spec_path, "resume")?;
    validate_materialized(&candidate, registry, run_id)?;
    let attempts = load_attempts(&guard.directory, &candidate, registry, run_id)?;
    let result = if is_completed(&candidate, &attempts)? {
        report(
            reporter,
            &format!("run {run_id}: already completed"),
            "resume",
        )
    } else {
        execute_scheduler(
            &guard, run_id, &candidate, terminal, signals, registry, "resume",
        )
        .and_then(|outcome| report_scheduler(outcome, run_id, reporter, "resume"))
    };
    let exit_result = report(reporter, &format!("Run {run_id} exited"), "resume");
    result.and(exit_result)
}

#[derive(Clone, Copy)]
struct AttemptExecution<'a> {
    guard: &'a RunGuard,
    run_id: RunId,
    workflow: &'a MaterializedWorkflow,
    registry: &'a dyn AgentRegistry,
    hub: &'a Arc<ControlHub>,
    endpoint: &'a Path,
    cancellation: &'a AgentCancellation,
}

enum AttemptOutcome {
    Returned(bool),
    UserExit,
}

fn run_current_attempt(
    execution: &AttemptExecution<'_>,
    attempt: &DurableAttempt,
) -> Result<AttemptOutcome, CommandError> {
    let AttemptExecution {
        guard,
        run_id,
        workflow,
        registry,
        hub,
        endpoint,
        cancellation,
    } = execution;
    let run_id = *run_id;
    let step = workflow.steps.get(attempt.step_index).ok_or_else(|| {
        invalid_run(
            run_id,
            "attempt ссылается на отсутствующий materialized Step",
        )
    })?;
    let state = Arc::new(Mutex::new(ControlState {
        active: true,
        run_id,
        attempt: attempt.number,
        run_directory: guard.directory.clone(),
        step_id: step.id.clone(),
        outputs: step.outputs.clone(),
        record: attempt.record.clone(),
        candidate: None,
        storage: Arc::clone(&hub.storage),
    }));
    register_context(hub, attempt.number, Arc::clone(&state))?;
    let mut control = ControlHandle {
        state: Arc::clone(&state),
    };
    let (inputs, prompt) = prepare_agent_input(&guard.directory, workflow, attempt, run_id)?;
    let run_id_text = run_id.to_string();
    let result = registry.run(
        &AgentRunRequest {
            step_id: &step.id,
            type_id: &step.agent.r#type,
            model: &step.agent.model,
            reasoning: &step.agent.reasoning,
            prompt: &prompt,
            inputs: &inputs,
            resume_session: attempt.record.last_session(),
            run_id: &run_id_text,
            attempt: attempt.number,
            control_endpoint: endpoint,
            cancellation,
            human: step.human,
        },
        &mut control,
    );
    let candidate = {
        let mut state = lock_state(&state)?;
        state.active = false;
        state.candidate.take()
    };
    unregister_context(hub, attempt.number)?;
    let completed = if let Some(candidate) = candidate {
        let _storage = lock_storage(&hub.storage)?;
        finalize_completion(&guard.directory, attempt.number, step, candidate)?;
        true
    } else {
        false
    };
    let exit = result.map_err(|context| CommandError::Runtime {
        context: format!("run {run_id}: {context}"),
        source: std::io::Error::other("Agent adapter failure"),
    })?;
    match exit {
        crate::agent::AgentExit::Returned(0) => Ok(AttemptOutcome::Returned(completed)),
        crate::agent::AgentExit::Returned(code) => Err(CommandError::Runtime {
            context: format!("run {run_id}: Agent завершился с кодом {code}"),
            source: std::io::Error::other("Agent process failure"),
        }),
        crate::agent::AgentExit::UserExit if step.human && !completed => {
            Ok(AttemptOutcome::UserExit)
        }
        crate::agent::AgentExit::UserExit => Err(CommandError::Runtime {
            context: format!("run {run_id}: невалидный /exit outcome Agent type"),
            source: std::io::Error::other("unexpected Agent user exit"),
        }),
    }
}

#[derive(Clone, Debug)]
struct DurableAttempt {
    number: u64,
    step_index: usize,
    record: AttemptRecord,
}

#[derive(Clone, Copy)]
enum SchedulerOutcome {
    Completed(bool),
    UserExit,
    Signal(TerminationSignal),
}

#[derive(Clone, Copy)]
struct SchedulerExecution<'a> {
    attempt: AttemptExecution<'a>,
    terminal: TerminalMode,
    signals: &'a LifecycleSignals,
    context: &'a str,
}

struct BatchOutcome {
    signal: Option<TerminationSignal>,
    user_exit: bool,
    first_error: Option<CommandError>,
}

fn execute_scheduler(
    guard: &RunGuard,
    run_id: RunId,
    workflow: &MaterializedWorkflow,
    terminal: TerminalMode,
    signals: &LifecycleSignals,
    registry: &dyn AgentRegistry,
    context: &str,
) -> Result<SchedulerOutcome, CommandError> {
    let hub = Arc::new(ControlHub {
        run_id,
        contexts: Mutex::new(HashMap::new()),
        storage: Arc::new(Mutex::new(())),
    });
    let server = registry
        .uses_process_control()
        .then(|| ControlServer::start(&guard.directory, Arc::clone(&hub)))
        .transpose()?;
    let endpoint = server
        .as_ref()
        .map_or_else(PathBuf::new, |server| server.path.clone());
    let cancellation = AgentCancellation::default();
    let execution = SchedulerExecution {
        attempt: AttemptExecution {
            guard,
            run_id,
            workflow,
            registry,
            hub: &hub,
            endpoint: &endpoint,
            cancellation: &cancellation,
        },
        terminal,
        signals,
        context,
    };
    let result = execute_scheduler_inner(&execution);
    let stop_result = server.map(ControlServer::stop).transpose();
    match (result, stop_result) {
        (Err(error), _) | (Ok(_), Err(error)) => Err(error),
        (Ok(completed), Ok(_)) => Ok(completed),
    }
}

fn execute_scheduler_inner(
    execution: &SchedulerExecution<'_>,
) -> Result<SchedulerOutcome, CommandError> {
    let mut launched = HashSet::new();
    loop {
        if let Some((signal, count)) = execution.signals.observation() {
            execution.attempt.cancellation.cancel_with(signal);
            if count > 1 {
                execution.attempt.cancellation.escalate();
            }
            return Ok(SchedulerOutcome::Signal(signal));
        }
        let attempt_execution = execution.attempt;
        let attempts = load_attempts(
            &attempt_execution.guard.directory,
            attempt_execution.workflow,
            attempt_execution.registry,
            attempt_execution.run_id,
        )?;
        let runnable = select_runnable(execution, &attempts, &launched)?;
        if !runnable.is_empty() {
            launched.extend(runnable.iter().map(|attempt| attempt.number));
            let batch = execute_batch(execution, runnable)?;
            if let Some(signal) = batch.signal {
                return Ok(SchedulerOutcome::Signal(signal));
            }
            if batch.user_exit {
                return Ok(SchedulerOutcome::UserExit);
            }
            if let Some(error) = batch.first_error {
                return Err(error);
            }
            continue;
        }

        let frontier = compute_frontier(attempt_execution.workflow, &attempts)?;
        if !frontier.ready.is_empty() {
            let _storage = lock_storage(&attempt_execution.hub.storage)?;
            publish_ready_attempts(
                attempt_execution.guard,
                attempt_execution.run_id,
                attempt_execution.workflow,
                &attempts,
                &frontier.ready,
                execution.context,
            )?;
            continue;
        }
        if !frontier.missing.is_empty() {
            return Err(blocked(
                attempt_execution.run_id,
                execution.context,
                &frontier.missing,
            ));
        }
        return Ok(SchedulerOutcome::Completed(
            attempts.iter().all(|attempt| attempt.record.is_completed()),
        ));
    }
}

fn select_runnable<'a>(
    execution: &SchedulerExecution<'_>,
    attempts: &'a [DurableAttempt],
    launched: &HashSet<u64>,
) -> Result<Vec<&'a DurableAttempt>, CommandError> {
    let workflow = execution.attempt.workflow;
    let mut runnable: Vec<&DurableAttempt> = attempts
        .iter()
        .filter(|attempt| !attempt.record.is_completed() && !launched.contains(&attempt.number))
        .collect();
    runnable.sort_by_key(|attempt| (attempt.step_index, attempt.number));
    let has_human = runnable
        .iter()
        .any(|attempt| workflow.steps[attempt.step_index].human);
    if has_human && !execution.terminal.is_available() {
        return Err(CommandError::Runtime {
            context: format!(
                "{}: run {}: human attempt нельзя запустить без TTY",
                execution.context, execution.attempt.run_id
            ),
            source: std::io::Error::other("TTY недоступен"),
        });
    }
    runnable.sort_by_key(|attempt| {
        (
            !workflow.steps[attempt.step_index].human,
            attempt.step_index,
            attempt.number,
        )
    });
    let mut human_selected = false;
    runnable.retain(|attempt| {
        !workflow.steps[attempt.step_index].human || !std::mem::replace(&mut human_selected, true)
    });
    runnable.truncate(workflow.max_parallel_agents);
    Ok(runnable)
}

fn execute_batch(
    execution: &SchedulerExecution<'_>,
    runnable: Vec<&DurableAttempt>,
) -> Result<BatchOutcome, CommandError> {
    let worker_count = runnable.len();
    thread::scope(|scope| {
        let (sender, receiver) = mpsc::channel();
        for attempt in runnable {
            let sender = sender.clone();
            let attempt_execution = execution.attempt;
            let context = execution.context;
            scope.spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run_current_attempt(&attempt_execution, attempt)
                }))
                .unwrap_or_else(|_| {
                    Err(CommandError::Runtime {
                        context: format!(
                            "{context}: run {}: Agent worker panic",
                            attempt_execution.run_id
                        ),
                        source: std::io::Error::other("Agent worker panic"),
                    })
                });
                let _ = sender.send(result);
            });
        }
        drop(sender);
        receive_batch(execution, &receiver, worker_count)
    })
}

fn receive_batch(
    execution: &SchedulerExecution<'_>,
    receiver: &mpsc::Receiver<Result<AttemptOutcome, CommandError>>,
    mut remaining: usize,
) -> Result<BatchOutcome, CommandError> {
    let mut outcome = BatchOutcome {
        signal: None,
        user_exit: false,
        first_error: None,
    };
    while remaining > 0 {
        match receiver.recv_timeout(std::time::Duration::from_millis(10)) {
            Ok(result) => {
                remaining = remaining.saturating_sub(1);
                receive_attempt_result(execution, &mut outcome, result);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(CommandError::Runtime {
                    context: format!(
                        "{}: run {}: Agent worker потерян",
                        execution.context, execution.attempt.run_id
                    ),
                    source: std::io::Error::other("Agent result channel disconnected"),
                });
            }
        }
        observe_batch_signal(execution, &mut outcome);
    }
    Ok(outcome)
}

fn receive_attempt_result(
    execution: &SchedulerExecution<'_>,
    outcome: &mut BatchOutcome,
    result: Result<AttemptOutcome, CommandError>,
) {
    let error = match result {
        Ok(AttemptOutcome::Returned(true)) => None,
        Ok(AttemptOutcome::Returned(false)) => Some(CommandError::Runtime {
            context: format!(
                "{}: run {}: Agent вернул управление без completion",
                execution.context, execution.attempt.run_id
            ),
            source: std::io::Error::other("attempt остался незавершённым"),
        }),
        Ok(AttemptOutcome::UserExit) => {
            outcome.user_exit = true;
            None
        }
        Err(error) => Some(error),
    };
    if let Some(error) = error
        && !outcome.user_exit
        && outcome.signal.is_none()
    {
        execution.attempt.cancellation.cancel();
        outcome.first_error.get_or_insert(error);
    }
}

fn observe_batch_signal(execution: &SchedulerExecution<'_>, outcome: &mut BatchOutcome) {
    if let Some((signal, count)) = execution.signals.observation() {
        outcome.signal.get_or_insert(signal);
        execution.attempt.cancellation.cancel_with(signal);
        if count > 1 {
            execution.attempt.cancellation.escalate();
        }
    }
}

struct Frontier {
    ready: Vec<usize>,
    missing: Vec<String>,
}

struct InspectedRun {
    run_id: RunId,
    directory: PathBuf,
    workflow: MaterializedWorkflow,
    attempts: Vec<DurableAttempt>,
    frontier: Frontier,
}

#[derive(Clone, Copy)]
enum InspectedRunState {
    Active,
    Blocked,
    Completed,
}

impl InspectedRunState {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Blocked => "blocked",
            Self::Completed => "completed",
        }
    }
}

impl InspectedRun {
    fn state(&self) -> InspectedRunState {
        if self
            .attempts
            .iter()
            .any(|attempt| !attempt.record.is_completed())
            || !self.frontier.ready.is_empty()
        {
            InspectedRunState::Active
        } else if self.frontier.missing.is_empty() {
            InspectedRunState::Completed
        } else {
            InspectedRunState::Blocked
        }
    }
}

fn load_inspected_run(
    root: &Path,
    run_id: RunId,
    context: &str,
) -> Result<InspectedRun, CommandError> {
    let directory = root.join("run").join(run_id.to_string());
    if !directory.is_dir() {
        return Err(CommandError::NotFound {
            context: format!("{context}: run {run_id} не существует"),
        });
    }
    let spec_path = directory.join("spec.yaml");
    if !spec_path.is_file() {
        return Err(CommandError::Invalid {
            context: format!(
                "{context}: run {run_id}: spec.yaml отсутствует или не является regular file"
            ),
        });
    }
    let workflow: MaterializedWorkflow = read_yaml(&spec_path, context)?;
    let attempts = load_attempts(&directory, &workflow, &BuiltinAgentRegistry, run_id)
        .map_err(|error| recontextualize(error, context))?;
    let frontier =
        compute_frontier(&workflow, &attempts).map_err(|error| recontextualize(error, context))?;
    Ok(InspectedRun {
        run_id,
        directory,
        workflow,
        attempts,
        frontier,
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

fn joined_or_dash(values: &[String]) -> String {
    if values.is_empty() {
        "-".to_owned()
    } else {
        values.join(",")
    }
}

fn formatting_error(source: std::fmt::Error) -> CommandError {
    CommandError::Runtime {
        context: "run inspection: не удалось сформировать вывод".to_owned(),
        source: std::io::Error::other(source),
    }
}

fn compute_frontier(
    workflow: &MaterializedWorkflow,
    attempts: &[DurableAttempt],
) -> Result<Frontier, CommandError> {
    let mut ready = Vec::new();
    let mut missing = Vec::new();
    let mut missing_seen = HashSet::new();
    for (step_index, step) in workflow.steps.iter().enumerate() {
        if attempts
            .iter()
            .any(|attempt| attempt.step_index == step_index && !attempt.record.is_completed())
        {
            continue;
        }
        let previous = attempts
            .iter()
            .filter(|attempt| attempt.step_index == step_index)
            .max_by_key(|attempt| attempt.number);
        if step.depends_on.is_empty() {
            continue;
        }
        let mut fresh = Vec::with_capacity(step.depends_on.len());
        for (dependency_index, dependency) in step.depends_on.iter().enumerate() {
            let source_index = workflow
                .steps
                .iter()
                .position(|candidate| candidate.id == *dependency)
                .ok_or_else(|| CommandError::Invalid {
                    context: format!("workflow: неизвестный dependency '{dependency}'"),
                })?;
            let latest = attempts
                .iter()
                .filter(|attempt| {
                    attempt.step_index == source_index && attempt.record.is_completed()
                })
                .map(|attempt| attempt.number)
                .max();
            let lower_bound =
                previous.and_then(|attempt| attempt.record.input.get(dependency_index).copied());
            fresh.push(latest.is_some_and(|number| lower_bound.is_none_or(|bound| number > bound)));
        }
        if fresh.iter().all(|value| *value) {
            ready.push(step_index);
        } else if fresh.iter().any(|value| *value) {
            for (dependency, is_fresh) in step.depends_on.iter().zip(fresh) {
                if !is_fresh && missing_seen.insert(dependency.clone()) {
                    missing.push(dependency.clone());
                }
            }
        }
    }
    Ok(Frontier { ready, missing })
}

fn publish_ready_attempts(
    guard: &RunGuard,
    run_id: RunId,
    workflow: &MaterializedWorkflow,
    attempts: &[DurableAttempt],
    ready: &[usize],
    context: &str,
) -> Result<(), CommandError> {
    let mut next = maximum_reserved_attempt_number(&guard.directory, attempts, run_id)?
        .checked_add(1)
        .ok_or_else(|| invalid_run(run_id, "attempt number overflow"))?;
    for &step_index in ready {
        let step = &workflow.steps[step_index];
        let mut input = Vec::with_capacity(step.depends_on.len());
        for dependency in &step.depends_on {
            let source_index = workflow
                .steps
                .iter()
                .position(|candidate| candidate.id == *dependency)
                .ok_or_else(|| invalid_run(run_id, "unknown dependency"))?;
            let source = attempts
                .iter()
                .filter(|attempt| {
                    attempt.step_index == source_index && attempt.record.is_completed()
                })
                .max_by_key(|attempt| attempt.number)
                .ok_or_else(|| invalid_run(run_id, "ready Step не имеет completed dependency"))?;
            input.push(source.number);
        }
        let record = AttemptRecord {
            input,
            events: Vec::new(),
        };
        let candidate = DurableAttempt {
            number: next,
            step_index,
            record: record.clone(),
        };
        prepare_agent_input(&guard.directory, workflow, &candidate, run_id)?;
        publish_yaml(
            &guard.directory,
            &attempt_name(next, &step.id),
            &record,
            context,
        )?;
        next = next
            .checked_add(1)
            .ok_or_else(|| invalid_run(run_id, "attempt number overflow"))?;
    }
    Ok(())
}

fn maximum_reserved_attempt_number(
    directory: &Path,
    attempts: &[DurableAttempt],
    run_id: RunId,
) -> Result<u64, CommandError> {
    let mut maximum = attempts
        .iter()
        .map(|attempt| attempt.number)
        .max()
        .unwrap_or(0);
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
            maximum = maximum.max(number);
        }
    }
    if maximum == u64::MAX {
        return Err(invalid_run(run_id, "attempt number overflow"));
    }
    Ok(maximum)
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
    storage: Arc<Mutex<()>>,
}

type ActiveContexts = HashMap<u64, Arc<Mutex<ControlState>>>;

struct ControlHub {
    run_id: RunId,
    contexts: Mutex<ActiveContexts>,
    storage: Arc<Mutex<()>>,
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
    let _storage = lock_storage(&state.storage)?;
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
    fn start(_directory: &Path, hub: Arc<ControlHub>) -> Result<Self, CommandError> {
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
        let thread = thread::spawn(move || serve_control(listener, hub));
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
        let dependencies: HashSet<&str> = step.depends_on.iter().map(String::as_str).collect();
        if dependencies.len() != step.depends_on.len()
            || step
                .depends_on
                .iter()
                .any(|dependency| !ids.contains(dependency.as_str()))
        {
            return Err(invalid_run(
                run_id,
                "depends-on повторяется или ссылается на неизвестный Step",
            ));
        }
    }
    Ok(())
}

fn load_attempts(
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
    if attempts.is_empty()
        || attempts[0].number != 0
        || attempts[0].step_index != 0
        || !attempts[0].record.input.is_empty()
    {
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
    if attempt.record.input.len() != step.depends_on.len() {
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
        .zip(&attempt.record.input)
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
            .and_then(|candidate| candidate.record.input.get(dependency_index))
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

fn prepare_agent_input(
    directory: &Path,
    workflow: &MaterializedWorkflow,
    attempt: &DurableAttempt,
    run_id: RunId,
) -> Result<(Vec<AgentInput>, String), CommandError> {
    let step = &workflow.steps[attempt.step_index];
    let mut inputs = Vec::new();
    for (dependency, source_number) in step.depends_on.iter().zip(&attempt.record.input) {
        let source = workflow
            .steps
            .iter()
            .find(|candidate| candidate.id == *dependency)
            .ok_or_else(|| invalid_run(run_id, "unknown dependency"))?;
        inputs.reserve(source.outputs.len());
        for input_id in &source.outputs {
            inputs.push(AgentInput {
                step_id: dependency.clone(),
                input_id: input_id.clone(),
                path: directory.join(format!("{source_number}.{dependency}.{input_id}.artifact")),
            });
        }
    }
    let prompt = render_prompt(step.prompt.as_deref().unwrap_or(""), &inputs, run_id)?;
    Ok((inputs, prompt))
}

fn render_prompt(
    template: &str,
    inputs: &[AgentInput],
    run_id: RunId,
) -> Result<String, CommandError> {
    let mut output = String::with_capacity(template.len());
    let mut remaining = template;
    while let Some(start) = remaining.find("{{") {
        output.push_str(&remaining[..start]);
        let body_start = start.saturating_add(2);
        let after_start = &remaining[body_start..];
        let end = after_start
            .find("}}")
            .ok_or_else(|| invalid_run(run_id, "незакрытый materialized placeholder"))?;
        let body = &after_start[..end];
        let mut parts = body.split(':');
        let kind = parts.next().unwrap_or_default();
        let step_id = parts.next().unwrap_or_default();
        let input_id = parts.next().unwrap_or_default();
        if parts.next().is_some() {
            return Err(invalid_run(run_id, "невалидный materialized placeholder"));
        }
        let input = inputs
            .iter()
            .find(|input| input.step_id == step_id && input.input_id == input_id)
            .ok_or_else(|| invalid_run(run_id, "placeholder отсутствует в input mapping"))?;
        match kind {
            "path" => {
                output.push_str(input.path.to_str().ok_or_else(|| {
                    invalid_run(run_id, "durable artifact path не является UTF-8")
                })?);
            }
            "content" => {
                let content = fs::read_to_string(&input.path).map_err(|source| {
                    if source.kind() == std::io::ErrorKind::InvalidData {
                        invalid_run(run_id, "artifact для content не является UTF-8")
                    } else {
                        runtime(
                            "run",
                            format!("не удалось прочитать {}", input.path.display()),
                            source,
                        )
                    }
                })?;
                output.push_str(&content);
            }
            _ => return Err(invalid_run(run_id, "неизвестный materialized placeholder")),
        }
        remaining = &after_start[end.saturating_add(2)..];
    }
    output.push_str(remaining);
    Ok(output)
}

fn is_completed(
    workflow: &MaterializedWorkflow,
    attempts: &[DurableAttempt],
) -> Result<bool, CommandError> {
    if attempts
        .iter()
        .any(|attempt| !attempt.record.is_completed())
    {
        return Ok(false);
    }
    let frontier = compute_frontier(workflow, attempts)?;
    Ok(frontier.ready.is_empty() && frontier.missing.is_empty())
}

fn validate_record(
    record: &AttemptRecord,
    step: &MaterializedStep,
    attempt: u64,
    directory: &Path,
    run_id: RunId,
) -> Result<(), CommandError> {
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

fn lock_storage(storage: &Arc<Mutex<()>>) -> Result<std::sync::MutexGuard<'_, ()>, CommandError> {
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

fn register_context(
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

fn unregister_context(hub: &ControlHub, attempt: u64) -> Result<(), CommandError> {
    lock_contexts(&hub.contexts)?.remove(&attempt);
    Ok(())
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

fn blocked(run_id: RunId, context: &str, missing: &[String]) -> CommandError {
    let suffix = if missing.is_empty() {
        String::new()
    } else {
        format!(": отсутствуют source Steps {}", missing.join(", "))
    };
    CommandError::Runtime {
        context: format!("{context}: run {run_id}: blocked{suffix}"),
        source: std::io::Error::other("workflow frontier blocked"),
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

fn report_scheduler(
    outcome: SchedulerOutcome,
    run_id: RunId,
    reporter: &mut dyn LifecycleReporter,
    context: &str,
) -> Result<(), CommandError> {
    match outcome {
        SchedulerOutcome::Completed(true) => {
            report(reporter, &format!("run {run_id}: completed"), context)
        }
        SchedulerOutcome::Completed(false) => Err(blocked(run_id, context, &[])),
        SchedulerOutcome::UserExit => {
            report(
                reporter,
                &format!("run {run_id}: interrupted by user"),
                context,
            )?;
            report(
                reporter,
                &format!("resume: orchestrator resume {run_id}"),
                context,
            )
        }
        SchedulerOutcome::Signal(signal) => Err(CommandError::Interrupted {
            context: format!("run {run_id}: interrupted by SIG{}", signal.name()),
            exit_code: signal.exit_code(),
        }),
    }
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
            storage: Arc::new(Mutex::new(())),
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
