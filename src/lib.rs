//! Библиотечная часть orchestrator; CLI использует её типизированные операции и не содержит продуктовой логики.

pub mod agent;
pub mod config;
pub mod run;
pub mod workflow;

pub use agent::{
    AgentCancellation, AgentExit, AgentInput, AgentRegistry, AgentRunRequest, AgentSessionObserver,
    AttemptControl, BuiltinAgentRegistry, ProcessAgentRegistry, TerminationSignal,
};
pub use config::{CommandError, ConfigCommand, ConfigKey, ProcessEnvironment, execute_config};
pub use run::{
    LifecycleCommand, LifecycleReporter, LifecycleSignals, RunId, TerminalMode, execute_lifecycle,
    send_attempt_completion, send_session_activation,
};
pub use workflow::{ValidateCommand, execute_validate, execute_validate_with_registry};
