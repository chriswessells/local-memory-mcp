use serde::{Deserialize, Serialize};

use crate::db::Db;
use crate::error::MemoryError;

// --- Constants ---

pub const MAX_ACTOR_ID_LEN: usize = 256;
pub const MAX_SESSION_ID_LEN: usize = 256;
pub const MAX_CONTENT_SIZE: usize = 1_048_576; // 1 MB
pub const MAX_BLOB_SIZE: usize = 10_485_760; // 10 MB
pub const MAX_METADATA_SIZE: usize = 65_536; // 64 KB
pub const MAX_METADATA_KEYS: usize = 50;
pub const MAX_METADATA_DEPTH: usize = 5;
pub const MAX_PAGE_LIMIT: u32 = 1000;
pub const DEFAULT_PAGE_LIMIT: u32 = 100;

// --- Data types ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id: String,
    pub actor_id: String,
    pub session_id: String,
    pub event_type: String,
    pub role: Option<String>,
    pub content: Option<String>,
    #[serde(with = "serde_bytes", skip_serializing_if = "Option::is_none", default)]
    pub blob_data: Option<Vec<u8>>,
    pub metadata: Option<serde_json::Value>,
    pub branch_id: Option<String>,
    /// ISO 8601 UTC timestamp: YYYY-MM-DDTHH:MM:SSZ
    pub created_at: String,
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub session_id: String,
    pub actor_id: String,
    pub event_count: u64,
    pub first_event_at: String,
    pub last_event_at: String,
}

/// Three-state branch filter for get_events.
#[derive(Debug, Clone)]
pub enum BranchFilter<'a> {
    /// No branch filter — return events from all branches including main.
    All,
    /// Main timeline only — events where branch_id IS NULL.
    MainOnly,
    /// Specific branch by ID.
    Specific(&'a str),
}

#[derive(Debug, Clone)]
pub struct InsertEventParams<'a> {
    pub actor_id: &'a str,
    pub session_id: &'a str,
    pub event_type: &'a str,
    pub role: Option<&'a str>,
    pub content: Option<&'a str>,
    pub blob_data: Option<&'a [u8]>,
    pub metadata: Option<serde_json::Value>,
    pub branch_id: Option<&'a str>,
    pub expires_at: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct GetEventsParams<'a> {
    pub actor_id: &'a str,
    pub session_id: &'a str,
    pub branch_id: BranchFilter<'a>,
    pub limit: u32,
    pub offset: u32,
    pub before: Option<&'a str>,
    pub after: Option<&'a str>,
}

// --- Helpers ---

pub fn json_value_depth(v: &serde_json::Value) -> usize {
    match v {
        serde_json::Value::Object(m) => 1 + m.values().map(json_value_depth).max().unwrap_or(0),
        serde_json::Value::Array(a) => 1 + a.iter().map(json_value_depth).max().unwrap_or(0),
        _ => 0,
    }
}

// --- Validation ---

fn validate_non_empty(value: &str, field: &str) -> Result<(), MemoryError> {
    if value.is_empty() {
        return Err(MemoryError::InvalidInput(format!(
            "{field} must not be empty"
        )));
    }
    Ok(())
}

fn validate_max_len(value: &str, max: usize, field: &str) -> Result<(), MemoryError> {
    if value.len() > max {
        return Err(MemoryError::InvalidInput(format!(
            "{field} exceeds maximum length of {max} bytes"
        )));
    }
    Ok(())
}

fn validate_json_object_value(v: &serde_json::Value, field: &str) -> Result<(), MemoryError> {
    if !v.is_object() {
        return Err(MemoryError::InvalidInput(format!(
            "{field} must be a JSON object"
        )));
    }
    Ok(())
}

/// Validates YYYY-MM-DDTHH:MM:SSZ format (basic structure check).
fn validate_timestamp_format(value: &str, field: &str) -> Result<(), MemoryError> {
    let b = value.as_bytes();
    if b.len() != 20
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
        || b[19] != b'Z'
    {
        return Err(MemoryError::InvalidInput(format!(
            "{field} must be in YYYY-MM-DDTHH:MM:SSZ format"
        )));
    }
    Ok(())
}

fn validate_insert_params(params: &InsertEventParams<'_>) -> Result<(), MemoryError> {
    validate_non_empty(params.actor_id, "actor_id")?;
    validate_max_len(params.actor_id, MAX_ACTOR_ID_LEN, "actor_id")?;
    validate_non_empty(params.session_id, "session_id")?;
    validate_max_len(params.session_id, MAX_SESSION_ID_LEN, "session_id")?;

    match params.event_type {
        "conversation" => {
            if params.content.is_none() {
                return Err(MemoryError::InvalidInput(
                    "content is required for conversation events".into(),
                ));
            }
            if params.blob_data.is_some() {
                return Err(MemoryError::InvalidInput(
                    "blob_data is not allowed for conversation events".into(),
                ));
            }
        }
        "blob" => {
            if params.blob_data.is_none() {
                return Err(MemoryError::InvalidInput(
                    "blob_data is required for blob events".into(),
                ));
            }
            if params.content.is_some() {
                return Err(MemoryError::InvalidInput(
                    "content is not allowed for blob events".into(),
                ));
            }
        }
        _ => {
            return Err(MemoryError::InvalidInput(
                "event_type must be 'conversation' or 'blob'".into(),
            ));
        }
    }

    if let Some(role) = params.role {
        if !matches!(role, "user" | "assistant" | "tool" | "system") {
            return Err(MemoryError::InvalidInput(
                "role must be 'user', 'assistant', 'tool', or 'system'".into(),
            ));
        }
    }

    if let Some(content) = params.content {
        if content.len() > MAX_CONTENT_SIZE {
            return Err(MemoryError::InvalidInput(format!(
                "content exceeds maximum size of {MAX_CONTENT_SIZE} bytes"
            )));
        }
    }

    if let Some(blob) = params.blob_data {
        if blob.len() > MAX_BLOB_SIZE {
            return Err(MemoryError::InvalidInput(format!(
                "blob_data exceeds maximum size of {MAX_BLOB_SIZE} bytes"
            )));
        }
    }

    if let Some(ref v) = params.metadata {
        validate_json_object_value(v, "metadata")?;
        let obj = v.as_object().expect("validated as object above");
        if obj.len() > MAX_METADATA_KEYS {
            return Err(MemoryError::InvalidInput(format!(
                "metadata exceeds maximum of {MAX_METADATA_KEYS} keys"
            )));
        }
        if json_value_depth(v) > MAX_METADATA_DEPTH {
            return Err(MemoryError::InvalidInput(format!(
                "metadata exceeds maximum nesting depth of {MAX_METADATA_DEPTH}"
            )));
        }
        let serialized =
            serde_json::to_string(v).expect("serde_json::Value is always serializable");
        if serialized.len() > MAX_METADATA_SIZE {
            return Err(MemoryError::InvalidInput(format!(
                "metadata exceeds maximum length of {MAX_METADATA_SIZE} bytes"
            )));
        }
    }

    if let Some(expires_at) = params.expires_at {
        validate_timestamp_format(expires_at, "expires_at")?;
    }

    Ok(())
}

fn validate_get_events_params(params: &GetEventsParams<'_>) -> Result<(), MemoryError> {
    validate_non_empty(params.actor_id, "actor_id")?;
    validate_non_empty(params.session_id, "session_id")?;
    if let Some(before) = params.before {
        validate_timestamp_format(before, "before")?;
    }
    if let Some(after) = params.after {
        validate_timestamp_format(after, "after")?;
    }
    Ok(())
}

// --- Business logic ---

/// Insert a validated event. Validates inputs then delegates to Db.
pub fn add_event(db: &dyn Db, params: &InsertEventParams<'_>) -> Result<Event, MemoryError> {
    validate_insert_params(params)?;
    db.insert_event(params)
}

/// Get a single event by ID, scoped to actor.
pub fn get_event(db: &dyn Db, actor_id: &str, event_id: &str) -> Result<Event, MemoryError> {
    validate_non_empty(actor_id, "actor_id")?;
    db.get_event(actor_id, event_id)
}

/// Get events for an actor+session with optional filters.
pub fn get_events(db: &dyn Db, params: &GetEventsParams<'_>) -> Result<Vec<Event>, MemoryError> {
    validate_get_events_params(params)?;
    let clamped = GetEventsParams {
        limit: params.limit.clamp(1, MAX_PAGE_LIMIT),
        ..params.clone()
    };
    db.get_events(&clamped)
}

/// List distinct sessions for an actor.
pub fn list_sessions(
    db: &dyn Db,
    actor_id: &str,
    limit: u32,
    offset: u32,
) -> Result<Vec<SessionInfo>, MemoryError> {
    validate_non_empty(actor_id, "actor_id")?;
    db.list_sessions(actor_id, limit.clamp(1, MAX_PAGE_LIMIT), offset)
}

/// Delete events past their expires_at.
pub fn delete_expired(db: &dyn Db) -> Result<u64, MemoryError> {
    db.delete_expired_events()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use tempfile::TempDir;

    fn open_db() -> (TempDir, rusqlite::Connection) {
        let dir = TempDir::new().unwrap();
        let conn = db::open(&dir.path().join("test.db")).unwrap();
        (dir, conn)
    }

    fn conversation_params<'a>(
        actor: &'a str,
        session: &'a str,
        content: &'a str,
    ) -> InsertEventParams<'a> {
        InsertEventParams {
            actor_id: actor,
            session_id: session,
            event_type: "conversation",
            role: Some("user"),
            content: Some(content),
            blob_data: None,
            metadata: None,
            branch_id: None,
            expires_at: None,
        }
    }

    #[test]
    fn test_validate_event_type() {
        let params = InsertEventParams {
            event_type: "invalid",
            ..conversation_params("a", "s", "hi")
        };
        let err = validate_insert_params(&params).unwrap_err();
        assert!(matches!(err, MemoryError::InvalidInput(_)));
    }

    #[test]
    fn test_validate_empty_actor() {
        let params = conversation_params("", "s", "hi");
        let err = validate_insert_params(&params).unwrap_err();
        assert!(matches!(err, MemoryError::InvalidInput(_)));
    }

    #[test]
    fn test_validate_content_blob_mismatch() {
        // conversation without content
        let params = InsertEventParams {
            content: None,
            ..conversation_params("a", "s", "hi")
        };
        assert!(validate_insert_params(&params).is_err());

        // blob without blob_data
        let params = InsertEventParams {
            event_type: "blob",
            content: None,
            blob_data: None,
            ..conversation_params("a", "s", "hi")
        };
        assert!(validate_insert_params(&params).is_err());
    }

    #[test]
    fn test_validate_content_size() {
        let big = "x".repeat(MAX_CONTENT_SIZE + 1);
        let params = conversation_params("a", "s", &big);
        let err = validate_insert_params(&params).unwrap_err();
        assert!(matches!(err, MemoryError::InvalidInput(_)));
    }

    #[test]
    fn test_add_event_validates() {
        let (_dir, conn) = open_db();
        let params = InsertEventParams {
            event_type: "bad",
            ..conversation_params("a", "s", "hi")
        };
        let err = add_event(&conn, &params).unwrap_err();
        assert!(matches!(err, MemoryError::InvalidInput(_)));
    }
}
