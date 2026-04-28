use serde::{Deserialize, Serialize};

use crate::db::Db;
use crate::error::MemoryError;
use crate::events::{MAX_ACTOR_ID_LEN, MAX_METADATA_SIZE, MAX_SESSION_ID_LEN};

// --- Constants ---

pub const MAX_CHECKPOINT_NAME_LEN: usize = 256;
pub const MAX_BRANCH_NAME_LEN: usize = 256;
pub const DEFAULT_CHECKPOINT_LIMIT: u32 = 100;
pub const MAX_CHECKPOINT_LIMIT: u32 = 1000;

/// Maximum UUID length (8-4-4-4-12 = 36 chars).
const MAX_UUID_LEN: usize = 36;

// --- Data types ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    pub id: String,
    pub session_id: String,
    pub actor_id: String,
    pub name: String,
    pub event_id: String,
    pub metadata: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Branch {
    pub id: String,
    pub session_id: String,
    pub name: Option<String>,
    pub parent_branch_id: Option<String>,
    pub root_event_id: String,
    pub created_at: String,
}

// --- Param structs ---

#[derive(Debug, Clone)]
pub struct InsertCheckpointParams<'a> {
    pub actor_id: &'a str,
    pub session_id: &'a str,
    pub name: &'a str,
    pub event_id: &'a str,
    pub metadata: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct InsertBranchParams<'a> {
    /// Used for authorization: verifies root_event_id belongs to this actor.
    pub actor_id: &'a str,
    pub session_id: &'a str,
    pub root_event_id: &'a str,
    pub name: Option<&'a str>,
    pub parent_branch_id: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct ListCheckpointsParams<'a> {
    pub actor_id: &'a str,
    pub session_id: &'a str,
    pub limit: u32,
    pub offset: u32,
}

#[derive(Debug, Clone)]
pub struct ListBranchesParams<'a> {
    pub actor_id: &'a str,
    pub session_id: &'a str,
    pub limit: u32,
    pub offset: u32,
}

// --- Validation ---

fn validate_non_empty(s: &str, field: &str) -> Result<(), MemoryError> {
    if s.is_empty() {
        return Err(MemoryError::InvalidInput(format!(
            "{field} must not be empty"
        )));
    }
    Ok(())
}

fn validate_max_len(s: &str, max: usize, field: &str) -> Result<(), MemoryError> {
    if s.len() > max {
        return Err(MemoryError::InvalidInput(format!(
            "{field} exceeds maximum length of {max} bytes"
        )));
    }
    Ok(())
}

fn validate_no_control_chars(s: &str, field: &str) -> Result<(), MemoryError> {
    if s.chars().any(|c| c.is_control()) {
        return Err(MemoryError::InvalidInput(format!(
            "{field} must not contain control characters"
        )));
    }
    Ok(())
}

fn validate_metadata_json(s: &str) -> Result<(), MemoryError> {
    serde_json::from_str::<serde_json::Value>(s)
        .ok()
        .and_then(|v| v.as_object().map(|_| ()))
        .ok_or_else(|| MemoryError::InvalidInput("metadata must be a JSON object".into()))
}

fn validate_checkpoint_params(params: &InsertCheckpointParams<'_>) -> Result<(), MemoryError> {
    validate_non_empty(params.actor_id, "actor_id")?;
    validate_max_len(params.actor_id, MAX_ACTOR_ID_LEN, "actor_id")?;
    validate_non_empty(params.session_id, "session_id")?;
    validate_max_len(params.session_id, MAX_SESSION_ID_LEN, "session_id")?;
    validate_non_empty(params.name, "name")?;
    validate_max_len(params.name, MAX_CHECKPOINT_NAME_LEN, "name")?;
    validate_no_control_chars(params.name, "name")?;
    validate_non_empty(params.event_id, "event_id")?;
    validate_max_len(params.event_id, MAX_UUID_LEN, "event_id")?;
    if let Some(metadata) = params.metadata {
        validate_max_len(metadata, MAX_METADATA_SIZE, "metadata")?;
        validate_metadata_json(metadata)?;
    }
    Ok(())
}

fn validate_branch_params(params: &InsertBranchParams<'_>) -> Result<(), MemoryError> {
    validate_non_empty(params.actor_id, "actor_id")?;
    validate_max_len(params.actor_id, MAX_ACTOR_ID_LEN, "actor_id")?;
    validate_non_empty(params.session_id, "session_id")?;
    validate_max_len(params.session_id, MAX_SESSION_ID_LEN, "session_id")?;
    validate_non_empty(params.root_event_id, "root_event_id")?;
    validate_max_len(params.root_event_id, MAX_UUID_LEN, "root_event_id")?;
    if let Some(name) = params.name {
        validate_non_empty(name, "name")?;
        validate_max_len(name, MAX_BRANCH_NAME_LEN, "name")?;
        validate_no_control_chars(name, "name")?;
    }
    if let Some(parent_id) = params.parent_branch_id {
        validate_non_empty(parent_id, "parent_branch_id")?;
        validate_max_len(parent_id, MAX_UUID_LEN, "parent_branch_id")?;
    }
    Ok(())
}

// --- Business logic ---

/// Create a checkpoint pointing to a specific event within a session.
pub fn create_checkpoint(
    db: &dyn Db,
    params: &InsertCheckpointParams<'_>,
) -> Result<Checkpoint, MemoryError> {
    validate_checkpoint_params(params)?;
    db.create_checkpoint(params)
}

/// Fork a conversation by creating a branch from a specific event.
pub fn create_branch(
    db: &dyn Db,
    params: &InsertBranchParams<'_>,
) -> Result<Branch, MemoryError> {
    validate_branch_params(params)?;
    db.create_branch(params)
}

/// List checkpoints for a session, scoped to actor.
pub fn list_checkpoints(
    db: &dyn Db,
    params: &ListCheckpointsParams<'_>,
) -> Result<Vec<Checkpoint>, MemoryError> {
    validate_non_empty(params.actor_id, "actor_id")?;
    validate_non_empty(params.session_id, "session_id")?;
    let limit = params.limit.clamp(1, MAX_CHECKPOINT_LIMIT);
    db.list_checkpoints(&ListCheckpointsParams { limit, ..*params })
}

/// List branches for a session, scoped to actor via root_event_id JOIN.
pub fn list_branches(
    db: &dyn Db,
    params: &ListBranchesParams<'_>,
) -> Result<Vec<Branch>, MemoryError> {
    validate_non_empty(params.actor_id, "actor_id")?;
    validate_non_empty(params.session_id, "session_id")?;
    let limit = params.limit.clamp(1, MAX_CHECKPOINT_LIMIT);
    db.list_branches(&ListBranchesParams { limit, ..*params })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use tempfile::TempDir;

    fn make_checkpoint_params<'a>(
        actor_id: &'a str,
        session_id: &'a str,
        name: &'a str,
        event_id: &'a str,
    ) -> InsertCheckpointParams<'a> {
        InsertCheckpointParams {
            actor_id,
            session_id,
            name,
            event_id,
            metadata: None,
        }
    }

    fn open_db() -> (TempDir, rusqlite::Connection) {
        let dir = TempDir::new().unwrap();
        let conn = db::open(&dir.path().join("test.db")).unwrap();
        (dir, conn)
    }

    #[test]
    fn test_validate_empty_actor_id() {
        let params = make_checkpoint_params("", "session1", "checkpoint1", "event-id-1234567890");
        let err = validate_checkpoint_params(&params).unwrap_err();
        assert!(matches!(err, MemoryError::InvalidInput(_)));
    }

    #[test]
    fn test_validate_checkpoint_name_control_chars() {
        let params = make_checkpoint_params("actor1", "session1", "\x01name", "event-id-123456789");
        let err = validate_checkpoint_params(&params).unwrap_err();
        assert!(matches!(err, MemoryError::InvalidInput(_)));
        let msg = err.to_string();
        assert!(msg.contains("control characters"));
    }

    #[test]
    fn test_validate_checkpoint_name_empty() {
        let params = make_checkpoint_params("actor1", "session1", "", "event-id-123456789012");
        let err = validate_checkpoint_params(&params).unwrap_err();
        assert!(matches!(err, MemoryError::InvalidInput(_)));
    }

    #[test]
    fn test_validate_metadata_not_object() {
        let params = InsertCheckpointParams {
            metadata: Some("[]"),
            ..make_checkpoint_params("actor1", "session1", "cp1", "event-id-1234567890123")
        };
        let err = validate_checkpoint_params(&params).unwrap_err();
        assert!(matches!(err, MemoryError::InvalidInput(_)));
        let msg = err.to_string();
        assert!(msg.contains("metadata must be a JSON object"));
    }

    #[test]
    fn test_validate_metadata_invalid_json() {
        let params = InsertCheckpointParams {
            metadata: Some("not json"),
            ..make_checkpoint_params("actor1", "session1", "cp1", "event-id-1234567890123")
        };
        let err = validate_checkpoint_params(&params).unwrap_err();
        assert!(matches!(err, MemoryError::InvalidInput(_)));
        let msg = err.to_string();
        assert!(msg.contains("metadata must be a JSON object"));
    }

    #[test]
    fn test_validate_branch_name_control_chars() {
        let params = InsertBranchParams {
            actor_id: "actor1",
            session_id: "session1",
            root_event_id: "event-id-12345678901",
            name: Some("\x01branch"),
            parent_branch_id: None,
        };
        let err = validate_branch_params(&params).unwrap_err();
        assert!(matches!(err, MemoryError::InvalidInput(_)));
        let msg = err.to_string();
        assert!(msg.contains("control characters"));
    }

    #[test]
    fn test_validate_event_id_too_long() {
        // event_id > 36 chars
        let long_id = "a".repeat(37);
        let params = make_checkpoint_params("actor1", "session1", "cp1", &long_id);
        let err = validate_checkpoint_params(&params).unwrap_err();
        assert!(matches!(err, MemoryError::InvalidInput(_)));
    }

    #[test]
    fn test_list_checkpoints_empty_returns_vec() {
        let (_dir, conn) = open_db();
        let params = ListCheckpointsParams {
            actor_id: "actor1",
            session_id: "session1",
            limit: 100,
            offset: 0,
        };
        let result = list_checkpoints(&conn, &params).unwrap();
        assert!(result.is_empty());
    }
}
