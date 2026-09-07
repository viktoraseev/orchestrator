//! Доменные агрегаты workflow, run и agent attempt; модуль не выполняет I/O, orchestration и process-вызовы.

mod agent;
mod attempt;
mod expression;
mod graph;
pub(crate) use expression::{Dependencies, Dependency, Expression, Outputs};
pub(crate) use graph::{Graph, GraphStep};
mod run;
mod workflow;

pub(crate) use agent::Agent;
pub use agent::AgentId;
pub(crate) use attempt::{AttemptRecord, DurableAttempt};
pub use run::RunId;
pub(crate) use run::{MaterializedProcess, MaterializedStep, MaterializedWorkflow};
pub use workflow::WorkflowId;
pub(crate) use workflow::{ProcessStep, Step, SymbolicId, Workflow};
