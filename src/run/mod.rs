//! Публичный lifecycle run; scheduling, execution, storage, inspection и control изолированы в дочерних модулях, а Agent protocol остаётся в adapter.

mod control;
mod executor;
mod inspection;
mod scheduler;
mod storage;

use std::collections::BTreeMap;
#[cfg(test)]
use std::fs;
use std::sync::Arc;
#[cfg(test)]
use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use crate::agent::{AgentRegistry, TerminationSignal};
use crate::config::{CommandError, ProcessEnvironment, resolve_state_root};
use crate::domain::{AttemptRecord, MaterializedWorkflow, SymbolicId};
use crate::workflow::{ValidateCommand, materialize_for_lifecycle};

pub use crate::domain::RunId;
pub use control::{send_attempt_completion, send_session_activation};
pub use inspection::{
    ArtifactInspection, AttemptInspection, AttemptInspectionState, FrontierInspection,
    InspectionFormat, InspectionReporter, RunInspection, RunInspectionState, RunVerification,
    StepInspection, VerificationReport, execute_run_artifacts, execute_run_list,
    execute_run_list_formatted, execute_run_show, execute_run_show_formatted, execute_run_verify,
    execute_run_watch, inspect_run, inspect_runs, open_run_artifact,
};

use scheduler::{SchedulerOutcome, execute_scheduler, is_completed};
use storage::{
    RunGuard, attempt_name, load_attempts, publish_yaml, read_yaml, reserve_run,
    validate_materialized,
};

#[cfg(test)]
use control::{ControlState, accept_completion, lock_state};
#[cfg(test)]
use inspection::load_inspected_run_with_hook;
#[cfg(test)]
use storage::publish_bytes_with_hook;

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
    let record = AttemptRecord::pending(Vec::new());
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

fn valid_id(value: &str) -> bool {
    SymbolicId::is_valid(value)
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
            record: AttemptRecord::pending(Vec::new()),
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
