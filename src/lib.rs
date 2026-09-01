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
    ArtifactInspection, AttemptInspection, AttemptInspectionState, FrontierInspection,
    InspectionFormat, InspectionReporter, LifecycleCommand, LifecycleReporter, LifecycleSignals,
    RunId, RunInspection, RunInspectionState, RunVerification, StepInspection, TerminalMode,
    VerificationReport, execute_lifecycle, execute_run_artifacts, execute_run_list,
    execute_run_list_formatted, execute_run_show, execute_run_show_formatted, execute_run_verify,
    execute_run_watch, inspect_run, inspect_runs, open_run_artifact, send_attempt_completion,
    send_session_activation,
};
pub use workflow::{ValidateCommand, execute_validate, execute_validate_with_registry};
