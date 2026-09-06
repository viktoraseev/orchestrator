//! Выполнение одного Agent или Process attempt; модуль не выбирает runnable frontier и не публикует новые attempts.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use super::control::{
    CompletionCandidate, ControlHandle, ControlHub, ControlState, finalize_completion, lock_state,
    lock_storage, register_context, unregister_context,
};
use super::storage::RunGuard;
use super::{TEMP_SEQUENCE, invalid_run, runtime};
use crate::agent::{AgentCancellation, AgentInput, AgentRegistry, AgentRunRequest, wait_for_child};
use crate::config::CommandError;
use crate::domain::{
    DurableAttempt, MaterializedProcess, MaterializedStep, MaterializedWorkflow, RunId,
};

#[derive(Clone, Copy)]
pub(super) struct AttemptExecution<'a> {
    pub(super) guard: &'a RunGuard,
    pub(super) run_id: RunId,
    pub(super) workflow: &'a MaterializedWorkflow,
    pub(super) registry: &'a dyn AgentRegistry,
    pub(super) hub: &'a Arc<ControlHub>,
    pub(super) endpoint: &'a Path,
    pub(super) cancellation: &'a AgentCancellation,
}

pub(super) enum AttemptOutcome {
    Returned(bool),
    UserExit,
}

pub(super) fn run_current_attempt(
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

pub(super) fn prepare_agent_input(
    directory: &Path,
    workflow: &MaterializedWorkflow,
    attempt: &DurableAttempt,
    run_id: RunId,
) -> Result<(Vec<AgentInput>, String), CommandError> {
    let step = &workflow.steps[attempt.step_index];
    let mut inputs = Vec::new();
    for (dependency, source_number) in step.depends_on.iter().zip(attempt.record.input()) {
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
