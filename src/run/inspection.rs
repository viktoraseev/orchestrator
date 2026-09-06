//! Read-only представление согласованного durable run; модуль не изменяет состояние и не запускает executors.

use std::fmt::Write as FmtWrite;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::thread;

use serde::Serialize;

use super::scheduler::{Frontier, compute_frontier};
#[cfg(test)]
use super::storage::load_run_snapshot_with_hook;
use super::storage::{RunSnapshot, artifact_size, list_run_ids, load_run_snapshot, open_artifact};
use super::{INSPECTION_WATCH_INTERVAL, LifecycleSignals, valid_id};
use crate::config::{CommandError, ProcessEnvironment, resolve_state_root};
use crate::domain::{DurableAttempt, MaterializedWorkflow, RunId};

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
    pub(super) const fn as_str(self) -> &'static str {
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
    pub(super) run_id: u64,
    pub(super) workflow: String,
    pub(super) state: RunInspectionState,
    pub(super) steps: Vec<StepInspection>,
    pub(super) attempts: Vec<AttemptInspection>,
    pub(super) frontier: FrontierInspection,
    pub(super) artifacts: Vec<ArtifactInspection>,
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
    pub(super) id: String,
    pub(super) attempts: Vec<u64>,
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
    pub(super) number: u64,
    pub(super) step: String,
    pub(super) state: AttemptInspectionState,
    pub(super) session: Option<String>,
    pub(super) input: Vec<u64>,
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
    pub(super) ready: Vec<String>,
    pub(super) missing: Vec<String>,
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
    pub(super) attempt: u64,
    pub(super) step: String,
    pub(super) input: String,
    pub(super) bytes: u64,
    pub(super) path: String,
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
    pub(super) run_id: u64,
    pub(super) valid: bool,
    pub(super) diagnostics: Vec<String>,
}

/// Полный отчёт read-only validation одного или нескольких runs.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct VerificationReport {
    pub(super) runs: Vec<RunVerification>,
}

impl VerificationReport {
    /// Возвращает `true`, если каждый проверенный run валиден.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.runs.iter().all(|run| run.valid)
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
    let run_ids = list_run_ids(&root, "run list")?;
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
    list_run_ids(root, context)?
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
            .input()
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
        None => list_run_ids(&root, "run verify")?,
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
    open_artifact(&path)
}

pub(super) struct InspectedRun {
    run_id: RunId,
    directory: PathBuf,
    pub(super) workflow: MaterializedWorkflow,
    attempts: Vec<DurableAttempt>,
    frontier: Frontier,
}

impl InspectedRun {
    fn from_snapshot(snapshot: RunSnapshot, context: &str) -> Result<Self, CommandError> {
        let frontier = compute_frontier(&snapshot.workflow, &snapshot.attempts)
            .map_err(|error| recontextualize(error, context))?;
        Ok(Self {
            run_id: snapshot.run_id,
            directory: snapshot.directory,
            workflow: snapshot.workflow,
            attempts: snapshot.attempts,
            frontier,
        })
    }

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
                input: attempt.record.input().to_vec(),
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
                let bytes = artifact_size(&path, context)?;
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
    InspectedRun::from_snapshot(load_run_snapshot(root, run_id, context)?, context)
}

#[cfg(test)]
pub(super) fn load_inspected_run_with_hook(
    directory: &Path,
    run_id: RunId,
    context: &str,
    after_fingerprint: impl FnMut(),
) -> Result<InspectedRun, CommandError> {
    let snapshot = load_run_snapshot_with_hook(directory, run_id, context, after_fingerprint)?;
    InspectedRun::from_snapshot(snapshot, context)
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
