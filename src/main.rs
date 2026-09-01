//! Тонкий CLI-вход: разбирает аргументы, подключает process-зависимости и отображает результат библиотечного API.

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::ffi::OsString;
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::thread;

use clap::{Parser, Subcommand, ValueEnum};
use orchestrator::{
    AgentSessionObserver, CommandError, ConfigCommand, ConfigKey, LifecycleCommand,
    LifecycleReporter, LifecycleSignals, ProcessAgentRegistry, ProcessEnvironment, RunId,
    TerminalMode, TerminationSignal, ValidateCommand, execute_config, execute_lifecycle,
    execute_validate, send_attempt_completion, send_session_activation,
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
    },
    Start {
        workflow_id: Option<String>,
    },
    Resume {
        run_id: String,
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
        Ok(Some(output)) => {
            println!("{output}");
            ExitCode::SUCCESS
        }
        Ok(None) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(error.exit_code())
        }
    }
}

fn dispatch(
    command: TopLevelCommand,
    environment: &ProcessEnvironment,
) -> Result<Option<String>, CommandError> {
    match command {
        TopLevelCommand::Config { command } => dispatch_config(command, environment).map(Some),
        TopLevelCommand::Validate { workflow_id } => workflow_id
            .map_or_else(
                || execute_validate(&ValidateCommand::configured_default(), environment),
                |workflow_id| {
                    ValidateCommand::explicit(&workflow_id)
                        .and_then(|command| execute_validate(&command, environment))
                },
            )
            .map(Some),
        TopLevelCommand::Start { workflow_id } => {
            let command = workflow_id.map_or_else(
                || Ok(LifecycleCommand::start_configured_default()),
                |workflow_id| LifecycleCommand::start_explicit(&workflow_id),
            )?;
            run_lifecycle(&command, environment)?;
            Ok(None)
        }
        TopLevelCommand::Resume { run_id } => {
            run_lifecycle(
                &LifecycleCommand::Resume(RunId::parse(&run_id)?),
                environment,
            )?;
            Ok(None)
        }
        TopLevelCommand::Session {
            command: SessionCliCommand::Activate { session_id },
        } => {
            let (endpoint, run_id, attempt) = control_context("session activate")?;
            send_session_activation(&endpoint, run_id, attempt, session_id)?;
            Ok(None)
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
            Ok(None)
        }
    }
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
