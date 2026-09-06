//! Agent attempt и его append-only события; модуль не управляет процессом агента и durable-файлами.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AttemptRecord {
    input: Vec<u64>,
    events: Vec<AttemptEvent>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) enum AttemptEvent {
    SessionActivated {
        #[serde(rename = "session-id")]
        session_id: String,
    },
    Completed,
}

impl AttemptRecord {
    pub(crate) fn pending(input: Vec<u64>) -> Self {
        Self {
            input,
            events: Vec::new(),
        }
    }

    pub(crate) fn input(&self) -> &[u64] {
        &self.input
    }

    pub(crate) fn activate_session(&mut self, session_id: String) {
        self.events
            .push(AttemptEvent::SessionActivated { session_id });
    }

    pub(crate) fn complete(&mut self) {
        self.events.push(AttemptEvent::Completed);
    }

    pub(crate) fn is_completed(&self) -> bool {
        matches!(self.events.last(), Some(AttemptEvent::Completed))
    }

    pub(crate) fn last_session(&self) -> Option<&str> {
        self.events.iter().rev().find_map(|event| match event {
            AttemptEvent::SessionActivated { session_id } => Some(session_id.as_str()),
            AttemptEvent::Completed => None,
        })
    }

    pub(crate) fn validate_history(&self) -> Result<(), &'static str> {
        let mut last_session = None;
        for (index, event) in self.events.iter().enumerate() {
            match event {
                AttemptEvent::SessionActivated { session_id } => {
                    if last_session == Some(session_id.as_str()) {
                        return Err("повтор последней session activation");
                    }
                    last_session = Some(session_id.as_str());
                }
                AttemptEvent::Completed if index + 1 != self.events.len() => {
                    return Err("completed не является последним событием");
                }
                AttemptEvent::Completed => {}
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub(crate) struct DurableAttempt {
    pub(crate) number: u64,
    pub(crate) step_index: usize,
    pub(crate) record: AttemptRecord,
}
