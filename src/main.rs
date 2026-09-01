//! Тонкий CLI-вход: разбирает аргументы, подключает process-зависимости и отображает результат библиотечного API.

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::ffi::OsString;
use std::fs::File;
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::thread;

use clap::{Parser, Subcommand, ValueEnum};
use orchestrator::{
    AgentSessionObserver, CommandError, ConfigCommand, ConfigKey, InspectionFormat,
    InspectionReporter, LifecycleCommand, LifecycleReporter, LifecycleSignals,
    ProcessAgentRegistry, ProcessEnvironment, RunId, RunInspectionState, TerminalMode,
    TerminationSignal, ValidateCommand, execute_agent_list, execute_agent_show, execute_config,
    execute_lifecycle, execute_prompt_list, execute_prompt_show, execute_run_artifacts,
    execute_run_list_formatted, execute_run_show_formatted, execute_run_verify, execute_run_watch,
    execute_validate, execute_validate_all, execute_workflow_graph, execute_workflow_list,
    execute_workflow_plan, execute_workflow_show, open_run_artifact, send_attempt_completion,
    send_session_activation,
};
use signal_hook::consts::signal::{SIGHUP, SIGINT, SIGTERM};
use signal_hook::iterator::Signals;

#[derive(Debug, Parser)]
#[command(name = "orchestrator", disable_help_subcommand = true)]
struct Cli {
    #[command(subcommand)]
    command: TopLevelCommand,
}

#[derive(Debug, Subcommand)]
enum TopLevelCommand {
    Config {
        #[command(subcommand)]
        command: ConfigCliCommand,
    },
    Validate {
        workflow_id: Option<String>,
        #[arg(long, conflicts_with = "workflow_id")]
        all: bool,
        #[arg(long, value_enum, requires = "all")]
        format: Option<InspectionFormatArgument>,
    },
    Start {
        workflow_id: Option<String>,
    },
    Resume {
        run_id: String,
    },
    Run {
        #[command(subcommand)]
        command: RunCliCommand,
    },
    Workflow {
        #[command(subcommand)]
        command: WorkflowCliCommand,
    },
    Agent {
        #[command(subcommand)]
        command: CatalogCliCommand,
    },
    Prompt {
        #[command(subcommand)]
        command: CatalogCliCommand,
    },
    Session {
        #[command(subcommand)]
        command: SessionCliCommand,
    },
    Attempt {
        #[command(subcommand)]
        command: AttemptCliCommand,
    },
}

#[derive(Clone, Debug, Subcommand)]
enum WorkflowCliCommand {
    List {
        #[arg(long, value_enum, default_value_t = InspectionFormatArgument::Text)]
        format: InspectionFormatArgument,
    },
    Show {
        workflow_id: String,
        #[arg(long, value_enum, default_value_t = InspectionFormatArgument::Text)]
        format: InspectionFormatArgument,
    },
    Graph {
        workflow_id: String,
        #[arg(long, value_enum, default_value_t = InspectionFormatArgument::Text)]
        format: InspectionFormatArgument,
    },
    Plan {
        workflow_id: String,
        #[arg(long, value_enum, default_value_t = InspectionFormatArgument::Text)]
        format: InspectionFormatArgument,
    },
}

#[derive(Clone, Debug, Subcommand)]
enum CatalogCliCommand {
    List {
        #[arg(long, value_enum, default_value_t = InspectionFormatArgument::Text)]
        format: InspectionFormatArgument,
    },
    Show {
        source_id: String,
        #[arg(long, value_enum, default_value_t = InspectionFormatArgument::Text)]
        format: InspectionFormatArgument,
    },
}

#[derive(Debug, Subcommand)]
enum RunCliCommand {
    List {
        #[arg(long, value_enum, default_value_t = InspectionFormatArgument::Text)]
        format: InspectionFormatArgument,
        #[arg(long, value_enum, action = clap::ArgAction::Append)]
        state: Vec<RunStateArgument>,
        #[arg(long)]
        workflow: Option<String>,
    },
    Show {
        run_id: String,
        #[arg(long, value_enum, default_value_t = InspectionFormatArgument::Text)]
        format: InspectionFormatArgument,
    },
    Artifacts {
        run_id: String,
        #[arg(long, value_enum, default_value_t = InspectionFormatArgument::Text)]
        format: InspectionFormatArgument,
    },
    Artifact {
        run_id: String,
        attempt: String,
        input_id: String,
    },
    Watch {
        run_id: String,
        #[arg(long, value_enum, default_value_t = InspectionFormatArgument::Text)]
        format: InspectionFormatArgument,
    },
    Verify {
        run_id: Option<String>,
        #[arg(long, value_enum, default_value_t = InspectionFormatArgument::Text)]
        format: InspectionFormatArgument,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum InspectionFormatArgument {
    Text,
    Json,
}

impl From<InspectionFormatArgument> for InspectionFormat {
    fn from(value: InspectionFormatArgument) -> Self {
        match value {
            InspectionFormatArgument::Text => Self::Text,
            InspectionFormatArgument::Json => Self::Json,
        }
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum RunStateArgument {
    Active,
    Blocked,
    Completed,
}

impl From<RunStateArgument> for RunInspectionState {
    fn from(value: RunStateArgument) -> Self {
        match value {
            RunStateArgument::Active => Self::Active,
            RunStateArgument::Blocked => Self::Blocked,
            RunStateArgument::Completed => Self::Completed,
        }
    }
}

#[derive(Debug, Subcommand)]
enum SessionCliCommand {
    Activate { session_id: String },
}

#[derive(Debug, Subcommand)]
enum AttemptCliCommand {
    Complete {
        #[arg(long = "artifact", num_args = 2, action = clap::ArgAction::Append)]
        artifacts: Vec<String>,
    },
}

#[derive(Debug, Subcommand)]
enum ConfigCliCommand {
    Get {
        key: ConfigKeyArgument,
    },
    Set {
        key: ConfigKeyArgument,
        #[arg(allow_hyphen_values = true)]
        value: String,
    },
    List,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum ConfigKeyArgument {
    DefaultWorkflow,
    DefaultAgent,
    MaxParallelAgents,
}

impl From<ConfigKeyArgument> for ConfigKey {
    fn from(value: ConfigKeyArgument) -> Self {
        match value {
            ConfigKeyArgument::DefaultWorkflow => Self::DefaultWorkflow,
            ConfigKeyArgument::DefaultAgent => Self::DefaultAgent,
            ConfigKeyArgument::MaxParallelAgents => Self::MaxParallelAgents,
        }
    }
}

struct StdoutReporter;

impl LifecycleReporter for StdoutReporter {
    fn line(&mut self, value: &str) -> Result<(), io::Error> {
        let mut stdout = io::stdout().lock();
        writeln!(stdout, "{value}")?;
        stdout.flush()
    }
}

impl InspectionReporter for StdoutReporter {
    fn snapshot(&mut self, value: &str) -> Result<(), io::Error> {
        let mut stdout = io::stdout().lock();
        writeln!(stdout, "{value}")?;
        stdout.flush()
    }
}

#[derive(Debug, Default)]
struct AgentBoardState {
    attempts: BTreeMap<u64, AgentBoardRow>,
    humans: BTreeSet<u64>,
}

#[derive(Debug)]
struct AgentBoardRow {
    step_id: String,
    messages: usize,
    last_message: String,
}

#[derive(Debug, Default)]
struct TerminalAgentBoard(Mutex<AgentBoardState>);

impl TerminalAgentBoard {
    fn render(state: &AgentBoardState) {
        if !state.humans.is_empty() || state.attempts.is_empty() {
            return;
        }
        let mut stdout = io::stdout().lock();
        let _ = writeln!(stdout, "\r\x1b[2KAgent sessions:");
        for row in state.attempts.values() {
            let _ = writeln!(
                stdout,
                "\x1b[2K  {} · {} · {}",
                row.step_id, row.messages, row.last_message
            );
        }
        let _ = stdout.flush();
    }
}

impl AgentSessionObserver for TerminalAgentBoard {
    fn started(&self, attempt: u64, step_id: &str, human: bool) {
        let mut state = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if human {
            state.humans.insert(attempt);
        } else {
            state.attempts.insert(
                attempt,
                AgentBoardRow {
                    step_id: step_id.to_owned(),
                    messages: 0,
                    last_message: String::new(),
                },
            );
        }
        Self::render(&state);
    }

    fn message(&self, attempt: u64, step_id: &str, message: &str) {
        let mut state = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let row = state
            .attempts
            .entry(attempt)
            .or_insert_with(|| AgentBoardRow {
                step_id: step_id.to_owned(),
                messages: 0,
                last_message: String::new(),
            });
        row.messages = row.messages.saturating_add(1);
        message.clone_into(&mut row.last_message);
        Self::render(&state);
    }

    fn finished(&self, attempt: u64, _step_id: &str, human: bool) {
        let mut state = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if human {
            state.humans.remove(&attempt);
        } else {
            state.attempts.remove(&attempt);
        }
        Self::render(&state);
    }
}

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            let _ = error.print();
            return ExitCode::from(2);
        }
    };
    let environment = ProcessEnvironment {
        home: env::var_os("HOME"),
        orc_home: env::var_os("ORC_HOME"),
    };
    match dispatch(cli.command, &environment) {
        Ok(CommandOutput::Text(output)) => write_text_output(&output),
        Ok(CommandOutput::ExactText(output)) => write_exact_text_output(&output),
        Ok(CommandOutput::TextWithCode(output, code)) => {
            let written = write_text_output(&output);
            if written == ExitCode::SUCCESS {
                ExitCode::from(code)
            } else {
                written
            }
        }
        Ok(CommandOutput::Artifact(file)) => write_artifact_output(file),
        Ok(CommandOutput::None) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(error.exit_code())
        }
    }
}

enum CommandOutput {
    None,
    Text(String),
    ExactText(String),
    TextWithCode(String, u8),
    Artifact(File),
}

fn write_exact_text_output(output: &str) -> ExitCode {
    let mut stdout = io::stdout().lock();
    match stdout
        .write_all(output.as_bytes())
        .and_then(|()| stdout.flush())
    {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: output: {error}");
            ExitCode::from(1)
        }
    }
}

fn write_text_output(output: &str) -> ExitCode {
    if output.is_empty() {
        return ExitCode::SUCCESS;
    }
    let mut stdout = io::stdout().lock();
    match writeln!(stdout, "{output}").and_then(|()| stdout.flush()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: output: {error}");
            ExitCode::from(1)
        }
    }
}

fn write_artifact_output(mut file: File) -> ExitCode {
    let mut stdout = io::stdout().lock();
    match io::copy(&mut file, &mut stdout).and_then(|_| stdout.flush()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: run artifact: не удалось записать stdout: {error}");
            ExitCode::from(1)
        }
    }
}

fn dispatch(
    command: TopLevelCommand,
    environment: &ProcessEnvironment,
) -> Result<CommandOutput, CommandError> {
    match command {
        TopLevelCommand::Config { command } => {
            dispatch_config(command, environment).map(CommandOutput::Text)
        }
        TopLevelCommand::Validate {
            workflow_id,
            all,
            format,
        } => {
            if all {
                let (output, report) = execute_validate_all(
                    environment,
                    format.unwrap_or(InspectionFormatArgument::Text).into(),
                )?;
                let code = if report.is_valid() { 0 } else { 3 };
                Ok(CommandOutput::TextWithCode(output, code))
            } else {
                workflow_id
                    .map_or_else(
                        || execute_validate(&ValidateCommand::configured_default(), environment),
                        |workflow_id| {
                            ValidateCommand::explicit(&workflow_id)
                                .and_then(|command| execute_validate(&command, environment))
                        },
                    )
                    .map(CommandOutput::Text)
            }
        }
        TopLevelCommand::Start { workflow_id } => {
            let command = workflow_id.map_or_else(
                || Ok(LifecycleCommand::start_configured_default()),
                |workflow_id| LifecycleCommand::start_explicit(&workflow_id),
            )?;
            run_lifecycle(&command, environment)?;
            Ok(CommandOutput::None)
        }
        TopLevelCommand::Resume { run_id } => {
            run_lifecycle(
                &LifecycleCommand::Resume(RunId::parse(&run_id)?),
                environment,
            )?;
            Ok(CommandOutput::None)
        }
        TopLevelCommand::Run { command } => dispatch_run(command, environment),
        TopLevelCommand::Workflow { command } => dispatch_workflow(command, environment),
        TopLevelCommand::Agent { command } => {
            dispatch_catalog(CatalogKind::Agent, command, environment)
        }
        TopLevelCommand::Prompt { command } => {
            dispatch_catalog(CatalogKind::Prompt, command, environment)
        }
        TopLevelCommand::Session {
            command: SessionCliCommand::Activate { session_id },
        } => {
            let (endpoint, run_id, attempt) = control_context("session activate")?;
            send_session_activation(&endpoint, run_id, attempt, session_id)?;
            Ok(CommandOutput::None)
        }
        TopLevelCommand::Attempt {
            command: AttemptCliCommand::Complete { artifacts },
        } => {
            let (endpoint, run_id, attempt) = control_context("attempt complete")?;
            let artifacts = artifacts
                .chunks_exact(2)
                .map(|pair| (pair[0].clone(), PathBuf::from(&pair[1])))
                .collect();
            send_attempt_completion(&endpoint, run_id, attempt, artifacts)?;
            Ok(CommandOutput::None)
        }
    }
}

#[derive(Clone, Copy)]
enum CatalogKind {
    Agent,
    Prompt,
}

fn dispatch_workflow(
    command: WorkflowCliCommand,
    environment: &ProcessEnvironment,
) -> Result<CommandOutput, CommandError> {
    match command {
        WorkflowCliCommand::List { format } => {
            execute_workflow_list(environment, format.into()).map(CommandOutput::Text)
        }
        WorkflowCliCommand::Show {
            workflow_id,
            format,
        } => {
            execute_workflow_show(&workflow_id, environment, format.into()).map(CommandOutput::Text)
        }
        WorkflowCliCommand::Graph {
            workflow_id,
            format,
        } => execute_workflow_graph(&workflow_id, environment, format.into())
            .map(CommandOutput::Text),
        WorkflowCliCommand::Plan {
            workflow_id,
            format,
        } => {
            execute_workflow_plan(&workflow_id, environment, format.into()).map(CommandOutput::Text)
        }
    }
}

fn dispatch_catalog(
    kind: CatalogKind,
    command: CatalogCliCommand,
    environment: &ProcessEnvironment,
) -> Result<CommandOutput, CommandError> {
    match command {
        CatalogCliCommand::List { format } => match kind {
            CatalogKind::Agent => execute_agent_list(environment, format.into()),
            CatalogKind::Prompt => execute_prompt_list(environment, format.into()),
        }
        .map(CommandOutput::Text),
        CatalogCliCommand::Show { source_id, format } => match kind {
            CatalogKind::Agent => {
                execute_agent_show(&source_id, environment, format.into()).map(CommandOutput::Text)
            }
            CatalogKind::Prompt => {
                let output = execute_prompt_show(&source_id, environment, format.into())?;
                Ok(match format {
                    InspectionFormatArgument::Text => CommandOutput::ExactText(output),
                    InspectionFormatArgument::Json => CommandOutput::Text(output),
                })
            }
        },
    }
}

fn dispatch_run(
    command: RunCliCommand,
    environment: &ProcessEnvironment,
) -> Result<CommandOutput, CommandError> {
    match command {
        RunCliCommand::List {
            format,
            state,
            workflow,
        } => {
            let states = state.into_iter().map(Into::into).collect::<Vec<_>>();
            execute_run_list_formatted(environment, format.into(), &states, workflow.as_deref())
                .map(CommandOutput::Text)
        }
        RunCliCommand::Show { run_id, format } => execute_run_show_formatted(
            parse_inspection_run_id(&run_id, "run show")?,
            environment,
            format.into(),
        )
        .map(CommandOutput::Text),
        RunCliCommand::Artifacts { run_id, format } => execute_run_artifacts(
            parse_inspection_run_id(&run_id, "run artifacts")?,
            environment,
            format.into(),
        )
        .map(CommandOutput::Text),
        RunCliCommand::Artifact {
            run_id,
            attempt,
            input_id,
        } => {
            let run_id = parse_inspection_run_id(&run_id, "run artifact")?;
            let attempt = attempt.parse::<u64>().map_err(|_| CommandError::Syntax {
                context: format!(
                    "run artifact: attempt '{attempt}' должен быть десятичным integer"
                ),
            })?;
            open_run_artifact(run_id, attempt, &input_id, environment).map(CommandOutput::Artifact)
        }
        RunCliCommand::Watch { run_id, format } => {
            let signals = LifecycleSignals::default();
            install_signal_listener(&signals)?;
            execute_run_watch(
                parse_inspection_run_id(&run_id, "run watch")?,
                environment,
                format.into(),
                &signals,
                &mut StdoutReporter,
            )?;
            Ok(CommandOutput::None)
        }
        RunCliCommand::Verify { run_id, format } => {
            let run_id = run_id
                .as_deref()
                .map(|value| parse_inspection_run_id(value, "run verify"))
                .transpose()?;
            let (output, report) = execute_run_verify(run_id, environment, format.into())?;
            let code = if report.is_valid() { 0 } else { 3 };
            Ok(CommandOutput::TextWithCode(output, code))
        }
    }
}

fn parse_inspection_run_id(value: &str, command: &str) -> Result<RunId, CommandError> {
    RunId::parse(value).map_err(|error| match error {
        CommandError::Syntax { context } => CommandError::Syntax {
            context: context.replacen("resume:", &format!("{command}:"), 1),
        },
        other => other,
    })
}

fn run_lifecycle(
    command: &LifecycleCommand,
    environment: &ProcessEnvironment,
) -> Result<(), CommandError> {
    let registry = env::var_os("ORC_AGENT_COMMAND")
        .map_or_else(
            || Ok(ProcessAgentRegistry::system()),
            ProcessAgentRegistry::new,
        )
        .map_err(|context| CommandError::Invalid {
            context: format!("lifecycle: {context}"),
        })?;
    let terminal = if io::stdin().is_terminal() && io::stdout().is_terminal() {
        TerminalMode::Available
    } else {
        TerminalMode::Unavailable
    };
    let registry = if terminal == TerminalMode::Available {
        registry.with_session_observer(Arc::new(TerminalAgentBoard::default()))
    } else {
        registry
    };
    let signals = LifecycleSignals::default();
    install_signal_listener(&signals)?;
    execute_lifecycle(
        command,
        environment,
        terminal,
        &signals,
        &registry,
        &mut StdoutReporter,
    )
}

fn install_signal_listener(state: &LifecycleSignals) -> Result<(), CommandError> {
    let mut signals =
        Signals::new([SIGHUP, SIGINT, SIGTERM]).map_err(|source| CommandError::Runtime {
            context: "lifecycle: не удалось установить signal handlers".to_owned(),
            source,
        })?;
    let state = state.clone();
    thread::Builder::new()
        .name("orchestrator-signals".to_owned())
        .spawn(move || {
            for signal in signals.forever() {
                let signal = match signal {
                    SIGHUP => TerminationSignal::Hangup,
                    SIGINT => TerminationSignal::Interrupt,
                    SIGTERM => TerminationSignal::Terminate,
                    _ => continue,
                };
                state.notify(signal);
            }
        })
        .map(|_| ())
        .map_err(|source| CommandError::Runtime {
            context: "lifecycle: не удалось запустить signal listener".to_owned(),
            source,
        })
}

fn control_context(context: &str) -> Result<(PathBuf, RunId, u64), CommandError> {
    let endpoint = required_environment("ORC_CONTROL_ENDPOINT", context)?;
    let run_id = required_environment("ORC_RUN_ID", context)?;
    let attempt = required_environment("ORC_ATTEMPT", context)?;
    let run_id = RunId::parse(&run_id.to_string_lossy()).map_err(|_| CommandError::Busy {
        context: format!("{context}: невалидный ORC_RUN_ID в control context"),
    })?;
    let attempt = attempt
        .to_string_lossy()
        .parse::<u64>()
        .map_err(|_| CommandError::Busy {
            context: format!("{context}: невалидный ORC_ATTEMPT в control context"),
        })?;
    Ok((PathBuf::from(endpoint), run_id, attempt))
}

fn required_environment(key: &str, context: &str) -> Result<OsString, CommandError> {
    env::var_os(key).ok_or_else(|| CommandError::Busy {
        context: format!("{context}: {key} отсутствует в control context"),
    })
}

fn dispatch_config(
    command: ConfigCliCommand,
    environment: &ProcessEnvironment,
) -> Result<String, CommandError> {
    match command {
        ConfigCliCommand::Get { key } => {
            execute_config(&ConfigCommand::Get(key.into()), environment)
        }
        ConfigCliCommand::Set { key, value } => match key {
            ConfigKeyArgument::MaxParallelAgents => ConfigCommand::set_max_parallel_agents(&value)
                .and_then(|command| execute_config(&command, environment)),
            ConfigKeyArgument::DefaultWorkflow => ConfigCommand::set_default_workflow(&value)
                .and_then(|command| execute_config(&command, environment)),
            ConfigKeyArgument::DefaultAgent => ConfigCommand::set_default_agent(&value)
                .and_then(|command| execute_config(&command, environment)),
        },
        ConfigCliCommand::List => execute_config(&ConfigCommand::List, environment),
    }
}
