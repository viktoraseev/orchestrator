//! Библиотечная часть orchestrator; CLI использует её типизированные операции и не содержит продуктовой логики.

pub mod agent;
pub mod config;
pub mod workflow;

pub use agent::{AgentRegistry, BuiltinAgentRegistry};
pub use config::{CommandError, ConfigCommand, ConfigKey, ProcessEnvironment, execute_config};
pub use workflow::{ValidateCommand, execute_validate, execute_validate_with_registry};
