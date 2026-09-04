//! Claude adapter владеет построением команды и интерпретацией stream-json протокола этого Agent type.

use std::process::Command;

use serde::Deserialize;

use super::{AgentProtocolEvent, AgentRunRequest, ProcessAgentType};

#[derive(Debug)]
struct ClaudeAgentType;

static CLAUDE_AGENT_TYPE: ClaudeAgentType = ClaudeAgentType;

pub(super) fn agent_type() -> &'static dyn ProcessAgentType {
    &CLAUDE_AGENT_TYPE
}

impl ProcessAgentType for ClaudeAgentType {
    fn executable(&self) -> &'static str {
        "claude"
    }

    fn configure(&self, command: &mut Command, request: &AgentRunRequest<'_>) {
        if !request.human {
            command.args(["--print", "--output-format", "stream-json", "--verbose"]);
        }
        command.args(["--model", request.model, "--effort", request.reasoning]);
        if let Some(session_id) = request.resume_session {
            command.args(["--resume", session_id]);
        }
        command.arg(request.prompt);
    }

    fn parse_line(&self, line: &str) -> Result<Option<AgentProtocolEvent>, String> {
        let event: ClaudeEvent = serde_json::from_str(line)
            .map_err(|error| format!("claude protocol содержит невалидный JSON: {error}"))?;
        Ok(match event {
            ClaudeEvent::System {
                subtype: ClaudeSystemSubtype::Init,
                session_id: Some(session_id),
            } => Some(AgentProtocolEvent::SessionStarted(session_id)),
            ClaudeEvent::Assistant { message } => {
                let text = message
                    .content
                    .into_iter()
                    .filter_map(|block| match block {
                        ClaudeContent::Text { text } => Some(text),
                        ClaudeContent::Other => None,
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                (!text.is_empty()).then_some(AgentProtocolEvent::Message(text))
            }
            ClaudeEvent::System { .. } | ClaudeEvent::Other => None,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum ClaudeEvent {
    #[serde(rename = "system")]
    System {
        subtype: ClaudeSystemSubtype,
        #[serde(default)]
        session_id: Option<String>,
    },
    #[serde(rename = "assistant")]
    Assistant { message: ClaudeMessage },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ClaudeSystemSubtype {
    Init,
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
struct ClaudeMessage {
    content: Vec<ClaudeContent>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum ClaudeContent {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(other)]
    Other,
}
