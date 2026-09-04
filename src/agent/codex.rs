//! Codex adapter владеет построением команды и интерпретацией JSONL-протокола этого Agent type.

use std::process::Command;

use serde::Deserialize;

use super::{AgentProtocolEvent, AgentRunRequest, ProcessAgentType};

#[derive(Debug)]
struct CodexAgentType;

static CODEX_AGENT_TYPE: CodexAgentType = CodexAgentType;

pub(super) fn agent_type() -> &'static dyn ProcessAgentType {
    &CODEX_AGENT_TYPE
}

impl ProcessAgentType for CodexAgentType {
    fn executable(&self) -> &'static str {
        "codex"
    }

    fn configure(&self, command: &mut Command, request: &AgentRunRequest<'_>) {
        let reasoning = format!("model_reasoning_effort=\"{}\"", request.reasoning);
        if request.human {
            if let Some(session_id) = request.resume_session {
                command.arg("resume");
                command.args(["--model", request.model, "--config", &reasoning]);
                command.args([session_id, request.prompt]);
            } else {
                command.args(["--model", request.model, "--config", &reasoning]);
                command.arg(request.prompt);
            }
        } else {
            command.arg("exec");
            if request.resume_session.is_some() {
                command.arg("resume");
            }
            command.args(["--json", "--model", request.model, "--config", &reasoning]);
            if let Some(session_id) = request.resume_session {
                command.arg(session_id);
            }
            command.arg(request.prompt);
        }
    }

    fn parse_line(&self, line: &str) -> Result<Option<AgentProtocolEvent>, String> {
        let event: CodexEvent = serde_json::from_str(line)
            .map_err(|error| format!("codex protocol содержит невалидный JSON: {error}"))?;
        Ok(match event {
            CodexEvent::ThreadStarted { thread_id } => {
                Some(AgentProtocolEvent::SessionStarted(thread_id))
            }
            CodexEvent::ItemCompleted {
                item: CodexItem::AgentMessage { text },
            } => Some(AgentProtocolEvent::Message(text)),
            CodexEvent::ItemCompleted {
                item: CodexItem::Other,
            }
            | CodexEvent::Other => None,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum CodexEvent {
    #[serde(rename = "thread.started")]
    ThreadStarted { thread_id: String },
    #[serde(rename = "item.completed")]
    ItemCompleted { item: CodexItem },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum CodexItem {
    #[serde(rename = "agent_message")]
    AgentMessage { text: String },
    #[serde(other)]
    Other,
}
