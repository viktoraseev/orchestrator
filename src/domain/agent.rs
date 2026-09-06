//! Effective Agent, выбранный для Step; модуль не запускает Agent type и не интерпретирует его протокол.

use serde::{Deserialize, Deserializer, Serialize, de};

use super::SymbolicId;

/// Проверенный идентификатор именованного Agent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentId(String);

impl AgentId {
    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        SymbolicId::parse("AgentId", value).map(|id| Self(id.into_string()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Agent {
    #[serde(deserialize_with = "deserialize_string")]
    pub(crate) r#type: String,
    #[serde(deserialize_with = "deserialize_string")]
    pub(crate) model: String,
    #[serde(deserialize_with = "deserialize_string")]
    pub(crate) reasoning: String,
}

fn deserialize_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    match serde_yaml::Value::deserialize(deserializer)? {
        serde_yaml::Value::String(value) => Ok(value),
        value => Err(de::Error::custom(format!("expected string, got {value:?}"))),
    }
}
