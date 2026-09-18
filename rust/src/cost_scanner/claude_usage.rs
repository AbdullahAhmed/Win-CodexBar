use chrono::{DateTime, Utc};
use serde_json::Value;
use std::collections::HashSet;

use super::ClaudeUsageRecord;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) enum ClaudeUsageDedupKey {
    Request {
        message_id: Option<String>,
        request_id: String,
    },
    Session {
        session_id: String,
        message_id: String,
    },
}

pub(super) fn session_id_from_entries<'a, I>(entries: I) -> Option<&'a str>
where
    I: IntoIterator<Item = (&'a String, &'a Value)>,
{
    let mut metadata = None;
    for (key, value) in entries {
        if key == "sessionId" || key == "session_id" {
            if let Some(session_id) = value.as_str().map(str::trim).filter(|id| !id.is_empty()) {
                return Some(session_id);
            }
        } else if key == "metadata" {
            metadata = Some(value);
        }
    }
    metadata.and_then(session_id_from_value)
}

fn session_id_from_value(value: &Value) -> Option<&str> {
    let Value::Object(entries) = value else {
        return None;
    };
    session_id_from_entries(entries.iter())
}

pub(super) fn claude_usage_dedup_key(
    message_id: Option<&str>,
    request_id: Option<&str>,
    session_id: Option<&str>,
) -> Option<ClaudeUsageDedupKey> {
    let message_id = message_id
        .map(str::trim)
        .filter(|message_id| !message_id.is_empty())
        .map(str::to_string);
    let request_id = request_id
        .map(str::trim)
        .filter(|request_id| !request_id.is_empty())
        .map(str::to_string);
    if let Some(request_id) = request_id {
        return Some(ClaudeUsageDedupKey::Request {
            message_id,
            request_id: request_id.to_string(),
        });
    }

    let session_id = session_id
        .map(str::trim)
        .filter(|session_id| !session_id.is_empty())
        .map(str::to_string)?;
    Some(ClaudeUsageDedupKey::Session {
        session_id,
        message_id: message_id?,
    })
}

pub(super) fn should_count_claude_record(
    record: &ClaudeUsageRecord,
    cutoff: &DateTime<Utc>,
    seen: &mut HashSet<ClaudeUsageDedupKey>,
) -> bool {
    if let Some(timestamp) = record.timestamp
        && timestamp < *cutoff
    {
        return false;
    }

    if let Some(key) = &record.dedup_key
        && !seen.insert(key.clone())
    {
        return false;
    }

    true
}
