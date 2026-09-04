//! Библиотечная часть orchestrator; CLI использует её типизированные операции и не содержит продуктовой логики.

pub mod agent;
pub mod catalog;
pub mod config;
pub mod run;
pub mod workflow;

pub use agent::{
    AgentCancellation, AgentExit, AgentInput, AgentRegistry, AgentRunRequest, AgentSessionObserver,
    AttemptControl, BuiltinAgentRegistry, ProcessAgentRegistry, TerminationSignal,
};
pub use catalog::{
    AgentCatalogEntry, PromptCatalogEntry, PromptTemplate, SourceWorkflow, SourceWorkflowProcess,
    SourceWorkflowStep, WorkflowCatalogEntry, execute_agent_list, execute_agent_show,
    execute_prompt_list, execute_prompt_show, execute_workflow_list, execute_workflow_show,
    list_agents, list_prompts, list_workflows, show_agent, show_prompt, show_workflow,
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
pub use workflow::{
    ValidateCommand, WorkflowGraph, WorkflowGraphEdge, WorkflowPlan, WorkflowPlanAgent,
    WorkflowPlanProcess, WorkflowPlanStep, WorkflowValidation, WorkflowValidationReport,
    build_workflow_graph, build_workflow_plan, build_workflow_plan_with_registry, execute_validate,
    execute_validate_all, execute_validate_with_registry, execute_workflow_graph,
    execute_workflow_plan, validate_all_workflows, validate_all_workflows_with_registry,
};
