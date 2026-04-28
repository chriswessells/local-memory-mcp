use serde::{Deserialize, Serialize};

use crate::db::{Db, EMBEDDING_DIM};
use crate::error::MemoryError;
use crate::events::{MAX_ACTOR_ID_LEN, MAX_METADATA_SIZE, MAX_PAGE_LIMIT};

// --- Constants ---

pub const MAX_MEMORY_CONTENT_SIZE: usize = 1_048_576; // 1 MB
pub const MAX_NAMESPACE_LEN: usize = 512;
pub const MAX_STRATEGY_LEN: usize = 128;

// --- Data types ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub id: String,
    pub actor_id: String,
    pub namespace: String,
    pub strategy: String,
    pub content: String,
    pub metadata: Option<String>,
    pub source_session_id: Option<String>,
    pub is_valid: bool,
    pub superseded_by: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct InsertMemoryParams<'a> {
    pub actor_id: &'a str,
    pub content: &'a str,
    pub strategy: &'a str,
    pub namespace: Option<&'a str>,
    pub metadata: Option<&'a str>,
    pub source_session_id: Option<&'a str>,
    pub embedding: Option<&'a [f32]>,
}

#[derive(Debug, Clone)]
pub enum ConsolidateAction<'a> {
    /// Replace content (and optionally embedding). Old memory marked invalid,
    /// superseded_by points to new memory.
    Update {
        content: &'a str,
        embedding: Option<&'a [f32]>,
    },
    /// Mark memory as invalid. No replacement created.
    Invalidate,
}

#[derive(Debug, Clone)]
pub struct ListMemoriesParams<'a> {
    pub actor_id: &'a str,
    pub namespace: Option<&'a str>,
    pub namespace_prefix: Option<&'a str>,
    pub strategy: Option<&'a str>,
    pub valid_only: bool,
    pub limit: u32,
    pub offset: u32,
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

fn validate_insert_memory_params(params: &InsertMemoryParams<'_>) -> Result<(), MemoryError> {
    validate_non_empty(params.actor_id, "actor_id")?;
    validate_max_len(params.actor_id, MAX_ACTOR_ID_LEN, "actor_id")?;
    validate_non_empty(params.content, "content")?;
    validate_max_len(params.content, MAX_MEMORY_CONTENT_SIZE, "content")?;
    validate_non_empty(params.strategy, "strategy")?;
    validate_max_len(params.strategy, MAX_STRATEGY_LEN, "strategy")?;

    if let Some(ns) = params.namespace {
        crate::namespaces::validate_namespace_name(ns)?;
    }
    if let Some(metadata) = params.metadata {
        validate_max_len(metadata, MAX_METADATA_SIZE, "metadata")?;
        match serde_json::from_str::<serde_json::Value>(metadata) {
            Ok(v) if v.is_object() => {}
            _ => {
                return Err(MemoryError::InvalidInput(
                    "metadata must be a valid JSON object".into(),
                ));
            }
        }
    }
    if let Some(sid) = params.source_session_id {
        validate_non_empty(sid, "source_session_id")?;
    }
    if let Some(emb) = params.embedding {
        if emb.len() != EMBEDDING_DIM as usize {
            return Err(MemoryError::InvalidInput(format!(
                "embedding must have exactly {EMBEDDING_DIM} dimensions"
            )));
        }
        if emb.iter().any(|v| !v.is_finite()) {
            return Err(MemoryError::InvalidInput(
                "embedding contains NaN or infinity".into(),
            ));
        }
    }
    Ok(())
}

fn validate_list_memories_params(params: &ListMemoriesParams<'_>) -> Result<(), MemoryError> {
    validate_non_empty(params.actor_id, "actor_id")?;
    if params.namespace.is_some() && params.namespace_prefix.is_some() {
        return Err(MemoryError::InvalidInput(
            "namespace and namespace_prefix are mutually exclusive".into(),
        ));
    }
    if let Some(ns) = params.namespace {
        validate_non_empty(ns, "namespace")?;
    }
    if let Some(prefix) = params.namespace_prefix {
        validate_non_empty(prefix, "namespace_prefix")?;
    }
    Ok(())
}

fn validate_consolidate_params(action: &ConsolidateAction<'_>) -> Result<(), MemoryError> {
    if let ConsolidateAction::Update { content, embedding } = action {
        validate_non_empty(content, "content")?;
        validate_max_len(content, MAX_MEMORY_CONTENT_SIZE, "content")?;
        if let Some(emb) = embedding {
            if emb.len() != EMBEDDING_DIM as usize {
                return Err(MemoryError::InvalidInput(format!(
                    "embedding must have exactly {EMBEDDING_DIM} dimensions"
                )));
            }
            if emb.iter().any(|v| !v.is_finite()) {
                return Err(MemoryError::InvalidInput(
                    "embedding contains NaN or infinity".into(),
                ));
            }
        }
    }
    Ok(())
}

// --- Business logic ---

/// Store an extracted insight as a long-term memory.
pub fn store_memory(db: &dyn Db, params: &InsertMemoryParams<'_>) -> Result<Memory, MemoryError> {
    validate_insert_memory_params(params)?;
    db.insert_memory(params)
}

/// Get a single memory by ID, scoped to actor.
pub fn get_memory(db: &dyn Db, actor_id: &str, memory_id: &str) -> Result<Memory, MemoryError> {
    validate_non_empty(actor_id, "actor_id")?;
    validate_non_empty(memory_id, "memory_id")?;
    db.get_memory(actor_id, memory_id)
}

/// List memories with filters.
pub fn list_memories(
    db: &dyn Db,
    params: &ListMemoriesParams<'_>,
) -> Result<Vec<Memory>, MemoryError> {
    validate_list_memories_params(params)?;
    let clamped = ListMemoriesParams {
        limit: params.limit.clamp(1, MAX_PAGE_LIMIT),
        ..params.clone()
    };
    db.list_memories(&clamped)
}

/// Consolidate (update or invalidate) a memory, scoped to actor.
pub fn consolidate_memory(
    db: &dyn Db,
    actor_id: &str,
    memory_id: &str,
    action: &ConsolidateAction<'_>,
) -> Result<Memory, MemoryError> {
    validate_non_empty(actor_id, "actor_id")?;
    validate_non_empty(memory_id, "memory_id")?;
    validate_consolidate_params(action)?;
    db.consolidate_memory(actor_id, memory_id, action)
}

/// Hard-delete a memory, scoped to actor.
pub fn delete_memory(db: &dyn Db, actor_id: &str, memory_id: &str) -> Result<(), MemoryError> {
    validate_non_empty(actor_id, "actor_id")?;
    validate_non_empty(memory_id, "memory_id")?;
    db.delete_memory(actor_id, memory_id)
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

    fn mem_params<'a>(
        actor: &'a str,
        content: &'a str,
        strategy: &'a str,
    ) -> InsertMemoryParams<'a> {
        InsertMemoryParams {
            actor_id: actor,
            content,
            strategy,
            namespace: None,
            metadata: None,
            source_session_id: None,
            embedding: None,
        }
    }

    #[test]
    fn test_validate_empty_actor() {
        let params = mem_params("", "content", "semantic");
        assert!(matches!(
            validate_insert_memory_params(&params),
            Err(MemoryError::InvalidInput(_))
        ));
    }

    #[test]
    fn test_validate_empty_content() {
        let params = mem_params("actor1", "", "semantic");
        assert!(matches!(
            validate_insert_memory_params(&params),
            Err(MemoryError::InvalidInput(_))
        ));
    }

    #[test]
    fn test_validate_embedding_wrong_dim() {
        let params = InsertMemoryParams {
            embedding: Some(&[1.0, 2.0, 3.0]),
            ..mem_params("actor1", "content", "semantic")
        };
        assert!(matches!(
            validate_insert_memory_params(&params),
            Err(MemoryError::InvalidInput(_))
        ));
    }

    #[test]
    fn test_validate_namespace_mutual_exclusion() {
        let params = ListMemoriesParams {
            actor_id: "actor1",
            namespace: Some("ns"),
            namespace_prefix: Some("prefix"),
            strategy: None,
            valid_only: true,
            limit: 100,
            offset: 0,
        };
        assert!(matches!(
            validate_list_memories_params(&params),
            Err(MemoryError::InvalidInput(_))
        ));
    }

    #[test]
    fn test_validate_consolidate_update_content() {
        let action = ConsolidateAction::Update {
            content: "",
            embedding: None,
        };
        assert!(matches!(
            validate_consolidate_params(&action),
            Err(MemoryError::InvalidInput(_))
        ));
    }

    #[test]
    fn test_store_memory_validates() {
        let (_dir, conn) = open_db();
        let params = mem_params("", "content", "semantic");
        assert!(matches!(
            store_memory(&conn, &params),
            Err(MemoryError::InvalidInput(_))
        ));
    }
}
