//! Тонкий CLI-вход: разбирает аргументы, подключает окружение процесса и отображает результат библиотеки.

use std::env;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use orchestrator::{
    ConfigCommand, ConfigKey, ProcessEnvironment, ValidateCommand, execute_config, execute_validate,
};

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
    let result = match cli.command {
        TopLevelCommand::Config { command } => match command {
            ConfigCliCommand::Get { key } => {
                execute_config(&ConfigCommand::Get(key.into()), &environment)
            }
            ConfigCliCommand::Set { key, value } => match key {
                ConfigKeyArgument::MaxParallelAgents => {
                    match ConfigCommand::set_max_parallel_agents(&value) {
                        Ok(command) => execute_config(&command, &environment),
                        Err(error) => Err(error),
                    }
                }
                ConfigKeyArgument::DefaultWorkflow => {
                    match ConfigCommand::set_default_workflow(&value) {
                        Ok(command) => execute_config(&command, &environment),
                        Err(error) => Err(error),
                    }
                }
                ConfigKeyArgument::DefaultAgent => match ConfigCommand::set_default_agent(&value) {
                    Ok(command) => execute_config(&command, &environment),
                    Err(error) => Err(error),
                },
            },
            ConfigCliCommand::List => execute_config(&ConfigCommand::List, &environment),
        },
        TopLevelCommand::Validate { workflow_id } => workflow_id.map_or_else(
            || execute_validate(&ValidateCommand::configured_default(), &environment),
            |workflow_id| {
                ValidateCommand::explicit(&workflow_id)
                    .and_then(|command| execute_validate(&command, &environment))
            },
        ),
    };

    match result {
        Ok(output) => {
            println!("{output}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(error.exit_code())
        }
    }
}
