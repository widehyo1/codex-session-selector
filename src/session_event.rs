use anyhow::{Result, bail};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MessageRole {
    User,
    Agent,
}

impl MessageRole {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Agent => "agent",
        }
    }
    pub(crate) fn from_str(value: &str) -> Result<Self> {
        match value {
            "user" => Ok(Self::User),
            "agent" => Ok(Self::Agent),
            _ => bail!("invalid canonical message role {value}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NormalizedMessage {
    pub role: MessageRole,
    pub phase: Option<String>,
    pub content: String,
}

pub(crate) fn normalize_message_record(value: &Value) -> Option<NormalizedMessage> {
    let (kind, payload) = match value.get("type").and_then(Value::as_str) {
        Some("event_msg") | Some("response_item") => (
            value.get("type").and_then(Value::as_str)?,
            value.get("payload")?,
        ),
        _ => ("bare", value),
    };
    match kind {
        "event_msg" | "bare" => match payload.get("type").and_then(Value::as_str) {
            Some("user_message") => payload
                .get("message")
                .and_then(Value::as_str)
                .zip(valid_phase(payload))
                .map(|(content, phase)| NormalizedMessage {
                    role: MessageRole::User,
                    phase,
                    content: content.to_owned(),
                }),
            Some("agent_message") => payload
                .get("message")
                .and_then(Value::as_str)
                .zip(valid_phase(payload))
                .map(|(content, phase)| NormalizedMessage {
                    role: MessageRole::Agent,
                    phase,
                    content: content.to_owned(),
                }),
            _ => None,
        },
        "response_item" if payload.get("type").and_then(Value::as_str) == Some("message") => {
            let role = match payload.get("role").and_then(Value::as_str) {
                Some("user") => MessageRole::User,
                Some("assistant") => MessageRole::Agent,
                _ => return None,
            };
            Some(NormalizedMessage {
                role,
                phase: valid_phase(payload)?,
                content: value_to_text(payload.get("content").unwrap_or(&Value::Null)),
            })
        }
        _ => None,
    }
}

pub(crate) fn value_to_text(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Array(values) => values
            .iter()
            .map(value_to_text)
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Object(values) => values
            .get("text")
            .and_then(Value::as_str)
            .or_else(|| values.get("output").and_then(Value::as_str))
            .map(str::to_owned)
            .unwrap_or_else(|| value.to_string()),
        Value::Number(_) | Value::Bool(_) => value.to_string(),
        Value::Null => String::new(),
    }
}

fn valid_phase(value: &Value) -> Option<Option<String>> {
    match value.get("phase") {
        None | Some(Value::Null) => Some(None),
        Some(Value::String(phase)) => Some(Some(phase.clone())),
        Some(_) => None,
    }
}
