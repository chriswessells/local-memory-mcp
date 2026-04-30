use serde::{Deserialize, Serialize};

use crate::db::Db;
use crate::error::MemoryError;
use crate::events::{json_value_depth, MAX_METADATA_DEPTH, MAX_METADATA_KEYS, MAX_METADATA_SIZE};
use crate::memories::Memory;

// --- Constants ---

pub const MAX_LABEL_LEN: usize = 256;
pub const MAX_PROPERTIES_SIZE: usize = MAX_METADATA_SIZE;
pub const MAX_PROPERTIES_KEYS: usize = MAX_METADATA_KEYS;
pub const MAX_PROPERTIES_DEPTH: usize = MAX_METADATA_DEPTH;
pub const MAX_EDGE_ID_LEN: usize = 256;
pub const MAX_MEMORY_ID_LEN: usize = 256;
pub const MAX_TRAVERSE_DEPTH: u32 = 5;
pub const DEFAULT_TRAVERSE_DEPTH: u32 = 2;
pub const MAX_NEIGHBOR_LIMIT: u32 = 1000;
pub const DEFAULT_NEIGHBOR_LIMIT: u32 = 100;

// --- Data types ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub id: String,
    pub from_memory_id: String,
    pub to_memory_id: String,
    pub label: String,
    pub properties: Option<serde_json::Value>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Neighbor {
    pub edge: Edge,
    pub memory: Memory,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraversalNode {
    pub memory: Memory,
    pub depth: u32,
    pub path: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    #[default]
    Out,
    In,
    Both,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LabelCount {
    pub label: String,
    pub count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphStats {
    pub total_edges: u64,
    pub labels: Vec<LabelCount>,
    pub most_connected: Vec<ConnectedMemory>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectedMemory {
    pub memory_id: String,
    pub edge_count: u64,
}

#[derive(Debug, Clone)]
pub struct InsertEdgeParams<'a> {
    pub actor_id: &'a str,
    pub from_memory_id: &'a str,
    pub to_memory_id: &'a str,
    pub label: &'a str,
    pub properties: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct UpdateEdgeParams<'a> {
    pub actor_id: &'a str,
    pub edge_id: &'a str,
    pub label: Option<&'a str>,
    pub properties: Option<serde_json::Value>,
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

fn validate_insert_edge_params(params: &InsertEdgeParams<'_>) -> Result<(), MemoryError> {
    validate_non_empty(params.actor_id, "actor_id")?;
    validate_non_empty(params.from_memory_id, "from_memory_record_id")?;
    validate_max_len(
        params.from_memory_id,
        MAX_MEMORY_ID_LEN,
        "from_memory_record_id",
    )?;
    validate_non_empty(params.to_memory_id, "to_memory_record_id")?;
    validate_max_len(
        params.to_memory_id,
        MAX_MEMORY_ID_LEN,
        "to_memory_record_id",
    )?;
    validate_non_empty(params.label, "label")?;
    validate_max_len(params.label, MAX_LABEL_LEN, "label")?;
    if params.from_memory_id == params.to_memory_id {
        return Err(MemoryError::InvalidInput(
            "self-edges are not allowed".into(),
        ));
    }
    if let Some(ref v) = params.properties {
        validate_json_object_value(v, "properties")?;
        let obj = v.as_object().expect("validated as object above");
        if obj.len() > MAX_PROPERTIES_KEYS {
            return Err(MemoryError::InvalidInput(format!(
                "properties exceeds maximum of {MAX_PROPERTIES_KEYS} keys"
            )));
        }
        if json_value_depth(v) > MAX_PROPERTIES_DEPTH {
            return Err(MemoryError::InvalidInput(format!(
                "properties exceeds maximum nesting depth of {MAX_PROPERTIES_DEPTH}"
            )));
        }
        let serialized =
            serde_json::to_string(v).expect("serde_json::Value is always serializable");
        if serialized.len() > MAX_PROPERTIES_SIZE {
            return Err(MemoryError::InvalidInput(format!(
                "properties exceeds maximum length of {MAX_PROPERTIES_SIZE} bytes"
            )));
        }
    }
    Ok(())
}

fn validate_update_edge_params(params: &UpdateEdgeParams<'_>) -> Result<(), MemoryError> {
    validate_non_empty(params.actor_id, "actor_id")?;
    validate_non_empty(params.edge_id, "edge_id")?;
    validate_max_len(params.edge_id, MAX_EDGE_ID_LEN, "edge_id")?;
    if params.label.is_none() && params.properties.is_none() {
        return Err(MemoryError::InvalidInput(
            "at least one of label or properties must be provided".into(),
        ));
    }
    if let Some(label) = params.label {
        validate_non_empty(label, "label")?;
        validate_max_len(label, MAX_LABEL_LEN, "label")?;
    }
    if let Some(ref v) = params.properties {
        validate_json_object_value(v, "properties")?;
        let obj = v.as_object().expect("validated as object above");
        if obj.len() > MAX_PROPERTIES_KEYS {
            return Err(MemoryError::InvalidInput(format!(
                "properties exceeds maximum of {MAX_PROPERTIES_KEYS} keys"
            )));
        }
        if json_value_depth(v) > MAX_PROPERTIES_DEPTH {
            return Err(MemoryError::InvalidInput(format!(
                "properties exceeds maximum nesting depth of {MAX_PROPERTIES_DEPTH}"
            )));
        }
        let serialized =
            serde_json::to_string(v).expect("serde_json::Value is always serializable");
        if serialized.len() > MAX_PROPERTIES_SIZE {
            return Err(MemoryError::InvalidInput(format!(
                "properties exceeds maximum length of {MAX_PROPERTIES_SIZE} bytes"
            )));
        }
    }
    Ok(())
}

// --- Business logic ---

pub fn add_edge(db: &dyn Db, params: &InsertEdgeParams<'_>) -> Result<Edge, MemoryError> {
    validate_insert_edge_params(params)?;
    db.insert_edge(params)
}

pub fn get_neighbors(
    db: &dyn Db,
    actor_id: &str,
    memory_id: &str,
    direction: Direction,
    label: Option<&str>,
    limit: u32,
) -> Result<Vec<Neighbor>, MemoryError> {
    validate_non_empty(actor_id, "actor_id")?;
    validate_non_empty(memory_id, "memory_record_id")?;
    let clamped = limit.clamp(1, MAX_NEIGHBOR_LIMIT);
    db.get_neighbors(actor_id, memory_id, direction, label, clamped)
}

pub fn traverse(
    db: &dyn Db,
    actor_id: &str,
    start_memory_id: &str,
    max_depth: u32,
    label: Option<&str>,
    direction: Direction,
) -> Result<Vec<TraversalNode>, MemoryError> {
    validate_non_empty(actor_id, "actor_id")?;
    validate_non_empty(start_memory_id, "start_memory_record_id")?;
    let clamped = max_depth.clamp(1, MAX_TRAVERSE_DEPTH);
    db.traverse(actor_id, start_memory_id, clamped, label, direction)
}

pub fn update_edge(db: &dyn Db, params: &UpdateEdgeParams<'_>) -> Result<Edge, MemoryError> {
    validate_update_edge_params(params)?;
    db.update_edge(params)
}

pub fn delete_edge(db: &dyn Db, actor_id: &str, edge_id: &str) -> Result<(), MemoryError> {
    validate_non_empty(actor_id, "actor_id")?;
    validate_non_empty(edge_id, "edge_id")?;
    db.delete_edge(actor_id, edge_id)
}

pub fn list_labels(db: &dyn Db, actor_id: &str) -> Result<Vec<LabelCount>, MemoryError> {
    validate_non_empty(actor_id, "actor_id")?;
    db.list_edge_labels(actor_id)
}

pub fn graph_stats(db: &dyn Db, actor_id: &str) -> Result<GraphStats, MemoryError> {
    validate_non_empty(actor_id, "actor_id")?;
    db.graph_stats(actor_id)
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

    #[test]
    fn test_add_edge_validates_empty_label() {
        let (_dir, conn) = open_db();
        let params = InsertEdgeParams {
            actor_id: "a1",
            from_memory_id: "m1",
            to_memory_id: "m2",
            label: "",
            properties: None,
        };
        let err = add_edge(&conn, &params).unwrap_err();
        assert!(matches!(err, MemoryError::InvalidInput(_)));
    }

    #[test]
    fn test_add_edge_validates_self_edge() {
        let (_dir, conn) = open_db();
        let params = InsertEdgeParams {
            actor_id: "a1",
            from_memory_id: "m1",
            to_memory_id: "m1",
            label: "uses",
            properties: None,
        };
        let err = add_edge(&conn, &params).unwrap_err();
        assert!(matches!(err, MemoryError::InvalidInput(_)));
        assert!(err.to_string().contains("self-edges"));
    }

    #[test]
    fn test_traverse_clamps_depth() {
        let (_dir, conn) = open_db();
        // Create memories and edge so traverse has something to work with
        let m1 = conn
            .insert_memory(&crate::memories::InsertMemoryParams {
                actor_id: "a1",
                content: "A",
                strategy: "semantic",
                namespace: None,
                metadata: None,
                source_session_id: None,
                embedding: None,
            })
            .unwrap();
        let m2 = conn
            .insert_memory(&crate::memories::InsertMemoryParams {
                actor_id: "a1",
                content: "B",
                strategy: "semantic",
                namespace: None,
                metadata: None,
                source_session_id: None,
                embedding: None,
            })
            .unwrap();
        conn.insert_edge(&InsertEdgeParams {
            actor_id: "a1",
            from_memory_id: &m1.id,
            to_memory_id: &m2.id,
            label: "uses",
            properties: None,
        })
        .unwrap();

        // Depth 100 should be clamped to MAX_TRAVERSE_DEPTH (5)
        let nodes = traverse(&conn, "a1", &m1.id, 100, None, Direction::Out).unwrap();
        assert_eq!(nodes.len(), 1); // only B reachable
    }
}
