//! Lifecycle run, storage и control boundary; модуль не разбирает CLI и не знает протокол конкретного Agent type.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as FmtWrite;
use std::fs::{self, File, OpenOptions};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU8, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fs2::FileExt;
use serde::{Deserialize, Serialize};

use crate::agent::{
    AgentCancellation, AgentInput, AgentRegistry, AgentRunRequest, AttemptControl,
    BuiltinAgentRegistry, TerminationSignal, wait_for_child,
};
use crate::config::{CommandError, ProcessEnvironment, RawAgent, resolve_state_root};
use crate::workflow::{SymbolicId, ValidateCommand, Workflow, materialize_for_lifecycle};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);
const INSPECTION_SNAPSHOT_ATTEMPTS: usize = 4;
const INSPECTION_WATCH_INTERVAL: Duration = Duration::from_millis(100);

/// Команда публичного lifecycle API после разбора CLI.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LifecycleCommand {
    /// Создать новый run выбранного workflow.
    Start {
        /// Способ выбора source workflow.
        selection: ValidateCommand,
        /// Явные значения объявленных run parameters.
        parameters: BTreeMap<String, String>,
    },
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
        Self::start_explicit_with_parameters(value, BTreeMap::new())
    }

    /// Создаёт `start` с явно выбранным workflow и run parameters.
    ///
    /// # Errors
    ///
    /// Возвращает [`CommandError::Syntax`], если `WorkflowId` не соответствует kebab-case.
    pub fn start_explicit_with_parameters(
        value: &str,
        parameters: BTreeMap<String, String>,
    ) -> Result<Self, CommandError> {
        ValidateCommand::explicit(value)
            .map(|selection| Self::Start {
                selection,
                parameters,
            })
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
        Self::Start {
            selection: ValidateCommand::ConfiguredDefault,
            parameters: BTreeMap::new(),
        }
    }

    /// Создаёт `start` с workflow из config default и run parameters.
    #[must_use]
    pub const fn start_configured_default_with_parameters(
        parameters: BTreeMap<String, String>,
    ) -> Self {
        Self::Start {
            selection: ValidateCommand::ConfiguredDefault,
            parameters,
        }
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

/// Формат представления read-only inspection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum InspectionFormat {
    /// Стабильное человекочитаемое представление.
    Text,
    /// Документ JSON по схеме `cli.md`.
    Json,
}

/// Вычисленное состояние validated durable run.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "lowercase")]
pub enum RunInspectionState {
    /// Run содержит незавершённый attempt или ready frontier.
    Active,
    /// Run валиден, но не имеет запускаемой работы.
    Blocked,
    /// Все достижимые Steps завершены.
    Completed,
}

impl RunInspectionState {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Blocked => "blocked",
            Self::Completed => "completed",
        }
    }
}

/// Typed read model одного согласованного durable snapshot.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct RunInspection {
    run_id: u64,
    workflow: String,
    state: RunInspectionState,
    steps: Vec<StepInspection>,
    attempts: Vec<AttemptInspection>,
    frontier: FrontierInspection,
    artifacts: Vec<ArtifactInspection>,
}

impl RunInspection {
    /// Возвращает `RunId` snapshot.
    #[must_use]
    pub const fn run_id(&self) -> u64 {
        self.run_id
    }

    /// Возвращает materialized `WorkflowId`.
    #[must_use]
    pub fn workflow(&self) -> &str {
        &self.workflow
    }

    /// Возвращает вычисленное состояние run.
    #[must_use]
    pub const fn state(&self) -> RunInspectionState {
        self.state
    }

    /// Возвращает опубликованные artifacts в deterministic durable order.
    #[must_use]
    pub fn artifacts(&self) -> &[ArtifactInspection] {
        &self.artifacts
    }

    /// Возвращает Steps в materialized order.
    #[must_use]
    pub fn steps(&self) -> &[StepInspection] {
        &self.steps
    }

    /// Возвращает attempts в порядке глобального номера.
    #[must_use]
    pub fn attempts(&self) -> &[AttemptInspection] {
        &self.attempts
    }

    /// Возвращает вычисленный frontier.
    #[must_use]
    pub const fn frontier(&self) -> &FrontierInspection {
        &self.frontier
    }
}

/// Step в typed inspection read model.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct StepInspection {
    id: String,
    attempts: Vec<u64>,
}

impl StepInspection {
    /// Возвращает `StepId`.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Возвращает глобальные номера attempts этого Step.
    #[must_use]
    pub fn attempts(&self) -> &[u64] {
        &self.attempts
    }
}

/// Attempt в typed inspection read model.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct AttemptInspection {
    number: u64,
    step: String,
    state: AttemptInspectionState,
    session: Option<String>,
    input: Vec<u64>,
}

impl AttemptInspection {
    /// Возвращает глобальный номер attempt.
    #[must_use]
    pub const fn number(&self) -> u64 {
        self.number
    }

    /// Возвращает `StepId` attempt.
    #[must_use]
    pub fn step(&self) -> &str {
        &self.step
    }

    /// Возвращает вычисленное состояние attempt.
    #[must_use]
    pub const fn state(&self) -> AttemptInspectionState {
        self.state
    }

    /// Возвращает последнюю durable session activation.
    #[must_use]
    pub fn session(&self) -> Option<&str> {
        self.session.as_deref()
    }

    /// Возвращает input mapping attempt.
    #[must_use]
    pub fn input(&self) -> &[u64] {
        &self.input
    }
}

/// Вычисленное состояние attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "lowercase")]
pub enum AttemptInspectionState {
    /// Completion ещё не опубликован.
    Active,
    /// Терминальное completion-событие опубликовано.
    Completed,
}

/// Frontier typed inspection read model.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct FrontierInspection {
    ready: Vec<String>,
    missing: Vec<String>,
}

impl FrontierInspection {
    /// Возвращает ready `StepId` в workflow order.
    #[must_use]
    pub fn ready(&self) -> &[String] {
        &self.ready
    }

    /// Возвращает недостающие dependencies в deterministic order.
    #[must_use]
    pub fn missing(&self) -> &[String] {
        &self.missing
    }
}

/// Дескриптор опубликованной версии durable artifact.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct ArtifactInspection {
    attempt: u64,
    step: String,
    input: String,
    bytes: u64,
    path: String,
}

impl ArtifactInspection {
    /// Возвращает глобальный номер attempt.
    #[must_use]
    pub const fn attempt(&self) -> u64 {
        self.attempt
    }

    /// Возвращает `StepId` publisher'а.
    #[must_use]
    pub fn step(&self) -> &str {
        &self.step
    }

    /// Возвращает опубликованный `InputId`.
    #[must_use]
    pub fn input(&self) -> &str {
        &self.input
    }

    /// Возвращает размер durable artifact.
    #[must_use]
    pub const fn bytes(&self) -> u64 {
        self.bytes
    }

    /// Возвращает абсолютный durable path в строковом JSON-compatible виде.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }
}

/// Одна строка полного отчёта `run verify`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct RunVerification {
    run_id: u64,
    valid: bool,
    diagnostics: Vec<String>,
}

/// Полный отчёт read-only validation одного или нескольких runs.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct VerificationReport {
    runs: Vec<RunVerification>,
}

impl VerificationReport {
    /// Возвращает `true`, если каждый проверенный run валиден.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.runs.iter().all(|run| run.valid)
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

/// Выполняет `start` или `resume` через одну lifecycle, storage и Step executor границу.
///
/// См. Rules в `features/lifecycle.feature`; storage commit attempt состоит в атомарной замене record, а `resume` читает только полностью опубликованную durable-модель.
///
/// # Errors
///
/// Возвращает категории 2/3/4 до создания run, 5 для занятого run и 1 для I/O, executor failure или возврата Agent без completion.
pub fn execute_lifecycle(
    command: &LifecycleCommand,
    environment: &ProcessEnvironment,
    terminal: TerminalMode,
    signals: &LifecycleSignals,
    registry: &dyn AgentRegistry,
    reporter: &mut dyn LifecycleReporter,
) -> Result<(), CommandError> {
    match command {
        LifecycleCommand::Start {
            selection,
            parameters,
        } => start(
            selection,
            parameters,
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

/// Строит typed snapshots всех числовых durable runs до применения фильтров.
///
/// # Errors
///
/// Возвращает ошибку чтения или validation любого run; частичный список не возвращается.
pub fn inspect_runs(environment: &ProcessEnvironment) -> Result<Vec<RunInspection>, CommandError> {
    let root = resolve_state_root(environment, "run list")?;
    inspect_runs_at_root(&root, "run list")
}

/// Строит typed snapshot одного durable run.
///
/// # Errors
///
/// Возвращает `4` для неизвестного run, `3` для стабильной противоречивой модели и runtime error, если согласованный snapshot не удалось получить за ограниченное число попыток.
pub fn inspect_run(
    run_id: RunId,
    environment: &ProcessEnvironment,
) -> Result<RunInspection, CommandError> {
    let root = resolve_state_root(environment, "run show")?;
    load_inspected_run(&root, run_id, "run show")?.to_public("run show")
}

/// Выполняет `run list` с форматом и validated фильтрами.
///
/// Все runs проверяются до фильтрации, поэтому скрытая corruption не маскируется.
///
/// # Errors
///
/// Возвращает `2` для невалидного `WorkflowId`, ошибку любого durable run или сериализации результата.
pub fn execute_run_list_formatted(
    environment: &ProcessEnvironment,
    format: InspectionFormat,
    states: &[RunInspectionState],
    workflow: Option<&str>,
) -> Result<String, CommandError> {
    if workflow.is_some_and(|value| !valid_id(value)) {
        return Err(CommandError::Syntax {
            context: format!(
                "run list: WorkflowId '{}' не соответствует kebab-case",
                workflow.unwrap_or_default()
            ),
        });
    }
    let runs = inspect_runs(environment)?;
    let filtered = runs
        .into_iter()
        .filter(|run| states.is_empty() || states.contains(&run.state))
        .filter(|run| workflow.is_none_or(|value| run.workflow == value))
        .collect::<Vec<_>>();
    render_run_list(&filtered, format)
}

fn inspect_runs_at_root(root: &Path, context: &str) -> Result<Vec<RunInspection>, CommandError> {
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
    run_ids
        .into_iter()
        .map(|run_id| load_inspected_run(root, run_id, context)?.to_public(context))
        .collect()
}

fn render_run_list(
    runs: &[RunInspection],
    format: InspectionFormat,
) -> Result<String, CommandError> {
    match format {
        InspectionFormat::Text => {
            let mut output = String::new();
            for run in runs {
                writeln!(
                    output,
                    "run {}: workflow={} state={}",
                    run.run_id,
                    run.workflow,
                    run.state.as_str()
                )
                .map_err(formatting_error)?;
            }
            if output.ends_with('\n') {
                output.pop();
            }
            Ok(output)
        }
        InspectionFormat::Json => serde_json::to_string_pretty(runs).map_err(json_formatting_error),
    }
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

/// Выполняет `run show` выбранным renderer'ом.
///
/// # Errors
///
/// Возвращает ошибки consistent snapshot либо JSON serialization.
pub fn execute_run_show_formatted(
    run_id: RunId,
    environment: &ProcessEnvironment,
    format: InspectionFormat,
) -> Result<String, CommandError> {
    let run = inspect_run(run_id, environment)?;
    render_run_show(&run, format, false)
}

/// Перечисляет дескрипторы artifacts опубликованных completed attempts.
///
/// # Errors
///
/// Возвращает ошибки consistent snapshot, metadata artifact либо JSON serialization.
pub fn execute_run_artifacts(
    run_id: RunId,
    environment: &ProcessEnvironment,
    format: InspectionFormat,
) -> Result<String, CommandError> {
    let root = resolve_state_root(environment, "run artifacts")?;
    let run = load_inspected_run(&root, run_id, "run artifacts")?.to_public("run artifacts")?;
    render_artifacts(&run.artifacts, format)
}

/// Приёмник полных snapshots команды `run watch`.
pub trait InspectionReporter {
    /// Делает один validated snapshot наблюдаемым до ожидания следующего.
    ///
    /// # Errors
    ///
    /// Возвращает ошибку вывода, после которой watch прекращается.
    fn snapshot(&mut self, value: &str) -> Result<(), std::io::Error>;
}

/// Наблюдает active run до terminal state или termination signal.
///
/// Initial snapshot публикуется немедленно; последующие одинаковые snapshots не публикуются. См. Rule «Run watch публикует только изменившиеся snapshots» в `features/run_inspection.feature`.
///
/// # Errors
///
/// Возвращает ошибки consistent snapshot и reporter, либо [`CommandError::Interrupted`] с кодом первого signal.
pub fn execute_run_watch(
    run_id: RunId,
    environment: &ProcessEnvironment,
    format: InspectionFormat,
    signals: &LifecycleSignals,
    reporter: &mut dyn InspectionReporter,
) -> Result<(), CommandError> {
    let mut previous = inspect_run(run_id, environment)?;
    report_inspection_snapshot(
        reporter,
        &render_run_show(&previous, format, true)?,
        "run watch",
    )?;
    loop {
        if previous.state != RunInspectionState::Active {
            return Ok(());
        }
        if let Some((signal, _)) = signals.observation() {
            return Err(CommandError::Interrupted {
                context: format!("run watch: прерван signal {}", signal.number()),
                exit_code: signal.exit_code(),
            });
        }
        thread::park_timeout(INSPECTION_WATCH_INTERVAL);
        let current = inspect_run(run_id, environment)?;
        if current != previous {
            report_inspection_snapshot(
                reporter,
                &render_run_show(&current, format, true)?,
                "run watch",
            )?;
            previous = current;
        }
    }
}

/// Проверяет один или все durable runs и возвращает полный deterministic report.
///
/// Stable validation errors становятся diagnostics отчёта; I/O errors прерывают команду.
///
/// # Errors
///
/// Возвращает `4` для неизвестного явно выбранного run и runtime error для I/O или непрерывно меняющегося snapshot.
pub fn execute_run_verify(
    run_id: Option<RunId>,
    environment: &ProcessEnvironment,
    format: InspectionFormat,
) -> Result<(String, VerificationReport), CommandError> {
    let root = resolve_state_root(environment, "run verify")?;
    let run_ids = match run_id {
        Some(run_id) => {
            let directory = root.join("run").join(run_id.to_string());
            if !directory.is_dir() {
                return Err(CommandError::NotFound {
                    context: format!("run verify: run {run_id} не существует"),
                });
            }
            vec![run_id]
        }
        None => numeric_run_ids(&root, "run verify")?,
    };
    let mut runs = Vec::with_capacity(run_ids.len());
    for run_id in run_ids {
        match load_inspected_run(&root, run_id, "run verify")
            .and_then(|run| run.to_public("run verify"))
        {
            Ok(_) => runs.push(RunVerification {
                run_id: run_id.0,
                valid: true,
                diagnostics: Vec::new(),
            }),
            Err(CommandError::Invalid { context }) => runs.push(RunVerification {
                run_id: run_id.0,
                valid: false,
                diagnostics: vec![context],
            }),
            Err(error) => return Err(error),
        }
    }
    let report = VerificationReport { runs };
    let output = match format {
        InspectionFormat::Text => {
            let mut output = String::new();
            for run in &report.runs {
                if run.valid {
                    writeln!(output, "run {}: valid", run.run_id).map_err(formatting_error)?;
                } else {
                    for diagnostic in &run.diagnostics {
                        writeln!(output, "run {}: invalid: {diagnostic}", run.run_id)
                            .map_err(formatting_error)?;
                    }
                }
            }
            if output.ends_with('\n') {
                output.pop();
            }
            output
        }
        InspectionFormat::Json => {
            serde_json::to_string_pretty(&report).map_err(json_formatting_error)?
        }
    };
    Ok((output, report))
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
    #[serde(default)]
    parameters: BTreeMap<String, String>,
    steps: Vec<MaterializedStep>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct MaterializedStep {
    id: String,
    agent: Option<RawAgent>,
    prompt: Option<String>,
    human: bool,
    process: Option<MaterializedProcess>,
    depends_on: Vec<String>,
    outputs: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct MaterializedProcess {
    executable: PathBuf,
    args: Vec<String>,
    cwd: PathBuf,
    stdout: Option<String>,
}

impl MaterializedWorkflow {
    fn from_workflow(workflow: Workflow, parameters: BTreeMap<String, String>) -> Self {
        Self {
            workflow_id: workflow.id.as_str().to_owned(),
            max_parallel_agents: workflow.max_parallel_agents,
            parameters,
            steps: workflow
                .steps
                .into_iter()
                .map(|step| MaterializedStep {
                    id: step.id.as_str().to_owned(),
                    agent: step.agent,
                    prompt: step.prompt,
                    human: step.human,
                    process: step.process.map(|process| MaterializedProcess {
                        executable: process.executable,
                        args: process.args,
                        cwd: process.cwd,
                        stdout: process.stdout.map(|id| id.as_str().to_owned()),
                    }),
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

fn validate_run_parameters(
    declared: &[SymbolicId],
    provided: &BTreeMap<String, String>,
) -> Result<(), CommandError> {
    if let Some(parameter) = provided
        .iter()
        .find_map(|(id, value)| value.contains('\0').then_some(id))
    {
        return Err(CommandError::Invalid {
            context: format!("start: значение ParameterId '{parameter}' содержит NUL"),
        });
    }
    if let Some(parameter) = declared
        .iter()
        .find(|id| !provided.contains_key(id.as_str()))
    {
        return Err(CommandError::Invalid {
            context: format!(
                "start: обязательный ParameterId '{}' не передан",
                parameter.as_str()
            ),
        });
    }
    if let Some(parameter) = provided
        .keys()
        .find(|id| !declared.iter().any(|declared| declared.as_str() == *id))
    {
        return Err(CommandError::Invalid {
            context: format!("start: ParameterId '{parameter}' не объявлен workflow"),
        });
    }
    Ok(())
}

fn start(
    selection: &ValidateCommand,
    parameters: &BTreeMap<String, String>,
    environment: &ProcessEnvironment,
    terminal: TerminalMode,
    signals: &LifecycleSignals,
    registry: &dyn AgentRegistry,
    reporter: &mut dyn LifecycleReporter,
) -> Result<(), CommandError> {
    let root = resolve_state_root(environment, "start")?;
    let workflow = materialize_for_lifecycle(selection, environment, registry, "start")?;
    validate_run_parameters(&workflow.parameters, parameters)?;
    let candidate = MaterializedWorkflow::from_workflow(workflow, parameters.clone());
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
    if let Some(process) = &step.process {
        return run_process_attempt(execution, attempt, step, process);
    }
    let agent = step.agent.as_ref().ok_or_else(|| {
        invalid_run(
            run_id,
            "materialized Step не содержит ни Agent, ни Process executor",
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
            type_id: &agent.r#type,
            model: &agent.model,
            reasoning: &agent.reasoning,
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

fn run_process_attempt(
    execution: &AttemptExecution<'_>,
    attempt: &DurableAttempt,
    step: &MaterializedStep,
    process: &MaterializedProcess,
) -> Result<AttemptOutcome, CommandError> {
    let (inputs, _) = prepare_agent_input(
        &execution.guard.directory,
        execution.workflow,
        attempt,
        execution.run_id,
    )?;
    let output_paths = process_output_paths(
        &execution.guard.directory,
        attempt.number,
        step,
        execution.run_id,
    )?;
    let args = process
        .args
        .iter()
        .map(|argument| {
            render_process_argument(
                argument,
                &execution.workflow.parameters,
                &inputs,
                &output_paths,
                execution.run_id,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut command = build_process_command(
        execution,
        attempt,
        step,
        process,
        &args,
        &inputs,
        &output_paths,
    )?;
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(source) => {
            cleanup_process_outputs(&output_paths);
            return Err(CommandError::Runtime {
                context: format!(
                    "run {}: не удалось запустить Process '{}'",
                    execution.run_id,
                    process.executable.display()
                ),
                source,
            });
        }
    };
    let status = match wait_for_child(&mut child, "Process", execution.cancellation) {
        Ok(status) => status,
        Err(context) => {
            cleanup_process_outputs(&output_paths);
            return Err(CommandError::Runtime {
                context: format!("run {}: {context}", execution.run_id),
                source: std::io::Error::other("Process supervision failure"),
            });
        }
    };
    if !status.success() {
        cleanup_process_outputs(&output_paths);
        return Err(CommandError::Runtime {
            context: format!(
                "run {}: Process завершился с кодом {}",
                execution.run_id,
                status
                    .code()
                    .map_or_else(|| "signal".to_owned(), |code| code.to_string())
            ),
            source: std::io::Error::other("Process failure"),
        });
    }
    let candidate = read_process_outputs(execution.run_id, step, &output_paths);
    cleanup_process_outputs(&output_paths);
    let candidate = candidate?;
    let _storage = lock_storage(&execution.hub.storage)?;
    finalize_completion(&execution.guard.directory, attempt.number, step, candidate)?;
    Ok(AttemptOutcome::Returned(true))
}

fn build_process_command(
    execution: &AttemptExecution<'_>,
    attempt: &DurableAttempt,
    step: &MaterializedStep,
    process: &MaterializedProcess,
    args: &[String],
    inputs: &[AgentInput],
    output_paths: &BTreeMap<String, PathBuf>,
) -> Result<Command, CommandError> {
    let inputs = serde_yaml::to_string(inputs).map_err(|source| CommandError::Runtime {
        context: format!(
            "run {}: не удалось сериализовать Process inputs",
            execution.run_id
        ),
        source: std::io::Error::other(source),
    })?;
    let outputs = serde_yaml::to_string(output_paths).map_err(|source| CommandError::Runtime {
        context: format!(
            "run {}: не удалось сериализовать Process outputs",
            execution.run_id
        ),
        source: std::io::Error::other(source),
    })?;
    let mut command = Command::new(&process.executable);
    command
        .args(args)
        .current_dir(&process.cwd)
        .env("ORC_STEP_ID", &step.id)
        .env("ORC_RUN_ID", execution.run_id.to_string())
        .env("ORC_ATTEMPT", attempt.number.to_string())
        .env("ORC_INPUT", inputs)
        .env("ORC_OUTPUT", outputs)
        .env_remove("ORC_CONTROL_ENDPOINT")
        .env_remove("ORC_RESUME_SESSION")
        .stdin(Stdio::null())
        .stderr(Stdio::inherit());
    if let Some(stdout) = &process.stdout {
        let path = output_paths
            .get(stdout)
            .ok_or_else(|| invalid_run(execution.run_id, "Process stdout output отсутствует"))?;
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|source| {
                runtime(
                    "run",
                    format!("не удалось создать Process stdout {}", path.display()),
                    source,
                )
            })?;
        command.stdout(Stdio::from(file));
    } else {
        command.stdout(Stdio::inherit());
    }
    command.process_group(0);
    Ok(command)
}

fn process_output_paths(
    directory: &Path,
    attempt: u64,
    step: &MaterializedStep,
    run_id: RunId,
) -> Result<BTreeMap<String, PathBuf>, CommandError> {
    if step.outputs.is_empty() {
        return Ok(BTreeMap::new());
    }
    let staging = loop {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let candidate = directory.join(format!(
            ".{attempt}.{}.process.{}.{sequence}.tmp",
            step.id,
            std::process::id()
        ));
        match fs::create_dir(&candidate) {
            Ok(()) => break candidate,
            Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(source) => {
                return Err(CommandError::Runtime {
                    context: format!("run {run_id}: не удалось создать Process staging directory"),
                    source,
                });
            }
        }
    };
    fs::set_permissions(&staging, fs::Permissions::from_mode(0o700)).map_err(|source| {
        CommandError::Runtime {
            context: format!("run {run_id}: не удалось защитить Process staging directory"),
            source,
        }
    })?;
    Ok(step
        .outputs
        .iter()
        .map(|output| (output.clone(), staging.join(output)))
        .collect())
}

fn render_process_argument(
    template: &str,
    parameters: &BTreeMap<String, String>,
    inputs: &[AgentInput],
    outputs: &BTreeMap<String, PathBuf>,
    run_id: RunId,
) -> Result<String, CommandError> {
    if !template.contains("{{") && !template.contains("}}") {
        return Ok(template.to_owned());
    }
    let body = template
        .strip_prefix("{{")
        .and_then(|value| value.strip_suffix("}}"))
        .ok_or_else(|| invalid_run(run_id, "Process placeholder не занимает весь argv element"))?;
    let parts = body.split(':').collect::<Vec<_>>();
    match parts.as_slice() {
        ["param", parameter_id] => parameters
            .get(*parameter_id)
            .cloned()
            .ok_or_else(|| invalid_run(run_id, "Process parameter отсутствует в spec.yaml")),
        ["path", step_id, input_id] => inputs
            .iter()
            .find(|input| input.step_id == *step_id && input.input_id == *input_id)
            .ok_or_else(|| invalid_run(run_id, "Process path отсутствует в input mapping"))?
            .path
            .to_str()
            .map(str::to_owned)
            .ok_or_else(|| invalid_run(run_id, "Process input path не является UTF-8")),
        ["output", input_id] => outputs
            .get(*input_id)
            .ok_or_else(|| invalid_run(run_id, "Process output отсутствует в mapping"))?
            .to_str()
            .map(str::to_owned)
            .ok_or_else(|| invalid_run(run_id, "Process output path не является UTF-8")),
        _ => Err(invalid_run(run_id, "невалидный Process placeholder")),
    }
}

fn read_process_outputs(
    run_id: RunId,
    step: &MaterializedStep,
    paths: &BTreeMap<String, PathBuf>,
) -> Result<CompletionCandidate, CommandError> {
    let mut outputs = BTreeMap::new();
    for output in &step.outputs {
        let path = paths
            .get(output)
            .ok_or_else(|| invalid_run(run_id, "Process output mapping неполон"))?;
        let metadata = fs::metadata(path).map_err(|source| CommandError::Runtime {
            context: format!("run {run_id}: Process не создал обязательный output '{output}'"),
            source,
        })?;
        if !metadata.is_file() {
            return Err(CommandError::Runtime {
                context: format!(
                    "run {run_id}: Process output '{}' не является regular file",
                    path.display()
                ),
                source: std::io::Error::other("invalid Process output"),
            });
        }
        let bytes = fs::read(path).map_err(|source| {
            runtime(
                "run",
                format!("не удалось прочитать Process output {}", path.display()),
                source,
            )
        })?;
        outputs.insert(output.clone(), bytes);
    }
    Ok(CompletionCandidate(outputs))
}

fn cleanup_process_outputs(paths: &BTreeMap<String, PathBuf>) {
    let staging = paths
        .values()
        .next()
        .and_then(|path| path.parent())
        .map(Path::to_owned);
    for path in paths.values() {
        let _ = fs::remove_file(path);
    }
    if let Some(staging) = staging {
        let _ = fs::remove_dir(staging);
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
    let has_agent_steps = workflow.steps.iter().any(|step| step.agent.is_some());
    let server = (has_agent_steps && registry.uses_process_control())
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

impl InspectedRun {
    fn state(&self) -> RunInspectionState {
        if self
            .attempts
            .iter()
            .any(|attempt| !attempt.record.is_completed())
            || !self.frontier.ready.is_empty()
        {
            RunInspectionState::Active
        } else if self.frontier.missing.is_empty() {
            RunInspectionState::Completed
        } else {
            RunInspectionState::Blocked
        }
    }

    fn to_public(&self, context: &str) -> Result<RunInspection, CommandError> {
        let steps = self
            .workflow
            .steps
            .iter()
            .enumerate()
            .map(|(step_index, step)| StepInspection {
                id: step.id.clone(),
                attempts: self
                    .attempts
                    .iter()
                    .filter(|attempt| attempt.step_index == step_index)
                    .map(|attempt| attempt.number)
                    .collect(),
            })
            .collect();
        let attempts = self
            .attempts
            .iter()
            .map(|attempt| AttemptInspection {
                number: attempt.number,
                step: self.workflow.steps[attempt.step_index].id.clone(),
                state: if attempt.record.is_completed() {
                    AttemptInspectionState::Completed
                } else {
                    AttemptInspectionState::Active
                },
                session: attempt.record.last_session().map(str::to_owned),
                input: attempt.record.input.clone(),
            })
            .collect();
        let frontier = FrontierInspection {
            ready: self
                .frontier
                .ready
                .iter()
                .map(|step_index| self.workflow.steps[*step_index].id.clone())
                .collect(),
            missing: self.frontier.missing.clone(),
        };
        let mut artifacts = Vec::new();
        for attempt in self
            .attempts
            .iter()
            .filter(|attempt| attempt.record.is_completed())
        {
            let step = &self.workflow.steps[attempt.step_index];
            for input in &step.outputs {
                let path = self
                    .directory
                    .join(format!("{}.{}.{input}.artifact", attempt.number, step.id));
                let bytes = path
                    .metadata()
                    .map_err(|source| {
                        runtime(
                            context,
                            format!("не удалось прочитать metadata {}", path.display()),
                            source,
                        )
                    })?
                    .len();
                artifacts.push(ArtifactInspection {
                    attempt: attempt.number,
                    step: step.id.clone(),
                    input: input.clone(),
                    bytes,
                    path: path.to_string_lossy().into_owned(),
                });
            }
        }
        Ok(RunInspection {
            run_id: self.run_id.0,
            workflow: self.workflow.workflow_id.clone(),
            state: self.state(),
            steps,
            attempts,
            frontier,
            artifacts,
        })
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
    load_inspected_run_with_hook(&directory, run_id, context, || {})
}

fn load_inspected_run_with_hook(
    directory: &Path,
    run_id: RunId,
    context: &str,
    mut after_fingerprint: impl FnMut(),
) -> Result<InspectedRun, CommandError> {
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
        let result = load_inspected_run_once(directory, run_id, context);
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

fn load_inspected_run_once(
    directory: &Path,
    run_id: RunId,
    context: &str,
) -> Result<InspectedRun, CommandError> {
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
    let frontier =
        compute_frontier(&workflow, &attempts).map_err(|error| recontextualize(error, context))?;
    Ok(InspectedRun {
        run_id,
        directory: directory.to_owned(),
        workflow,
        attempts,
        frontier,
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

fn render_run_show(
    run: &RunInspection,
    format: InspectionFormat,
    compact_json: bool,
) -> Result<String, CommandError> {
    match format {
        InspectionFormat::Text => {
            let mut output = String::new();
            writeln!(
                output,
                "run {}: workflow={} state={}",
                run.run_id,
                run.workflow,
                run.state.as_str()
            )
            .map_err(formatting_error)?;
            for step in &run.steps {
                let attempts = step.attempts.iter().map(u64::to_string).collect::<Vec<_>>();
                writeln!(
                    output,
                    "step {}: attempts={}",
                    step.id,
                    joined_or_dash(&attempts)
                )
                .map_err(formatting_error)?;
            }
            for attempt in &run.attempts {
                let input = attempt.input.iter().map(u64::to_string).collect::<Vec<_>>();
                writeln!(
                    output,
                    "attempt {}: step={} state={} session={} input={}",
                    attempt.number,
                    attempt.step,
                    match attempt.state {
                        AttemptInspectionState::Active => "active",
                        AttemptInspectionState::Completed => "completed",
                    },
                    attempt.session.as_deref().unwrap_or("-"),
                    joined_or_dash(&input)
                )
                .map_err(formatting_error)?;
            }
            writeln!(
                output,
                "frontier: ready={} missing={}",
                joined_or_dash(&run.frontier.ready),
                joined_or_dash(&run.frontier.missing)
            )
            .map_err(formatting_error)?;
            output.pop();
            Ok(output)
        }
        InspectionFormat::Json if compact_json => {
            serde_json::to_string(run).map_err(json_formatting_error)
        }
        InspectionFormat::Json => serde_json::to_string_pretty(run).map_err(json_formatting_error),
    }
}

fn render_artifacts(
    artifacts: &[ArtifactInspection],
    format: InspectionFormat,
) -> Result<String, CommandError> {
    match format {
        InspectionFormat::Text => {
            let mut output = String::new();
            for artifact in artifacts {
                writeln!(
                    output,
                    "artifact {}: step={} input={} bytes={} path={}",
                    artifact.attempt, artifact.step, artifact.input, artifact.bytes, artifact.path
                )
                .map_err(formatting_error)?;
            }
            if output.ends_with('\n') {
                output.pop();
            }
            Ok(output)
        }
        InspectionFormat::Json => {
            serde_json::to_string_pretty(artifacts).map_err(json_formatting_error)
        }
    }
}

fn numeric_run_ids(root: &Path, context: &str) -> Result<Vec<RunId>, CommandError> {
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
    let mut ids = Vec::new();
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
            ids.push(RunId(value));
        }
    }
    ids.sort_by_key(|id| id.0);
    Ok(ids)
}

fn report_inspection_snapshot(
    reporter: &mut dyn InspectionReporter,
    value: &str,
    context: &str,
) -> Result<(), CommandError> {
    reporter
        .snapshot(value)
        .map_err(|source| CommandError::Runtime {
            context: format!("{context}: не удалось записать snapshot"),
            source,
        })
}

fn json_formatting_error(source: serde_json::Error) -> CommandError {
    CommandError::Runtime {
        context: "run inspection: не удалось сериализовать JSON".to_owned(),
        source: std::io::Error::other(source),
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
        || workflow
            .parameters
            .iter()
            .any(|(id, value)| !valid_id(id) || value.contains('\0'))
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
        match (&step.agent, &step.process) {
            (Some(agent), None) => registry
                .validate(&agent.r#type, &agent.model, &agent.reasoning)
                .map_err(|context| invalid_run(run_id, &context))?,
            (None, Some(process)) => {
                if step.human
                    || step.prompt.is_some()
                    || !process.executable.is_absolute()
                    || !process.cwd.is_absolute()
                    || process
                        .stdout
                        .as_ref()
                        .is_some_and(|id| !step.outputs.contains(id))
                {
                    return Err(invalid_run(run_id, "невалидный materialized Process Step"));
                }
            }
            _ => {
                return Err(invalid_run(
                    run_id,
                    "Step должен содержать ровно один Agent или Process executor",
                ));
            }
        }
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
        if let Some(process) = &step.process {
            for argument in &process.args {
                validate_materialized_process_argument(workflow, step, argument, run_id)?;
            }
        }
    }
    Ok(())
}

fn validate_materialized_process_argument(
    workflow: &MaterializedWorkflow,
    step: &MaterializedStep,
    argument: &str,
    run_id: RunId,
) -> Result<(), CommandError> {
    if argument.contains('\0') {
        return Err(invalid_run(run_id, "Process argv содержит NUL"));
    }
    if !argument.contains("{{") && !argument.contains("}}") {
        return Ok(());
    }
    let body = argument
        .strip_prefix("{{")
        .and_then(|value| value.strip_suffix("}}"))
        .ok_or_else(|| invalid_run(run_id, "Process placeholder не занимает весь argv element"))?;
    let parts = body.split(':').collect::<Vec<_>>();
    let valid = match parts.as_slice() {
        ["param", parameter] => workflow.parameters.contains_key(*parameter),
        ["path", source_step, input_id] => {
            step.depends_on
                .iter()
                .any(|dependency| dependency == source_step)
                && workflow.steps.iter().any(|source| {
                    source.id == *source_step
                        && source.outputs.iter().any(|output| output == input_id)
                })
        }
        ["output", output] => step.outputs.iter().any(|candidate| candidate == output),
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(invalid_run(
            run_id,
            "невалидный materialized Process placeholder",
        ))
    }
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
    use std::cell::Cell;
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

    #[test]
    fn inspection_retries_snapshot_changed_between_fingerprints() {
        let root = TempDir::new().expect("test root must be created");
        let spec_path = root.path().join("spec.yaml");
        let spec = |workflow: &str| {
            format!(
                "workflow-id: {workflow}\nmax-parallel-agents: 1\nsteps:\n- id: first\n  agent:\n    type: codex\n    model: model\n    reasoning: high\n  prompt: null\n  human: false\n  depends-on: []\n  outputs: []\n"
            )
        };
        fs::write(&spec_path, spec("before")).expect("initial spec must be written");
        fs::write(
            root.path().join("0.first.attempt.yaml"),
            b"input: []\nevents: []\n",
        )
        .expect("attempt must be written");
        let calls = Cell::new(0_usize);

        let run = load_inspected_run_with_hook(root.path(), RunId(1), "test", || {
            let call = calls.get();
            calls.set(call + 1);
            if call == 0 {
                fs::write(&spec_path, spec("after")).expect("replacement spec must be written");
            }
        })
        .expect("changed snapshot must be retried");

        assert_eq!(run.workflow.workflow_id, "after");
        assert_eq!(calls.get(), 2);
    }

    #[test]
    fn inspection_stops_after_bounded_continuous_changes() {
        let root = TempDir::new().expect("test root must be created");
        let spec_path = root.path().join("spec.yaml");
        let spec = |workflow: &str| {
            format!(
                "workflow-id: {workflow}\nmax-parallel-agents: 1\nsteps:\n- id: first\n  agent:\n    type: codex\n    model: model\n    reasoning: high\n  prompt: null\n  human: false\n  depends-on: []\n  outputs: []\n"
            )
        };
        fs::write(&spec_path, spec("even")).expect("initial spec must be written");
        fs::write(
            root.path().join("0.first.attempt.yaml"),
            b"input: []\nevents: []\n",
        )
        .expect("attempt must be written");
        let calls = Cell::new(0_usize);

        let result = load_inspected_run_with_hook(root.path(), RunId(1), "test", || {
            let call = calls.get() + 1;
            calls.set(call);
            let workflow = if call.is_multiple_of(2) {
                "even"
            } else {
                "odd"
            };
            fs::write(&spec_path, spec(workflow)).expect("changing spec must be written");
        });
        let Err(error) = result else {
            panic!("continuously changing snapshot must fail");
        };

        assert!(matches!(error, CommandError::Runtime { .. }));
        assert_eq!(calls.get(), INSPECTION_SNAPSHOT_ATTEMPTS);
    }
}
