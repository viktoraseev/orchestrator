//! Registry Agent type проверяет совместимость Agent; graph и config не знают допустимые type, model и reasoning.

/// Registry встроенных или тестовых Agent type.
pub trait AgentRegistry {
    /// Проверяет существование type, совместимость model и reasoning и поддержку native resume.
    ///
    /// # Errors
    ///
    /// Возвращает диагностику несовместимости Agent; caller добавляет контекст config или Step.
    fn validate(&self, type_id: &str, model: &str, reasoning: &str) -> Result<(), String>;
}

/// Registry type IDs, встроенных в бинарник orchestrator.
#[derive(Clone, Copy, Debug, Default)]
pub struct BuiltinAgentRegistry;

impl AgentRegistry for BuiltinAgentRegistry {
    fn validate(&self, type_id: &str, model: &str, reasoning: &str) -> Result<(), String> {
        if !matches!(type_id, "codex" | "claude") {
            return Err(format!("неизвестный Agent type '{type_id}'"));
        }
        if model.is_empty() {
            return Err("model не может быть пустым".to_owned());
        }
        if reasoning.is_empty() {
            return Err("reasoning не может быть пустым".to_owned());
        }
        Ok(())
    }
}
