//! Выбор runnable frontier и supervision batch attempts; модуль не исполняет Agent protocol и не реализует storage.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;

use super::control::{ControlHub, ControlServer, lock_storage};
use super::executor::{AttemptExecution, AttemptOutcome, prepare_agent_input, run_current_attempt};
use super::storage::{
    RunGuard, attempt_name, load_attempts, maximum_reserved_attempt_number, publish_yaml,
};
use super::{LifecycleSignals, TerminalMode, blocked, invalid_run};
use crate::agent::{AgentCancellation, AgentRegistry, TerminationSignal};
use crate::config::CommandError;
use crate::domain::{AttemptRecord, DurableAttempt, MaterializedWorkflow, RunId};

#[derive(Clone, Copy)]
pub(super) enum SchedulerOutcome {
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

/// Ведёт run до terminal outcome, публикуя каждый новый attempt до его запуска; см. Rules «Fan-out и fan-in frontier вычисляется из durable attempts» и «Non-human attempts выполняются параллельно под общим лимитом» в `features/graph_execution.feature`.
pub(super) fn execute_scheduler(
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

pub(super) struct Frontier {
    pub(super) ready: Vec<usize>,
    pub(super) missing: Vec<String>,
}

/// Вычисляет ready и частично удовлетворённые dependency groups только из validated durable attempts; см. Rule «Fan-out и fan-in frontier вычисляется из durable attempts» в `features/graph_execution.feature`.
pub(super) fn compute_frontier(
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
                previous.and_then(|attempt| attempt.record.input().get(dependency_index).copied());
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
        let record = AttemptRecord::pending(input);
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

pub(super) fn is_completed(
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
