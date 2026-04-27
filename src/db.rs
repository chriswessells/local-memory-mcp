use rusqlite::Connection;
use std::path::Path;
use std::sync::OnceLock;
use crate::error::MemoryError;
use crate::events::{BranchFilter, Event, GetEventsParams, InsertEventParams, SessionInfo};
use crate::graph::{ConnectedMemory, Direction, Edge, GraphStats, InsertEdgeParams, LabelCount, Neighbor, TraversalNode, UpdateEdgeParams};
use crate::memories::{ConsolidateAction, InsertMemoryParams, ListMemoriesParams, Memory};
use crate::search::{SearchFtsParams, SearchVectorParams};

pub const SCHEMA_VERSION: u32 = 1;
pub const EMBEDDING_DIM: u32 = 384;

static VEC_INIT: OnceLock<Result<(), String>> = OnceLock::new();

fn ensure_sqlite_vec() -> Result<(), MemoryError> {
    let result = VEC_INIT.get_or_init(|| {
        // SAFETY: sqlite3_vec_init has the same function pointer signature as
        // RawAutoExtension (sqlite3_auto_extension callback). This transmute is
        // valid only for sqlite-vec =0.1.7-alpha.10 with rusqlite =0.35.0.
        // Any version bump of either crate requires re-verifying this cast.
        unsafe {
            use rusqlite::auto_extension::{RawAutoExtension, register_auto_extension};
            use sqlite_vec::sqlite3_vec_init;
            let raw: RawAutoExtension = std::mem::transmute(sqlite3_vec_init as *const ());
            register_auto_extension(raw).map_err(|e| e.to_string())
        }
    });
    result.as_ref().map(|_| ()).map_err(|e| MemoryError::SchemaError(
        format!("failed to register sqlite-vec: {e}")
    ))
}

pub fn open(path: &Path) -> Result<Connection, MemoryError> {
    ensure_sqlite_vec()?;

    let mut conn = Connection::open(path).map_err(|e| {
        tracing::error!("SQLite open failed for {}: {e}", path.display());
        MemoryError::ConnectionFailed("failed to open database".into())
    })?;

    // Pragmas
    let sync_mode = if std::env::var("LOCAL_MEMORY_SYNC").as_deref() == Ok("normal") {
        "NORMAL"
    } else {
        "FULL"
    };
    conn.execute_batch(&format!(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = {sync_mode};
         PRAGMA foreign_keys = ON;
         PRAGMA busy_timeout = 5000;
         PRAGMA cache_size = -2000;
         PRAGMA wal_autocheckpoint = 1000;
         PRAGMA journal_size_limit = 67108864;
         PRAGMA locking_mode = EXCLUSIVE;"
    )).map_err(|e| {
        tracing::error!("Pragma setup failed: {e}");
        MemoryError::ConnectionFailed("failed to configure database".into())
    })?;

    // Force EXCLUSIVE lock acquisition
    conn.execute_batch("BEGIN IMMEDIATE; COMMIT;").map_err(|e| {
        if let rusqlite::Error::SqliteFailure(ref err, _) = e {
            if err.code == rusqlite::ErrorCode::DatabaseBusy {
                return MemoryError::StoreLocked(path.display().to_string());
            }
        }
        tracing::error!("Lock acquisition failed: {e}");
        MemoryError::ConnectionFailed("failed to acquire database lock".into())
    })?;

    // sqlite-vec smoke test
    conn.query_row("SELECT vec_version()", [], |_| Ok(())).map_err(|e| {
        tracing::error!("sqlite-vec smoke test failed: {e}");
        MemoryError::SchemaError("sqlite-vec extension not functional".into())
    })?;

    // Integrity check for pre-existing databases only
    let user_version: u32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))
        .map_err(|e| {
            tracing::error!("user_version query failed: {e}");
            MemoryError::QueryFailed("failed to read schema version".into())
        })?;

    if user_version > 0 {
        let check: String = conn.query_row("PRAGMA quick_check", [], |r| r.get(0))
            .map_err(|e| {
                tracing::error!("quick_check failed: {e}");
                MemoryError::QueryFailed("integrity check failed".into())
            })?;
        if check != "ok" {
            return Err(MemoryError::DatabaseCorrupted(path.display().to_string()));
        }
    }

    migrate(&mut conn)?;

    Ok(conn)
}

pub fn migrate(conn: &mut Connection) -> Result<(), MemoryError> {
    let version: u32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))
        .map_err(|e| {
            tracing::error!("user_version read failed: {e}");
            MemoryError::SchemaError("failed to read schema version".into())
        })?;

    if version > SCHEMA_VERSION {
        return Err(MemoryError::SchemaVersionTooNew(version, SCHEMA_VERSION));
    }

    if version < 1 {
        migrate_v1(conn)?;
    }

    Ok(())
}

fn migrate_v1(conn: &mut Connection) -> Result<(), MemoryError> {
    let tx = conn.transaction().map_err(|e| {
        tracing::error!("Failed to begin migration transaction: {e}");
        MemoryError::SchemaError("failed to begin migration".into())
    })?;

    tx.execute_batch("
        CREATE TABLE IF NOT EXISTS events (
            id TEXT PRIMARY KEY,
            actor_id TEXT NOT NULL,
            session_id TEXT NOT NULL,
            event_type TEXT NOT NULL,
            role TEXT,
            content TEXT,
            blob_data BLOB,
            metadata TEXT,
            branch_id TEXT,
            created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
            expires_at TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_events_session ON events(actor_id, session_id, created_at);
        CREATE INDEX IF NOT EXISTS idx_events_branch ON events(session_id, branch_id, created_at);
        CREATE INDEX IF NOT EXISTS idx_events_actor ON events(actor_id, created_at);
        CREATE INDEX IF NOT EXISTS idx_events_expires ON events(expires_at) WHERE expires_at IS NOT NULL;

        CREATE TABLE IF NOT EXISTS memories (
            memory_rowid INTEGER PRIMARY KEY,
            id TEXT UNIQUE NOT NULL,
            actor_id TEXT NOT NULL,
            namespace TEXT DEFAULT 'default',
            strategy TEXT NOT NULL,
            content TEXT NOT NULL,
            metadata TEXT,
            source_session_id TEXT,
            is_valid INTEGER DEFAULT 1,
            superseded_by TEXT,
            created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
            updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
        );
        CREATE INDEX IF NOT EXISTS idx_memories_actor ON memories(actor_id, namespace, is_valid);
        CREATE INDEX IF NOT EXISTS idx_memories_strategy ON memories(strategy, is_valid);

        CREATE TABLE IF NOT EXISTS knowledge_edges (
            id TEXT PRIMARY KEY,
            from_memory_id TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
            to_memory_id TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
            label TEXT NOT NULL,
            properties TEXT,
            created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
        );
        CREATE INDEX IF NOT EXISTS idx_edges_from ON knowledge_edges(from_memory_id, label);
        CREATE INDEX IF NOT EXISTS idx_edges_to ON knowledge_edges(to_memory_id, label);
        CREATE INDEX IF NOT EXISTS idx_edges_label ON knowledge_edges(label);

        CREATE VIRTUAL TABLE IF NOT EXISTS memory_vec USING vec0(
            memory_id TEXT PRIMARY KEY,
            embedding float[384]
        );

        CREATE VIRTUAL TABLE IF NOT EXISTS memory_fts USING fts5(
            content,
            content=memories,
            content_rowid=memory_rowid
        );

        CREATE TRIGGER IF NOT EXISTS memory_fts_insert AFTER INSERT ON memories BEGIN
            INSERT INTO memory_fts(rowid, content) VALUES (new.memory_rowid, new.content);
        END;
        CREATE TRIGGER IF NOT EXISTS memory_fts_delete AFTER DELETE ON memories BEGIN
            INSERT INTO memory_fts(memory_fts, rowid, content)
                VALUES ('delete', old.memory_rowid, old.content);
        END;
        CREATE TRIGGER IF NOT EXISTS memory_fts_update AFTER UPDATE OF content ON memories BEGIN
            INSERT INTO memory_fts(memory_fts, rowid, content)
                VALUES ('delete', old.memory_rowid, old.content);
            INSERT INTO memory_fts(rowid, content) VALUES (new.memory_rowid, new.content);
        END;

        CREATE TABLE IF NOT EXISTS namespaces (
            name TEXT PRIMARY KEY,
            description TEXT,
            created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
        );

        CREATE TABLE IF NOT EXISTS checkpoints (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            actor_id TEXT NOT NULL,
            name TEXT NOT NULL,
            event_id TEXT NOT NULL,
            metadata TEXT,
            created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_checkpoint_name ON checkpoints(session_id, name);

        CREATE TABLE IF NOT EXISTS branches (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            name TEXT,
            parent_branch_id TEXT,
            root_event_id TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
        );
        CREATE INDEX IF NOT EXISTS idx_branches_session ON branches(session_id);
    ").map_err(|e| {
        tracing::error!("V1 migration DDL failed: {e}");
        MemoryError::SchemaError("V1 migration failed".into())
    })?;

    tx.commit().map_err(|e| {
        tracing::error!("Migration commit failed: {e}");
        MemoryError::SchemaError("migration commit failed".into())
    })?;

    // user_version is not transactional in SQLite — set after successful commit
    conn.pragma_update(None, "user_version", 1u32).map_err(|e| {
        tracing::error!("Failed to set user_version: {e}");
        MemoryError::SchemaError("failed to record migration version".into())
    })?;

    // Post-migration verification — confirm tables exist
    conn.prepare("SELECT 1 FROM memories LIMIT 0").map_err(|e| {
        tracing::error!("Post-migration verification failed: {e}");
        MemoryError::SchemaError("migration verification failed".into())
    })?;

    Ok(())
}

/// API contract for all database operations. Implemented for rusqlite::Connection.
/// Downstream components (events.rs, memories.rs, etc.) accept &dyn Db.
///
/// # Safety / Threading
/// `Db` is intentionally NOT Send/Sync. It must only be used within the
/// `spawn_blocking` closure that holds the StoreManager mutex lock.
/// Do not capture `&dyn Db` across `.await` points or thread boundaries.
pub trait Db {
    // -- Store management (Component 1) --
    /// Get database size in bytes (page_count * page_size).
    fn db_size(&self) -> Result<u64, MemoryError>;

    // -- Events (Component 2) --

    /// Insert an immutable event. Returns the full Event with generated id and created_at.
    /// Precondition: params must be pre-validated by the business logic layer.
    fn insert_event(&self, params: &InsertEventParams<'_>) -> Result<Event, MemoryError>;

    /// Get a single event by ID, scoped to actor.
    fn get_event(&self, actor_id: &str, event_id: &str) -> Result<Event, MemoryError>;

    /// Get events for an actor+session, ordered by created_at ASC.
    /// Precondition: params must be pre-validated by the business logic layer.
    fn get_events(&self, params: &GetEventsParams<'_>) -> Result<Vec<Event>, MemoryError>;

    /// List distinct sessions for an actor with event counts and date ranges.
    fn list_sessions(
        &self,
        actor_id: &str,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<SessionInfo>, MemoryError>;

    /// Delete events past their expires_at. Returns count of deleted rows.
    fn delete_expired_events(&self) -> Result<u64, MemoryError>;

    // -- Memories (Component 3) --

    /// Insert a memory. If embedding is provided, also inserts into memory_vec.
    fn insert_memory(&self, params: &InsertMemoryParams<'_>) -> Result<Memory, MemoryError>;

    /// Get a single memory by ID, scoped to actor.
    fn get_memory(&self, actor_id: &str, memory_id: &str) -> Result<Memory, MemoryError>;

    /// List memories with filters. Ordered by created_at DESC.
    fn list_memories(&self, params: &ListMemoriesParams<'_>) -> Result<Vec<Memory>, MemoryError>;

    /// Consolidate a memory, scoped to actor.
    fn consolidate_memory(
        &self,
        actor_id: &str,
        memory_id: &str,
        action: &ConsolidateAction<'_>,
    ) -> Result<Memory, MemoryError>;

    /// Hard-delete a memory and its embedding, scoped to actor.
    fn delete_memory(&self, actor_id: &str, memory_id: &str) -> Result<(), MemoryError>;

    // -- Search (Component 4) --

    /// Full-text search over memory content via FTS5.
    /// `params.fts_query` must be pre-sanitized by the caller (search.rs).
    /// Returns memories ordered by BM25 relevance (negated rank, higher = better).
    fn search_fts(&self, params: &SearchFtsParams<'_>) -> Result<Vec<(Memory, f64)>, MemoryError>;

    /// Vector similarity search over memory embeddings via sqlite-vec.
    /// Returns memories ordered by L2 distance (ascending, lower = closer).
    /// Callers must convert distance to similarity if needed.
    fn search_vector(&self, params: &SearchVectorParams<'_>) -> Result<Vec<(Memory, f64)>, MemoryError>;

    // -- Graph (Component 5) --

    /// Insert a directed edge between two memories. Both memories must belong to actor_id.
    fn insert_edge(&self, params: &InsertEdgeParams<'_>) -> Result<Edge, MemoryError>;

    /// Get an edge by ID, scoped to actor (verified via joined memories).
    fn get_edge(&self, actor_id: &str, edge_id: &str) -> Result<Edge, MemoryError>;

    /// Get neighbors of a memory, scoped to actor. Returns edges + connected memories.
    fn get_neighbors(
        &self,
        actor_id: &str,
        memory_id: &str,
        direction: Direction,
        label: Option<&str>,
        limit: u32,
    ) -> Result<Vec<Neighbor>, MemoryError>;

    /// BFS traversal from a start memory via recursive CTE, scoped to actor.
    fn traverse(
        &self,
        actor_id: &str,
        start_memory_id: &str,
        max_depth: u32,
        label: Option<&str>,
        direction: Direction,
    ) -> Result<Vec<TraversalNode>, MemoryError>;

    /// Update an edge's label and/or properties, scoped to actor.
    fn update_edge(&self, params: &UpdateEdgeParams<'_>) -> Result<Edge, MemoryError>;

    /// Delete an edge by ID, scoped to actor.
    fn delete_edge(&self, actor_id: &str, edge_id: &str) -> Result<(), MemoryError>;

    /// List all distinct edge labels with counts, scoped to actor.
    fn list_edge_labels(&self, actor_id: &str) -> Result<Vec<LabelCount>, MemoryError>;

    /// Graph statistics, scoped to actor.
    fn graph_stats(&self, actor_id: &str) -> Result<GraphStats, MemoryError>;

    // -- Sessions (Component 6) --
    // fn create_checkpoint(...) -> Result<String, MemoryError>;
    // fn create_branch(...) -> Result<String, MemoryError>;
    // fn list_checkpoints(session_id: &str) -> Result<Vec<Checkpoint>, MemoryError>;
    // fn list_branches(session_id: &str) -> Result<Vec<Branch>, MemoryError>;

    // -- Namespaces (Component 7) --
    // fn create_namespace(name: &str, description: Option<&str>) -> Result<(), MemoryError>;
    // fn list_namespaces(prefix: Option<&str>) -> Result<Vec<Namespace>, MemoryError>;
    // fn delete_namespace(name: &str) -> Result<u64, MemoryError>; // returns count of deleted memories
}

fn row_to_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<Event> {
    Ok(Event {
        id: row.get(0)?,
        actor_id: row.get(1)?,
        session_id: row.get(2)?,
        event_type: row.get(3)?,
        role: row.get(4)?,
        content: row.get(5)?,
        blob_data: row.get(6)?,
        metadata: row.get(7)?,
        branch_id: row.get(8)?,
        created_at: row.get(9)?,
        expires_at: row.get(10)?,
    })
}

fn row_to_memory(row: &rusqlite::Row<'_>) -> rusqlite::Result<Memory> {
    Ok(Memory {
        id: row.get(0)?,
        actor_id: row.get(1)?,
        namespace: row.get(2)?,
        strategy: row.get(3)?,
        content: row.get(4)?,
        metadata: row.get(5)?,
        source_session_id: row.get(6)?,
        is_valid: row.get::<_, i32>(7)? != 0,
        superseded_by: row.get(8)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
    })
}

/// Escape LIKE wildcards in a string for safe prefix matching.
fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

fn row_to_edge(row: &rusqlite::Row<'_>) -> rusqlite::Result<Edge> {
    Ok(Edge {
        id: row.get(0)?,
        from_memory_id: row.get(1)?,
        to_memory_id: row.get(2)?,
        label: row.get(3)?,
        properties: row.get(4)?,
        created_at: row.get(5)?,
    })
}

fn row_to_neighbor(row: &rusqlite::Row<'_>) -> rusqlite::Result<Neighbor> {
    let edge = Edge {
        id: row.get(0)?,
        from_memory_id: row.get(1)?,
        to_memory_id: row.get(2)?,
        label: row.get(3)?,
        properties: row.get(4)?,
        created_at: row.get(5)?,
    };
    let memory = Memory {
        id: row.get(6)?,
        actor_id: row.get(7)?,
        namespace: row.get(8)?,
        strategy: row.get(9)?,
        content: row.get(10)?,
        metadata: row.get(11)?,
        source_session_id: row.get(12)?,
        is_valid: row.get::<_, i32>(13)? != 0,
        superseded_by: row.get(14)?,
        created_at: row.get(15)?,
        updated_at: row.get(16)?,
    };
    Ok(Neighbor { edge, memory })
}

fn row_to_traversal_node(row: &rusqlite::Row<'_>) -> rusqlite::Result<TraversalNode> {
    let _memory_id: String = row.get(0)?;
    let depth: u32 = row.get(1)?;
    let path_json: String = row.get(2)?;
    let path: Vec<String> = serde_json::from_str(&path_json).unwrap_or_default();
    let memory = Memory {
        id: row.get(3)?,
        actor_id: row.get(4)?,
        namespace: row.get(5)?,
        strategy: row.get(6)?,
        content: row.get(7)?,
        metadata: row.get(8)?,
        source_session_id: row.get(9)?,
        is_valid: row.get::<_, i32>(10)? != 0,
        superseded_by: row.get(11)?,
        created_at: row.get(12)?,
        updated_at: row.get(13)?,
    };
    Ok(TraversalNode { memory, depth, path })
}

// Static SQL for traverse — one per direction. Label clause appended optionally.
const SQL_TRAVERSE_OUT: &str =
    "WITH RECURSIVE graph_walk(memory_id, depth, path, visited) AS (
        SELECT :start_memory_id, 0, '[]', json_array(:start_memory_id)
        UNION ALL
        SELECT e.to_memory_id,
            gw.depth + 1,
            json_insert(gw.path, '$[#]', e.to_memory_id),
            json_insert(gw.visited, '$[#]', e.to_memory_id)
        FROM graph_walk gw
        JOIN knowledge_edges e ON e.from_memory_id = gw.memory_id
        JOIN memories mcheck ON mcheck.id = e.to_memory_id AND mcheck.actor_id = :actor_id
        WHERE gw.depth < :max_depth
          AND json_array_length(gw.visited) < 1000
          AND NOT EXISTS (
              SELECT 1 FROM json_each(gw.visited) WHERE value = e.to_memory_id
          )";

const SQL_TRAVERSE_IN: &str =
    "WITH RECURSIVE graph_walk(memory_id, depth, path, visited) AS (
        SELECT :start_memory_id, 0, '[]', json_array(:start_memory_id)
        UNION ALL
        SELECT e.from_memory_id,
            gw.depth + 1,
            json_insert(gw.path, '$[#]', e.from_memory_id),
            json_insert(gw.visited, '$[#]', e.from_memory_id)
        FROM graph_walk gw
        JOIN knowledge_edges e ON e.to_memory_id = gw.memory_id
        JOIN memories mcheck ON mcheck.id = e.from_memory_id AND mcheck.actor_id = :actor_id
        WHERE gw.depth < :max_depth
          AND json_array_length(gw.visited) < 1000
          AND NOT EXISTS (
              SELECT 1 FROM json_each(gw.visited) WHERE value = e.from_memory_id
          )";

const SQL_TRAVERSE_BOTH: &str =
    "WITH RECURSIVE graph_walk(memory_id, depth, path, visited) AS (
        SELECT :start_memory_id, 0, '[]', json_array(:start_memory_id)
        UNION ALL
        SELECT CASE WHEN e.from_memory_id = gw.memory_id THEN e.to_memory_id ELSE e.from_memory_id END,
            gw.depth + 1,
            json_insert(gw.path, '$[#]', CASE WHEN e.from_memory_id = gw.memory_id THEN e.to_memory_id ELSE e.from_memory_id END),
            json_insert(gw.visited, '$[#]', CASE WHEN e.from_memory_id = gw.memory_id THEN e.to_memory_id ELSE e.from_memory_id END)
        FROM graph_walk gw
        JOIN knowledge_edges e ON e.from_memory_id = gw.memory_id OR e.to_memory_id = gw.memory_id
        JOIN memories mcheck ON mcheck.id = CASE WHEN e.from_memory_id = gw.memory_id THEN e.to_memory_id ELSE e.from_memory_id END AND mcheck.actor_id = :actor_id
        WHERE gw.depth < :max_depth
          AND json_array_length(gw.visited) < 1000
          AND NOT EXISTS (
              SELECT 1 FROM json_each(gw.visited) WHERE value = CASE WHEN e.from_memory_id = gw.memory_id THEN e.to_memory_id ELSE e.from_memory_id END
          )";

const SQL_TRAVERSE_TAIL: &str =
    ")
     SELECT gw.memory_id, gw.depth, gw.path,
            m.id, m.actor_id, m.namespace, m.strategy, m.content, m.metadata,
            m.source_session_id, m.is_valid, m.superseded_by, m.created_at, m.updated_at
     FROM graph_walk gw
     JOIN memories m ON m.id = gw.memory_id AND m.actor_id = :actor_id
     WHERE gw.depth > 0
     ORDER BY gw.depth ASC, m.created_at DESC
     LIMIT 1000";

impl Db for Connection {
    fn db_size(&self) -> Result<u64, MemoryError> {
        let page_count: u64 = self.pragma_query_value(None, "page_count", |r| r.get(0))
            .map_err(|e| {
                tracing::error!("page_count query failed: {e}");
                MemoryError::QueryFailed("failed to query database size".into())
            })?;
        let page_size: u64 = self.pragma_query_value(None, "page_size", |r| r.get(0))
            .map_err(|e| {
                tracing::error!("page_size query failed: {e}");
                MemoryError::QueryFailed("failed to query database size".into())
            })?;
        Ok(page_count * page_size)
    }

    fn insert_event(&self, params: &InsertEventParams<'_>) -> Result<Event, MemoryError> {
        let id = uuid::Uuid::new_v4().to_string();
        self.query_row(
            "INSERT INTO events (id, actor_id, session_id, event_type, role, content, blob_data, metadata, branch_id, expires_at)
             VALUES (:id, :actor_id, :session_id, :event_type, :role, :content, :blob_data, :metadata, :branch_id, :expires_at)
             RETURNING id, actor_id, session_id, event_type, role, content, blob_data, metadata, branch_id, created_at, expires_at",
            rusqlite::named_params! {
                ":id": id,
                ":actor_id": params.actor_id,
                ":session_id": params.session_id,
                ":event_type": params.event_type,
                ":role": params.role,
                ":content": params.content,
                ":blob_data": params.blob_data,
                ":metadata": params.metadata,
                ":branch_id": params.branch_id,
                ":expires_at": params.expires_at,
            },
            row_to_event,
        )
        .map_err(|e| {
            tracing::error!("insert_event failed: {e}");
            MemoryError::QueryFailed("failed to insert event".into())
        })
    }

    fn get_event(&self, actor_id: &str, event_id: &str) -> Result<Event, MemoryError> {
        self.query_row(
            "SELECT id, actor_id, session_id, event_type, role, content, blob_data, metadata, branch_id, created_at, expires_at
             FROM events WHERE id = :id AND actor_id = :actor_id",
            rusqlite::named_params! { ":id": event_id, ":actor_id": actor_id },
            row_to_event,
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => {
                MemoryError::NotFound(event_id.to_string())
            }
            _ => {
                tracing::error!("get_event failed: {e}");
                MemoryError::QueryFailed("failed to get event".into())
            }
        })
    }

    fn get_events(&self, params: &GetEventsParams<'_>) -> Result<Vec<Event>, MemoryError> {
        let mut sql = String::from(
            "SELECT id, actor_id, session_id, event_type, role, content, blob_data, metadata, branch_id, created_at, expires_at
             FROM events WHERE actor_id = ?1 AND session_id = ?2",
        );

        // Track positional parameter index
        let mut bind_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        bind_values.push(Box::new(params.actor_id.to_string()));
        bind_values.push(Box::new(params.session_id.to_string()));

        match &params.branch_id {
            BranchFilter::All => {}
            BranchFilter::MainOnly => sql.push_str(" AND branch_id IS NULL"),
            BranchFilter::Specific(id) => {
                bind_values.push(Box::new(id.to_string()));
                sql.push_str(&format!(" AND branch_id = ?{}", bind_values.len()));
            }
        }
        if let Some(before) = params.before {
            bind_values.push(Box::new(before.to_string()));
            sql.push_str(&format!(" AND created_at < ?{}", bind_values.len()));
        }
        if let Some(after) = params.after {
            bind_values.push(Box::new(after.to_string()));
            sql.push_str(&format!(" AND created_at > ?{}", bind_values.len()));
        }

        bind_values.push(Box::new(params.limit));
        let limit_idx = bind_values.len();
        bind_values.push(Box::new(params.offset));
        let offset_idx = bind_values.len();
        sql.push_str(&format!(
            " ORDER BY created_at ASC, rowid ASC LIMIT ?{limit_idx} OFFSET ?{offset_idx}"
        ));

        let mut stmt = self.prepare(&sql).map_err(|e| {
            tracing::error!("get_events prepare failed: {e}");
            MemoryError::QueryFailed("failed to prepare get_events query".into())
        })?;

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            bind_values.iter().map(|b| b.as_ref()).collect();
        let rows = stmt
            .query_map(param_refs.as_slice(), row_to_event)
            .map_err(|e| {
                tracing::error!("get_events query failed: {e}");
                MemoryError::QueryFailed("failed to query events".into())
            })?;

        let mut events = Vec::new();
        for row in rows {
            events.push(row.map_err(|e| {
                tracing::error!("get_events row read failed: {e}");
                MemoryError::QueryFailed("failed to read event row".into())
            })?);
        }
        Ok(events)
    }

    fn list_sessions(
        &self,
        actor_id: &str,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<SessionInfo>, MemoryError> {
        let mut stmt = self
            .prepare(
                "SELECT session_id, COUNT(*) as event_count,
                        MIN(created_at) as first_event_at,
                        MAX(created_at) as last_event_at
                 FROM events
                 WHERE actor_id = :actor_id
                 GROUP BY session_id
                 ORDER BY last_event_at DESC
                 LIMIT :limit OFFSET :offset",
            )
            .map_err(|e| {
                tracing::error!("list_sessions prepare failed: {e}");
                MemoryError::QueryFailed("failed to prepare list_sessions query".into())
            })?;

        let rows = stmt
            .query_map(
                rusqlite::named_params! {
                    ":actor_id": actor_id,
                    ":limit": limit,
                    ":offset": offset,
                },
                |row| {
                    Ok(SessionInfo {
                        session_id: row.get(0)?,
                        actor_id: actor_id.to_string(),
                        event_count: row.get(1)?,
                        first_event_at: row.get(2)?,
                        last_event_at: row.get(3)?,
                    })
                },
            )
            .map_err(|e| {
                tracing::error!("list_sessions query failed: {e}");
                MemoryError::QueryFailed("failed to query sessions".into())
            })?;

        let mut sessions = Vec::new();
        for row in rows {
            sessions.push(row.map_err(|e| {
                tracing::error!("list_sessions row read failed: {e}");
                MemoryError::QueryFailed("failed to read session row".into())
            })?);
        }
        Ok(sessions)
    }

    fn delete_expired_events(&self) -> Result<u64, MemoryError> {
        let count = self
            .execute(
                "DELETE FROM events WHERE expires_at IS NOT NULL AND expires_at < strftime('%Y-%m-%dT%H:%M:%SZ', 'now')",
                [],
            )
            .map_err(|e| {
                tracing::error!("delete_expired_events failed: {e}");
                MemoryError::QueryFailed("failed to delete expired events".into())
            })?;
        Ok(count as u64)
    }

    // -- Memories (Component 3) --

    fn insert_memory(&self, params: &InsertMemoryParams<'_>) -> Result<Memory, MemoryError> {
        let id = uuid::Uuid::new_v4().to_string();
        let namespace = params.namespace.unwrap_or("default");

        if let Some(emb) = params.embedding {
            let tx = self.unchecked_transaction().map_err(|e| {
                tracing::error!("insert_memory transaction failed: {e}");
                MemoryError::QueryFailed("failed to begin transaction".into())
            })?;

            let memory = tx.query_row(
                "INSERT INTO memories (id, actor_id, namespace, strategy, content, metadata, source_session_id)
                 VALUES (:id, :actor_id, :namespace, :strategy, :content, :metadata, :source_session_id)
                 RETURNING id, actor_id, namespace, strategy, content, metadata, source_session_id,
                           is_valid, superseded_by, created_at, updated_at",
                rusqlite::named_params! {
                    ":id": id,
                    ":actor_id": params.actor_id,
                    ":namespace": namespace,
                    ":strategy": params.strategy,
                    ":content": params.content,
                    ":metadata": params.metadata,
                    ":source_session_id": params.source_session_id,
                },
                row_to_memory,
            ).map_err(|e| {
                tracing::error!("insert_memory failed: {e}");
                MemoryError::QueryFailed("failed to insert memory".into())
            })?;

            let emb_bytes: Vec<u8> = emb.iter().flat_map(|f| f.to_le_bytes()).collect();
            tx.execute(
                "INSERT INTO memory_vec (memory_id, embedding) VALUES (:id, :embedding)",
                rusqlite::named_params! { ":id": memory.id, ":embedding": emb_bytes },
            ).map_err(|e| {
                tracing::error!("insert_memory embedding failed: {e}");
                MemoryError::QueryFailed("failed to insert embedding".into())
            })?;

            tx.commit().map_err(|e| {
                tracing::error!("insert_memory commit failed: {e}");
                MemoryError::QueryFailed("failed to commit memory insert".into())
            })?;

            Ok(memory)
        } else {
            self.query_row(
                "INSERT INTO memories (id, actor_id, namespace, strategy, content, metadata, source_session_id)
                 VALUES (:id, :actor_id, :namespace, :strategy, :content, :metadata, :source_session_id)
                 RETURNING id, actor_id, namespace, strategy, content, metadata, source_session_id,
                           is_valid, superseded_by, created_at, updated_at",
                rusqlite::named_params! {
                    ":id": id,
                    ":actor_id": params.actor_id,
                    ":namespace": namespace,
                    ":strategy": params.strategy,
                    ":content": params.content,
                    ":metadata": params.metadata,
                    ":source_session_id": params.source_session_id,
                },
                row_to_memory,
            ).map_err(|e| {
                tracing::error!("insert_memory failed: {e}");
                MemoryError::QueryFailed("failed to insert memory".into())
            })
        }
    }

    fn get_memory(&self, actor_id: &str, memory_id: &str) -> Result<Memory, MemoryError> {
        self.query_row(
            "SELECT id, actor_id, namespace, strategy, content, metadata, source_session_id,
                    is_valid, superseded_by, created_at, updated_at
             FROM memories WHERE id = :id AND actor_id = :actor_id",
            rusqlite::named_params! { ":id": memory_id, ":actor_id": actor_id },
            row_to_memory,
        ).map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => MemoryError::NotFound(memory_id.to_string()),
            _ => {
                tracing::error!("get_memory failed: {e}");
                MemoryError::QueryFailed("failed to get memory".into())
            }
        })
    }

    fn list_memories(&self, params: &ListMemoriesParams<'_>) -> Result<Vec<Memory>, MemoryError> {
        let mut sql = String::from(
            "SELECT id, actor_id, namespace, strategy, content, metadata, source_session_id,
                    is_valid, superseded_by, created_at, updated_at
             FROM memories WHERE actor_id = ?1",
        );
        let mut bind_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        bind_values.push(Box::new(params.actor_id.to_string()));

        if let Some(ns) = params.namespace {
            bind_values.push(Box::new(ns.to_string()));
            sql.push_str(&format!(" AND namespace = ?{}", bind_values.len()));
        }
        if let Some(prefix) = params.namespace_prefix {
            let escaped = format!("{}%", escape_like(prefix));
            bind_values.push(Box::new(escaped));
            sql.push_str(&format!(" AND namespace LIKE ?{} ESCAPE '\\'", bind_values.len()));
        }
        if let Some(strategy) = params.strategy {
            bind_values.push(Box::new(strategy.to_string()));
            sql.push_str(&format!(" AND strategy = ?{}", bind_values.len()));
        }
        if params.valid_only {
            sql.push_str(" AND is_valid = 1");
        }

        bind_values.push(Box::new(params.limit));
        let limit_idx = bind_values.len();
        bind_values.push(Box::new(params.offset));
        let offset_idx = bind_values.len();
        sql.push_str(&format!(" ORDER BY created_at DESC LIMIT ?{limit_idx} OFFSET ?{offset_idx}"));

        let mut stmt = self.prepare(&sql).map_err(|e| {
            tracing::error!("list_memories prepare failed: {e}");
            MemoryError::QueryFailed("failed to prepare list_memories query".into())
        })?;

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            bind_values.iter().map(|b| b.as_ref()).collect();
        let rows = stmt.query_map(param_refs.as_slice(), row_to_memory).map_err(|e| {
            tracing::error!("list_memories query failed: {e}");
            MemoryError::QueryFailed("failed to query memories".into())
        })?;

        let mut memories = Vec::new();
        for row in rows {
            memories.push(row.map_err(|e| {
                tracing::error!("list_memories row read failed: {e}");
                MemoryError::QueryFailed("failed to read memory row".into())
            })?);
        }
        Ok(memories)
    }

    fn consolidate_memory(
        &self,
        actor_id: &str,
        memory_id: &str,
        action: &ConsolidateAction<'_>,
    ) -> Result<Memory, MemoryError> {
        match action {
            ConsolidateAction::Update { content, embedding } => {
                let tx = self.unchecked_transaction().map_err(|e| {
                    tracing::error!("consolidate_memory transaction failed: {e}");
                    MemoryError::QueryFailed("failed to begin transaction".into())
                })?;

                // 1. Fetch old memory to get namespace/strategy
                let (old_namespace, old_strategy): (String, String) = tx.query_row(
                    "SELECT namespace, strategy FROM memories
                     WHERE id = :id AND actor_id = :actor_id AND is_valid = 1",
                    rusqlite::named_params! { ":id": memory_id, ":actor_id": actor_id },
                    |row| Ok((row.get(0)?, row.get(1)?)),
                ).map_err(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => MemoryError::NotFound(memory_id.to_string()),
                    _ => {
                        tracing::error!("consolidate_memory fetch failed: {e}");
                        MemoryError::QueryFailed("failed to fetch memory for consolidation".into())
                    }
                })?;

                // 2. Insert new memory
                let new_id = uuid::Uuid::new_v4().to_string();
                let new_memory = tx.query_row(
                    "INSERT INTO memories (id, actor_id, namespace, strategy, content)
                     VALUES (:id, :actor_id, :namespace, :strategy, :content)
                     RETURNING id, actor_id, namespace, strategy, content, metadata, source_session_id,
                               is_valid, superseded_by, created_at, updated_at",
                    rusqlite::named_params! {
                        ":id": new_id,
                        ":actor_id": actor_id,
                        ":namespace": old_namespace,
                        ":strategy": old_strategy,
                        ":content": content,
                    },
                    row_to_memory,
                ).map_err(|e| {
                    tracing::error!("consolidate_memory insert failed: {e}");
                    MemoryError::QueryFailed("failed to insert consolidated memory".into())
                })?;

                // 3. Mark old memory invalid
                tx.execute(
                    "UPDATE memories SET is_valid = 0, superseded_by = :new_id,
                            updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
                     WHERE id = :old_id AND actor_id = :actor_id AND is_valid = 1",
                    rusqlite::named_params! {
                        ":new_id": new_memory.id,
                        ":old_id": memory_id,
                        ":actor_id": actor_id,
                    },
                ).map_err(|e| {
                    tracing::error!("consolidate_memory update failed: {e}");
                    MemoryError::QueryFailed("failed to invalidate old memory".into())
                })?;

                // 4. Delete old embedding
                tx.execute(
                    "DELETE FROM memory_vec WHERE memory_id = :id",
                    rusqlite::named_params! { ":id": memory_id },
                ).map_err(|e| {
                    tracing::error!("consolidate_memory delete old embedding failed: {e}");
                    MemoryError::QueryFailed("failed to delete old embedding".into())
                })?;

                // 5. Insert new embedding if provided
                if let Some(emb) = embedding {
                    let emb_bytes: Vec<u8> = emb.iter().flat_map(|f| f.to_le_bytes()).collect();
                    tx.execute(
                        "INSERT INTO memory_vec (memory_id, embedding) VALUES (:id, :embedding)",
                        rusqlite::named_params! { ":id": new_memory.id, ":embedding": emb_bytes },
                    ).map_err(|e| {
                        tracing::error!("consolidate_memory insert embedding failed: {e}");
                        MemoryError::QueryFailed("failed to insert new embedding".into())
                    })?;
                }

                tx.commit().map_err(|e| {
                    tracing::error!("consolidate_memory commit failed: {e}");
                    MemoryError::QueryFailed("failed to commit consolidation".into())
                })?;

                Ok(new_memory)
            }
            ConsolidateAction::Invalidate => {
                self.query_row(
                    "UPDATE memories SET is_valid = 0,
                            updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
                     WHERE id = :id AND actor_id = :actor_id AND is_valid = 1
                     RETURNING id, actor_id, namespace, strategy, content, metadata, source_session_id,
                               is_valid, superseded_by, created_at, updated_at",
                    rusqlite::named_params! { ":id": memory_id, ":actor_id": actor_id },
                    row_to_memory,
                ).map_err(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => MemoryError::NotFound(memory_id.to_string()),
                    _ => {
                        tracing::error!("consolidate_memory invalidate failed: {e}");
                        MemoryError::QueryFailed("failed to invalidate memory".into())
                    }
                })
            }
        }
    }

    fn delete_memory(&self, actor_id: &str, memory_id: &str) -> Result<(), MemoryError> {
        let tx = self.unchecked_transaction().map_err(|e| {
            tracing::error!("delete_memory transaction failed: {e}");
            MemoryError::QueryFailed("failed to begin transaction".into())
        })?;

        // Delete from memories first (verifies actor ownership)
        let count = tx.execute(
            "DELETE FROM memories WHERE id = :id AND actor_id = :actor_id",
            rusqlite::named_params! { ":id": memory_id, ":actor_id": actor_id },
        ).map_err(|e| {
            tracing::error!("delete_memory failed: {e}");
            MemoryError::QueryFailed("failed to delete memory".into())
        })?;

        if count == 0 {
            return Err(MemoryError::NotFound(memory_id.to_string()));
        }

        // Delete embedding (only reached if ownership verified)
        tx.execute(
            "DELETE FROM memory_vec WHERE memory_id = :id",
            rusqlite::named_params! { ":id": memory_id },
        ).map_err(|e| {
            tracing::error!("delete_memory embedding failed: {e}");
            MemoryError::QueryFailed("failed to delete embedding".into())
        })?;

        tx.commit().map_err(|e| {
            tracing::error!("delete_memory commit failed: {e}");
            MemoryError::QueryFailed("failed to commit memory deletion".into())
        })?;

        Ok(())
    }

    // -- Search (Component 4) --

    fn search_fts(&self, params: &SearchFtsParams<'_>) -> Result<Vec<(Memory, f64)>, MemoryError> {
        let mut sql = String::from(
            "SELECT m.id, m.actor_id, m.namespace, m.strategy, m.content, m.metadata,
                    m.source_session_id, m.is_valid, m.superseded_by, m.created_at, m.updated_at,
                    -rank AS score
             FROM memory_fts
             JOIN memories m ON memory_fts.rowid = m.memory_rowid
             WHERE memory_fts MATCH ?1
               AND m.actor_id = ?2
               AND m.is_valid = 1",
        );
        let mut bind: Vec<Box<dyn rusqlite::types::ToSql>> = vec![
            Box::new(params.fts_query.to_string()),
            Box::new(params.actor_id.to_string()),
        ];

        if let Some(ns) = params.namespace {
            bind.push(Box::new(ns.to_string()));
            sql.push_str(&format!(" AND m.namespace = ?{}", bind.len()));
        }
        if let Some(prefix) = params.namespace_prefix {
            bind.push(Box::new(format!("{}%", escape_like(prefix))));
            sql.push_str(&format!(" AND m.namespace LIKE ?{} ESCAPE '\\'", bind.len()));
        }
        if let Some(s) = params.strategy {
            bind.push(Box::new(s.to_string()));
            sql.push_str(&format!(" AND m.strategy = ?{}", bind.len()));
        }

        bind.push(Box::new(params.limit));
        sql.push_str(&format!(" ORDER BY rank LIMIT ?{}", bind.len()));

        let mut stmt = self.prepare(&sql).map_err(|e| {
            tracing::error!("search_fts prepare failed: {e}");
            MemoryError::QueryFailed("failed to prepare FTS search query".into())
        })?;

        let refs: Vec<&dyn rusqlite::types::ToSql> = bind.iter().map(|b| b.as_ref()).collect();
        let rows = stmt
            .query_map(refs.as_slice(), |row| {
                Ok((row_to_memory(row)?, row.get::<_, f64>(11)?))
            })
            .map_err(|e| {
                tracing::error!("search_fts query failed: {e}");
                MemoryError::QueryFailed("failed to execute FTS search".into())
            })?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(|e| {
                tracing::error!("search_fts row read failed: {e}");
                MemoryError::QueryFailed("failed to read FTS search result".into())
            })?);
        }
        Ok(results)
    }

    fn search_vector(&self, params: &SearchVectorParams<'_>) -> Result<Vec<(Memory, f64)>, MemoryError> {
        use crate::search::{VECTOR_OVERFETCH_FACTOR, MAX_K_OVERFETCH};

        if params.embedding.len() != EMBEDDING_DIM as usize {
            return Err(MemoryError::InvalidInput(format!(
                "embedding must have exactly {EMBEDDING_DIM} dimensions"
            )));
        }
        if params.embedding.iter().any(|v| !v.is_finite()) {
            return Err(MemoryError::InvalidInput(
                "embedding contains NaN or infinity".into(),
            ));
        }

        let query_bytes: Vec<u8> = params
            .embedding
            .iter()
            .flat_map(|f| f.to_le_bytes())
            .collect();
        let k_overfetch = (params.limit * VECTOR_OVERFETCH_FACTOR).min(MAX_K_OVERFETCH);

        let mut sql = String::from(
            "WITH knn AS (
                 SELECT memory_id, distance
                 FROM memory_vec
                 WHERE embedding MATCH ?1 AND k = ?2
             )
             SELECT m.id, m.actor_id, m.namespace, m.strategy, m.content, m.metadata,
                    m.source_session_id, m.is_valid, m.superseded_by, m.created_at, m.updated_at,
                    knn.distance
             FROM knn
             JOIN memories m ON knn.memory_id = m.id
             WHERE m.actor_id = ?3
               AND m.is_valid = 1",
        );
        let mut bind: Vec<Box<dyn rusqlite::types::ToSql>> = vec![
            Box::new(query_bytes),
            Box::new(k_overfetch),
            Box::new(params.actor_id.to_string()),
        ];

        if let Some(ns) = params.namespace {
            bind.push(Box::new(ns.to_string()));
            sql.push_str(&format!(" AND m.namespace = ?{}", bind.len()));
        }
        if let Some(prefix) = params.namespace_prefix {
            bind.push(Box::new(format!("{}%", escape_like(prefix))));
            sql.push_str(&format!(" AND m.namespace LIKE ?{} ESCAPE '\\'", bind.len()));
        }
        if let Some(s) = params.strategy {
            bind.push(Box::new(s.to_string()));
            sql.push_str(&format!(" AND m.strategy = ?{}", bind.len()));
        }

        bind.push(Box::new(params.limit));
        sql.push_str(&format!(" ORDER BY knn.distance ASC LIMIT ?{}", bind.len()));

        let mut stmt = self.prepare(&sql).map_err(|e| {
            tracing::error!("search_vector prepare failed: {e}");
            MemoryError::QueryFailed("failed to prepare vector search query".into())
        })?;

        let refs: Vec<&dyn rusqlite::types::ToSql> = bind.iter().map(|b| b.as_ref()).collect();
        let rows = stmt
            .query_map(refs.as_slice(), |row| {
                Ok((row_to_memory(row)?, row.get::<_, f64>(11)?))
            })
            .map_err(|e| {
                tracing::error!("search_vector query failed: {e}");
                MemoryError::QueryFailed("failed to execute vector search".into())
            })?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(|e| {
                tracing::error!("search_vector row read failed: {e}");
                MemoryError::QueryFailed("failed to read vector search result".into())
            })?);
        }
        Ok(results)
    }
    // -- Graph (Component 5) --

    fn insert_edge(&self, params: &InsertEdgeParams<'_>) -> Result<Edge, MemoryError> {
        // Verify both memories exist and belong to actor
        let count: u32 = self.query_row(
            "SELECT COUNT(*) FROM memories WHERE id IN (:from_id, :to_id) AND actor_id = :actor_id",
            rusqlite::named_params! {
                ":from_id": params.from_memory_id,
                ":to_id": params.to_memory_id,
                ":actor_id": params.actor_id,
            },
            |row| row.get(0),
        ).map_err(|e| {
            tracing::error!("insert_edge verify failed: {e}");
            MemoryError::QueryFailed("failed to verify memories".into())
        })?;
        if count != 2 {
            return Err(MemoryError::NotFound("memory not found".into()));
        }

        let id = uuid::Uuid::new_v4().to_string();
        self.query_row(
            "INSERT INTO knowledge_edges (id, from_memory_id, to_memory_id, label, properties)
             VALUES (:id, :from_memory_id, :to_memory_id, :label, :properties)
             RETURNING id, from_memory_id, to_memory_id, label, properties, created_at",
            rusqlite::named_params! {
                ":id": id,
                ":from_memory_id": params.from_memory_id,
                ":to_memory_id": params.to_memory_id,
                ":label": params.label,
                ":properties": params.properties,
            },
            row_to_edge,
        ).map_err(|e| {
            tracing::error!("insert_edge failed: {e}");
            MemoryError::QueryFailed("failed to insert edge".into())
        })
    }

    fn get_edge(&self, actor_id: &str, edge_id: &str) -> Result<Edge, MemoryError> {
        self.query_row(
            "SELECT e.id, e.from_memory_id, e.to_memory_id, e.label, e.properties, e.created_at
             FROM knowledge_edges e
             JOIN memories m ON m.id = e.from_memory_id
             WHERE e.id = :edge_id AND m.actor_id = :actor_id",
            rusqlite::named_params! { ":edge_id": edge_id, ":actor_id": actor_id },
            row_to_edge,
        ).map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => MemoryError::NotFound(edge_id.to_string()),
            _ => {
                tracing::error!("get_edge failed: {e}");
                MemoryError::QueryFailed("failed to get edge".into())
            }
        })
    }

    fn get_neighbors(
        &self,
        actor_id: &str,
        memory_id: &str,
        direction: Direction,
        label: Option<&str>,
        limit: u32,
    ) -> Result<Vec<Neighbor>, MemoryError> {
        let base_sql = match direction {
            Direction::Out => {
                "SELECT e.id, e.from_memory_id, e.to_memory_id, e.label, e.properties, e.created_at,
                        m.id, m.actor_id, m.namespace, m.strategy, m.content, m.metadata,
                        m.source_session_id, m.is_valid, m.superseded_by, m.created_at, m.updated_at
                 FROM knowledge_edges e
                 JOIN memories m ON m.id = e.to_memory_id
                 WHERE e.from_memory_id = :memory_id AND m.actor_id = :actor_id"
            }
            Direction::In => {
                "SELECT e.id, e.from_memory_id, e.to_memory_id, e.label, e.properties, e.created_at,
                        m.id, m.actor_id, m.namespace, m.strategy, m.content, m.metadata,
                        m.source_session_id, m.is_valid, m.superseded_by, m.created_at, m.updated_at
                 FROM knowledge_edges e
                 JOIN memories m ON m.id = e.from_memory_id
                 WHERE e.to_memory_id = :memory_id AND m.actor_id = :actor_id"
            }
            Direction::Both => {
                "SELECT e.id, e.from_memory_id, e.to_memory_id, e.label, e.properties, e.created_at,
                        m.id, m.actor_id, m.namespace, m.strategy, m.content, m.metadata,
                        m.source_session_id, m.is_valid, m.superseded_by, m.created_at, m.updated_at
                 FROM knowledge_edges e
                 JOIN memories m ON m.id = CASE
                     WHEN e.from_memory_id = :memory_id THEN e.to_memory_id
                     ELSE e.from_memory_id
                 END
                 WHERE (e.from_memory_id = :memory_id OR e.to_memory_id = :memory_id)
                   AND m.actor_id = :actor_id"
            }
        };

        let sql = if label.is_some() {
            format!("{base_sql} AND e.label = :label ORDER BY e.created_at DESC LIMIT :limit")
        } else {
            format!("{base_sql} ORDER BY e.created_at DESC LIMIT :limit")
        };

        let mut stmt = self.prepare(&sql).map_err(|e| {
            tracing::error!("get_neighbors prepare failed: {e}");
            MemoryError::QueryFailed("failed to prepare get_neighbors query".into())
        })?;

        let rows = if let Some(lbl) = label {
            stmt.query_map(
                rusqlite::named_params! {
                    ":memory_id": memory_id,
                    ":actor_id": actor_id,
                    ":label": lbl,
                    ":limit": limit,
                },
                row_to_neighbor,
            )
        } else {
            stmt.query_map(
                rusqlite::named_params! {
                    ":memory_id": memory_id,
                    ":actor_id": actor_id,
                    ":limit": limit,
                },
                row_to_neighbor,
            )
        }.map_err(|e| {
            tracing::error!("get_neighbors query failed: {e}");
            MemoryError::QueryFailed("failed to query neighbors".into())
        })?;

        let mut neighbors = Vec::new();
        for row in rows {
            neighbors.push(row.map_err(|e| {
                tracing::error!("get_neighbors row read failed: {e}");
                MemoryError::QueryFailed("failed to read neighbor row".into())
            })?);
        }
        Ok(neighbors)
    }

    fn traverse(
        &self,
        actor_id: &str,
        start_memory_id: &str,
        max_depth: u32,
        label: Option<&str>,
        direction: Direction,
    ) -> Result<Vec<TraversalNode>, MemoryError> {
        // Verify start memory exists and belongs to actor
        self.query_row(
            "SELECT 1 FROM memories WHERE id = :id AND actor_id = :actor_id",
            rusqlite::named_params! { ":id": start_memory_id, ":actor_id": actor_id },
            |_| Ok(()),
        ).map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => MemoryError::NotFound(start_memory_id.to_string()),
            _ => {
                tracing::error!("traverse verify failed: {e}");
                MemoryError::QueryFailed("failed to verify start memory".into())
            }
        })?;

        let base = match direction {
            Direction::Out => SQL_TRAVERSE_OUT,
            Direction::In => SQL_TRAVERSE_IN,
            Direction::Both => SQL_TRAVERSE_BOTH,
        };

        let sql = if label.is_some() {
            format!("{base} AND e.label = :label{SQL_TRAVERSE_TAIL}")
        } else {
            format!("{base}{SQL_TRAVERSE_TAIL}")
        };

        let mut stmt = self.prepare(&sql).map_err(|e| {
            tracing::error!("traverse prepare failed: {e}");
            MemoryError::QueryFailed("failed to prepare traverse query".into())
        })?;

        let rows = if let Some(lbl) = label {
            stmt.query_map(
                rusqlite::named_params! {
                    ":start_memory_id": start_memory_id,
                    ":actor_id": actor_id,
                    ":max_depth": max_depth,
                    ":label": lbl,
                },
                row_to_traversal_node,
            )
        } else {
            stmt.query_map(
                rusqlite::named_params! {
                    ":start_memory_id": start_memory_id,
                    ":actor_id": actor_id,
                    ":max_depth": max_depth,
                },
                row_to_traversal_node,
            )
        }.map_err(|e| {
            tracing::error!("traverse query failed: {e}");
            MemoryError::QueryFailed("failed to execute traverse".into())
        })?;

        let mut nodes = Vec::new();
        for row in rows {
            nodes.push(row.map_err(|e| {
                tracing::error!("traverse row read failed: {e}");
                MemoryError::QueryFailed("failed to read traversal row".into())
            })?);
        }
        Ok(nodes)
    }

    fn update_edge(&self, params: &UpdateEdgeParams<'_>) -> Result<Edge, MemoryError> {
        let result = match (params.label, params.properties) {
            (Some(label), Some(props)) => {
                self.query_row(
                    "UPDATE knowledge_edges SET label = :label, properties = :properties
                     WHERE id = :edge_id
                       AND EXISTS (SELECT 1 FROM memories WHERE id = knowledge_edges.from_memory_id AND actor_id = :actor_id)
                     RETURNING id, from_memory_id, to_memory_id, label, properties, created_at",
                    rusqlite::named_params! {
                        ":edge_id": params.edge_id,
                        ":actor_id": params.actor_id,
                        ":label": label,
                        ":properties": props,
                    },
                    row_to_edge,
                )
            }
            (Some(label), None) => {
                self.query_row(
                    "UPDATE knowledge_edges SET label = :label
                     WHERE id = :edge_id
                       AND EXISTS (SELECT 1 FROM memories WHERE id = knowledge_edges.from_memory_id AND actor_id = :actor_id)
                     RETURNING id, from_memory_id, to_memory_id, label, properties, created_at",
                    rusqlite::named_params! {
                        ":edge_id": params.edge_id,
                        ":actor_id": params.actor_id,
                        ":label": label,
                    },
                    row_to_edge,
                )
            }
            (None, Some(props)) => {
                self.query_row(
                    "UPDATE knowledge_edges SET properties = :properties
                     WHERE id = :edge_id
                       AND EXISTS (SELECT 1 FROM memories WHERE id = knowledge_edges.from_memory_id AND actor_id = :actor_id)
                     RETURNING id, from_memory_id, to_memory_id, label, properties, created_at",
                    rusqlite::named_params! {
                        ":edge_id": params.edge_id,
                        ":actor_id": params.actor_id,
                        ":properties": props,
                    },
                    row_to_edge,
                )
            }
            (None, None) => unreachable!("validated by caller"),
        };

        result.map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => MemoryError::NotFound(params.edge_id.to_string()),
            _ => {
                tracing::error!("update_edge failed: {e}");
                MemoryError::QueryFailed("failed to update edge".into())
            }
        })
    }

    fn delete_edge(&self, actor_id: &str, edge_id: &str) -> Result<(), MemoryError> {
        let count = self.execute(
            "DELETE FROM knowledge_edges
             WHERE id = :edge_id
               AND EXISTS (SELECT 1 FROM memories WHERE id = knowledge_edges.from_memory_id AND actor_id = :actor_id)",
            rusqlite::named_params! { ":edge_id": edge_id, ":actor_id": actor_id },
        ).map_err(|e| {
            tracing::error!("delete_edge failed: {e}");
            MemoryError::QueryFailed("failed to delete edge".into())
        })?;
        if count == 0 {
            return Err(MemoryError::NotFound(edge_id.to_string()));
        }
        Ok(())
    }

    fn list_edge_labels(&self, actor_id: &str) -> Result<Vec<LabelCount>, MemoryError> {
        let mut stmt = self.prepare(
            "SELECT e.label, COUNT(*) as count
             FROM knowledge_edges e
             JOIN memories m ON m.id = e.from_memory_id
             WHERE m.actor_id = :actor_id
             GROUP BY e.label ORDER BY count DESC"
        ).map_err(|e| {
            tracing::error!("list_edge_labels prepare failed: {e}");
            MemoryError::QueryFailed("failed to prepare list_edge_labels query".into())
        })?;

        let rows = stmt.query_map(
            rusqlite::named_params! { ":actor_id": actor_id },
            |row| {
            Ok(LabelCount {
                label: row.get(0)?,
                count: row.get(1)?,
            })
        }).map_err(|e| {
            tracing::error!("list_edge_labels query failed: {e}");
            MemoryError::QueryFailed("failed to query edge labels".into())
        })?;

        let mut labels = Vec::new();
        for row in rows {
            labels.push(row.map_err(|e| {
                tracing::error!("list_edge_labels row read failed: {e}");
                MemoryError::QueryFailed("failed to read label row".into())
            })?);
        }
        Ok(labels)
    }

    fn graph_stats(&self, actor_id: &str) -> Result<GraphStats, MemoryError> {
        let total_edges: u64 = self.query_row(
            "SELECT COUNT(*) FROM knowledge_edges e
             JOIN memories m ON m.id = e.from_memory_id
             WHERE m.actor_id = :actor_id",
            rusqlite::named_params! { ":actor_id": actor_id },
            |row| row.get(0),
        ).map_err(|e| {
            tracing::error!("graph_stats count failed: {e}");
            MemoryError::QueryFailed("failed to count edges".into())
        })?;

        let labels = self.list_edge_labels(actor_id)?;

        let mut stmt = self.prepare(
            "SELECT memory_id, COUNT(*) as edge_count FROM (
                 SELECT e.from_memory_id AS memory_id FROM knowledge_edges e
                 JOIN memories m ON m.id = e.from_memory_id WHERE m.actor_id = :actor_id
                 UNION ALL
                 SELECT e.to_memory_id AS memory_id FROM knowledge_edges e
                 JOIN memories m ON m.id = e.from_memory_id WHERE m.actor_id = :actor_id
             )
             GROUP BY memory_id
             ORDER BY edge_count DESC
             LIMIT 10"
        ).map_err(|e| {
            tracing::error!("graph_stats most_connected prepare failed: {e}");
            MemoryError::QueryFailed("failed to prepare most_connected query".into())
        })?;

        let rows = stmt.query_map(
            rusqlite::named_params! { ":actor_id": actor_id },
            |row| {
            Ok(ConnectedMemory {
                memory_id: row.get(0)?,
                edge_count: row.get(1)?,
            })
        }).map_err(|e| {
            tracing::error!("graph_stats most_connected query failed: {e}");
            MemoryError::QueryFailed("failed to query most connected".into())
        })?;

        let mut most_connected = Vec::new();
        for row in rows {
            most_connected.push(row.map_err(|e| {
                tracing::error!("graph_stats row read failed: {e}");
                MemoryError::QueryFailed("failed to read most connected row".into())
            })?);
        }

        Ok(GraphStats { total_edges, labels, most_connected })
    }
}

// Compile-time object safety assertion
fn _assert_object_safe(_: &dyn Db) {}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn open_temp() -> (TempDir, Connection) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.db");
        let conn = open(&path).unwrap();
        (dir, conn)
    }

    #[test]
    fn test_open_creates_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.db");
        assert!(!path.exists());
        open(&path).unwrap();
        assert!(path.exists());
    }

    #[test]
    fn test_open_wal_mode() {
        let (_dir, conn) = open_temp();
        let mode: String = conn.pragma_query_value(None, "journal_mode", |r| r.get(0)).unwrap();
        assert_eq!(mode, "wal");
    }

    #[test]
    fn test_open_foreign_keys() {
        let (_dir, conn) = open_temp();
        let fk: i32 = conn.pragma_query_value(None, "foreign_keys", |r| r.get(0)).unwrap();
        assert_eq!(fk, 1);
    }

    #[test]
    fn test_open_exclusive_locking() {
        let (_dir, conn) = open_temp();
        let mode: String = conn.pragma_query_value(None, "locking_mode", |r| r.get(0)).unwrap();
        assert_eq!(mode, "exclusive");
    }

    #[test]
    fn test_sqlite_vec_loaded() {
        let (_dir, conn) = open_temp();
        let version: String = conn.query_row("SELECT vec_version()", [], |r| r.get(0)).unwrap();
        assert!(!version.is_empty());
    }

    #[test]
    fn test_migrate_fresh_db() {
        let (_dir, conn) = open_temp();
        let tables: Vec<String> = {
            let mut stmt = conn.prepare(
                "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name"
            ).unwrap();
            stmt.query_map([], |r| r.get(0)).unwrap()
                .map(|r| r.unwrap())
                .collect()
        };
        for expected in &["events", "memories", "knowledge_edges", "namespaces", "checkpoints", "branches"] {
            assert!(tables.contains(&expected.to_string()), "missing table: {expected}");
        }
    }

    #[test]
    fn test_migrate_idempotent() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.db");
        // First open migrates
        drop(open(&path).unwrap());
        // Second open is no-op (EXCLUSIVE lock released on drop)
        open(&path).unwrap();
    }

    #[test]
    fn test_migrate_future_version() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.db");
        {
            let conn = open(&path).unwrap();
            conn.pragma_update(None, "user_version", 999u32).unwrap();
        }
        let result = open(&path);
        assert!(matches!(result, Err(MemoryError::SchemaVersionTooNew(999, 1))));
    }

    #[test]
    fn test_fts_trigger_sync() {
        let (_dir, conn) = open_temp();
        conn.execute(
            "INSERT INTO memories (id, actor_id, strategy, content) VALUES ('m1', 'a1', 'semantic', 'hello world')",
            [],
        ).unwrap();
        let count: i64 = conn.query_row(
            "SELECT count(*) FROM memory_fts WHERE memory_fts MATCH 'hello'",
            [],
            |r| r.get(0),
        ).unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_open_locked_db() {
        // EXCLUSIVE locking is per-process in SQLite. To test lock contention,
        // we spawn a child process that holds the lock, then try to open from this process.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.db");
        let _conn = open(&path).unwrap(); // create the DB first
        drop(_conn); // release lock

        // Child process: open DB and hold lock, signal readiness via a file
        let ready_path = dir.path().join("ready");
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--ignored") // won't match any test, just keeps process alive
            .env("TEST_LOCK_DB_PATH", path.to_str().unwrap())
            .env("TEST_LOCK_READY_PATH", ready_path.to_str().unwrap())
            .spawn();

        // If we can't spawn (e.g., CI sandbox), skip gracefully
        let Ok(mut child) = child else { return; };

        // The child won't actually hold the lock via test runner, so instead
        // test the error mapping directly: simulate SQLITE_BUSY on BEGIN IMMEDIATE
        child.kill().ok();
        child.wait().ok();

        // Direct unit test of the error path: open a connection with a raw SQLite lock
        use rusqlite::Connection as RawConn;
        let holder = RawConn::open(&path).unwrap();
        holder.execute_batch("PRAGMA locking_mode = EXCLUSIVE; BEGIN IMMEDIATE;").unwrap();
        // holder now has an exclusive write lock

        let result = open(&path);
        // The holder has an exclusive write lock. Our open() will fail either at
        // pragma setup (ConnectionFailed) or at BEGIN IMMEDIATE (StoreLocked),
        // depending on which statement first hits SQLITE_BUSY. Both indicate
        // the lock is preventing concurrent access.
        assert!(matches!(
            result,
            Err(MemoryError::StoreLocked(_)) | Err(MemoryError::ConnectionFailed(_))
        ));
    }

    // -- Event tests (Component 2) --

    fn conv_params<'a>(actor: &'a str, session: &'a str, content: &'a str) -> InsertEventParams<'a> {
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
    fn test_insert_and_get_event() {
        let (_dir, conn) = open_temp();
        let event = conn.insert_event(&conv_params("actor1", "sess1", "hello")).unwrap();
        assert_eq!(event.actor_id, "actor1");
        assert_eq!(event.session_id, "sess1");
        assert_eq!(event.event_type, "conversation");
        assert_eq!(event.role.as_deref(), Some("user"));
        assert_eq!(event.content.as_deref(), Some("hello"));
        assert!(event.created_at.ends_with('Z'));
        assert!(!event.id.is_empty());

        let fetched = conn.get_event("actor1", &event.id).unwrap();
        assert_eq!(fetched.id, event.id);
        assert_eq!(fetched.content, event.content);
    }

    #[test]
    fn test_get_event_not_found() {
        let (_dir, conn) = open_temp();
        let result = conn.get_event("actor1", "nonexistent");
        assert!(matches!(result, Err(MemoryError::NotFound(_))));
    }

    #[test]
    fn test_get_event_wrong_actor() {
        let (_dir, conn) = open_temp();
        let event = conn.insert_event(&conv_params("actor1", "sess1", "hello")).unwrap();
        // Different actor cannot access the event
        let result = conn.get_event("actor2", &event.id);
        assert!(matches!(result, Err(MemoryError::NotFound(_))));
    }

    #[test]
    fn test_get_events_chronological() {
        let (_dir, conn) = open_temp();
        // Insert with explicit timestamps to guarantee ordering
        for (i, ts) in ["2026-01-01T00:00:01Z", "2026-01-01T00:00:02Z", "2026-01-01T00:00:03Z"].iter().enumerate() {
            conn.execute(
                "INSERT INTO events (id, actor_id, session_id, event_type, role, content, created_at)
                 VALUES (?1, 'a1', 's1', 'conversation', 'user', ?2, ?3)",
                rusqlite::params![format!("e{i}"), format!("msg{i}"), ts],
            ).unwrap();
        }
        let params = GetEventsParams {
            actor_id: "a1",
            session_id: "s1",
            branch_id: BranchFilter::All,
            limit: 100,
            offset: 0,
            before: None,
            after: None,
        };
        let events = conn.get_events(&params).unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].content.as_deref(), Some("msg0"));
        assert_eq!(events[2].content.as_deref(), Some("msg2"));
    }

    #[test]
    fn test_get_events_branch_filter() {
        let (_dir, conn) = open_temp();
        // Main timeline event
        conn.insert_event(&conv_params("a1", "s1", "main")).unwrap();
        // Branched event
        conn.insert_event(&InsertEventParams {
            branch_id: Some("branch1"),
            ..conv_params("a1", "s1", "branched")
        }).unwrap();

        let all = conn.get_events(&GetEventsParams {
            actor_id: "a1", session_id: "s1", branch_id: BranchFilter::All,
            limit: 100, offset: 0, before: None, after: None,
        }).unwrap();
        assert_eq!(all.len(), 2);

        let main_only = conn.get_events(&GetEventsParams {
            actor_id: "a1", session_id: "s1", branch_id: BranchFilter::MainOnly,
            limit: 100, offset: 0, before: None, after: None,
        }).unwrap();
        assert_eq!(main_only.len(), 1);
        assert_eq!(main_only[0].content.as_deref(), Some("main"));

        let specific = conn.get_events(&GetEventsParams {
            actor_id: "a1", session_id: "s1", branch_id: BranchFilter::Specific("branch1"),
            limit: 100, offset: 0, before: None, after: None,
        }).unwrap();
        assert_eq!(specific.len(), 1);
        assert_eq!(specific[0].content.as_deref(), Some("branched"));
    }

    #[test]
    fn test_get_events_time_range() {
        let (_dir, conn) = open_temp();
        for (i, ts) in ["2026-01-01T00:00:01Z", "2026-01-01T00:00:02Z", "2026-01-01T00:00:03Z"].iter().enumerate() {
            conn.execute(
                "INSERT INTO events (id, actor_id, session_id, event_type, content, created_at)
                 VALUES (?1, 'a1', 's1', 'conversation', ?2, ?3)",
                rusqlite::params![format!("e{i}"), format!("msg{i}"), ts],
            ).unwrap();
        }
        let params = GetEventsParams {
            actor_id: "a1", session_id: "s1", branch_id: BranchFilter::All,
            limit: 100, offset: 0,
            before: Some("2026-01-01T00:00:03Z"),
            after: Some("2026-01-01T00:00:01Z"),
        };
        let events = conn.get_events(&params).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].content.as_deref(), Some("msg1"));
    }

    #[test]
    fn test_get_events_limit_offset() {
        let (_dir, conn) = open_temp();
        for i in 0..5 {
            conn.insert_event(&conv_params("a1", "s1", &format!("msg{i}"))).unwrap();
        }
        let params = GetEventsParams {
            actor_id: "a1", session_id: "s1", branch_id: BranchFilter::All,
            limit: 2, offset: 1, before: None, after: None,
        };
        let events = conn.get_events(&params).unwrap();
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn test_list_sessions() {
        let (_dir, conn) = open_temp();
        conn.insert_event(&conv_params("a1", "s1", "msg1")).unwrap();
        conn.insert_event(&conv_params("a1", "s1", "msg2")).unwrap();
        conn.insert_event(&conv_params("a1", "s2", "msg3")).unwrap();
        // Different actor — should not appear
        conn.insert_event(&conv_params("a2", "s3", "msg4")).unwrap();

        let sessions = conn.list_sessions("a1", 100, 0).unwrap();
        assert_eq!(sessions.len(), 2);
        // All should have actor_id = "a1"
        assert!(sessions.iter().all(|s| s.actor_id == "a1"));
        // s1 has 2 events
        let s1 = sessions.iter().find(|s| s.session_id == "s1").unwrap();
        assert_eq!(s1.event_count, 2);
    }

    #[test]
    fn test_delete_expired() {
        let (_dir, conn) = open_temp();
        // Expired event (past timestamp)
        conn.insert_event(&InsertEventParams {
            expires_at: Some("2020-01-01T00:00:00Z"),
            ..conv_params("a1", "s1", "expired")
        }).unwrap();
        // Non-expired event
        conn.insert_event(&conv_params("a1", "s1", "keep")).unwrap();

        let deleted = conn.delete_expired_events().unwrap();
        assert_eq!(deleted, 1);

        let remaining = conn.get_events(&GetEventsParams {
            actor_id: "a1", session_id: "s1", branch_id: BranchFilter::All,
            limit: 100, offset: 0, before: None, after: None,
        }).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].content.as_deref(), Some("keep"));
    }

    #[test]
    fn test_insert_event_blob() {
        let (_dir, conn) = open_temp();
        let blob_data = vec![0u8, 1, 2, 3, 255];
        let event = conn.insert_event(&InsertEventParams {
            actor_id: "a1",
            session_id: "s1",
            event_type: "blob",
            role: None,
            content: None,
            blob_data: Some(&blob_data),
            metadata: None,
            branch_id: None,
            expires_at: None,
        }).unwrap();
        assert_eq!(event.event_type, "blob");
        assert_eq!(event.blob_data.as_deref(), Some(blob_data.as_slice()));

        let fetched = conn.get_event("a1", &event.id).unwrap();
        assert_eq!(fetched.blob_data.as_deref(), Some(blob_data.as_slice()));
    }

    // -- Memory tests (Component 3) --

    use crate::memories::{InsertMemoryParams, ListMemoriesParams, ConsolidateAction};

    fn mem_params<'a>(actor: &'a str, content: &'a str, strategy: &'a str) -> InsertMemoryParams<'a> {
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
    fn test_insert_and_get_memory() {
        let (_dir, conn) = open_temp();
        let memory = conn.insert_memory(&mem_params("actor1", "Rust is great", "semantic")).unwrap();
        assert_eq!(memory.actor_id, "actor1");
        assert_eq!(memory.content, "Rust is great");
        assert_eq!(memory.strategy, "semantic");
        assert_eq!(memory.namespace, "default");
        assert!(memory.is_valid);
        assert!(memory.superseded_by.is_none());
        assert!(memory.created_at.ends_with('Z'));
        assert!(!memory.id.is_empty());

        let fetched = conn.get_memory("actor1", &memory.id).unwrap();
        assert_eq!(fetched.id, memory.id);
        assert_eq!(fetched.content, memory.content);
    }

    #[test]
    fn test_insert_memory_with_embedding() {
        let (_dir, conn) = open_temp();
        let embedding = vec![0.1f32; 384];
        let memory = conn.insert_memory(&InsertMemoryParams {
            embedding: Some(&embedding),
            ..mem_params("actor1", "with vector", "semantic")
        }).unwrap();

        // Verify embedding exists in memory_vec
        let count: i64 = conn.query_row(
            "SELECT count(*) FROM memory_vec WHERE memory_id = ?1",
            [&memory.id],
            |r| r.get(0),
        ).unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_insert_memory_default_namespace() {
        let (_dir, conn) = open_temp();
        let memory = conn.insert_memory(&mem_params("a1", "content", "semantic")).unwrap();
        assert_eq!(memory.namespace, "default");

        let memory2 = conn.insert_memory(&InsertMemoryParams {
            namespace: Some("/custom/ns"),
            ..mem_params("a1", "content2", "semantic")
        }).unwrap();
        assert_eq!(memory2.namespace, "/custom/ns");
    }

    #[test]
    fn test_get_memory_not_found() {
        let (_dir, conn) = open_temp();
        let result = conn.get_memory("actor1", "nonexistent");
        assert!(matches!(result, Err(MemoryError::NotFound(_))));
    }

    #[test]
    fn test_get_memory_wrong_actor() {
        let (_dir, conn) = open_temp();
        let memory = conn.insert_memory(&mem_params("actor1", "content", "semantic")).unwrap();
        let result = conn.get_memory("actor2", &memory.id);
        assert!(matches!(result, Err(MemoryError::NotFound(_))));
    }

    #[test]
    fn test_list_memories_by_actor() {
        let (_dir, conn) = open_temp();
        conn.insert_memory(&mem_params("a1", "m1", "semantic")).unwrap();
        conn.insert_memory(&mem_params("a1", "m2", "semantic")).unwrap();
        conn.insert_memory(&mem_params("a2", "m3", "semantic")).unwrap();

        let results = conn.list_memories(&ListMemoriesParams {
            actor_id: "a1", namespace: None, namespace_prefix: None,
            strategy: None, valid_only: false, limit: 100, offset: 0,
        }).unwrap();
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|m| m.actor_id == "a1"));
    }

    #[test]
    fn test_list_memories_by_namespace() {
        let (_dir, conn) = open_temp();
        conn.insert_memory(&InsertMemoryParams {
            namespace: Some("/prefs"),
            ..mem_params("a1", "m1", "semantic")
        }).unwrap();
        conn.insert_memory(&InsertMemoryParams {
            namespace: Some("/facts"),
            ..mem_params("a1", "m2", "semantic")
        }).unwrap();

        let results = conn.list_memories(&ListMemoriesParams {
            actor_id: "a1", namespace: Some("/prefs"), namespace_prefix: None,
            strategy: None, valid_only: false, limit: 100, offset: 0,
        }).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].namespace, "/prefs");
    }

    #[test]
    fn test_list_memories_by_namespace_prefix() {
        let (_dir, conn) = open_temp();
        conn.insert_memory(&InsertMemoryParams {
            namespace: Some("/user/prefs"),
            ..mem_params("a1", "m1", "semantic")
        }).unwrap();
        conn.insert_memory(&InsertMemoryParams {
            namespace: Some("/user/facts"),
            ..mem_params("a1", "m2", "semantic")
        }).unwrap();
        conn.insert_memory(&InsertMemoryParams {
            namespace: Some("/system"),
            ..mem_params("a1", "m3", "semantic")
        }).unwrap();

        let results = conn.list_memories(&ListMemoriesParams {
            actor_id: "a1", namespace: None, namespace_prefix: Some("/user"),
            strategy: None, valid_only: false, limit: 100, offset: 0,
        }).unwrap();
        assert_eq!(results.len(), 2);

        // Test LIKE escaping: underscore in prefix should be literal
        conn.insert_memory(&InsertMemoryParams {
            namespace: Some("/user_data/x"),
            ..mem_params("a1", "m4", "semantic")
        }).unwrap();
        conn.insert_memory(&InsertMemoryParams {
            namespace: Some("/userXdata/x"),
            ..mem_params("a1", "m5", "semantic")
        }).unwrap();

        let results = conn.list_memories(&ListMemoriesParams {
            actor_id: "a1", namespace: None, namespace_prefix: Some("/user_data"),
            strategy: None, valid_only: false, limit: 100, offset: 0,
        }).unwrap();
        // Should match "/user_data/x" but NOT "/userXdata/x"
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].namespace, "/user_data/x");
    }

    #[test]
    fn test_list_memories_by_strategy() {
        let (_dir, conn) = open_temp();
        conn.insert_memory(&mem_params("a1", "m1", "semantic")).unwrap();
        conn.insert_memory(&mem_params("a1", "m2", "summary")).unwrap();

        let results = conn.list_memories(&ListMemoriesParams {
            actor_id: "a1", namespace: None, namespace_prefix: None,
            strategy: Some("summary"), valid_only: false, limit: 100, offset: 0,
        }).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].strategy, "summary");
    }

    #[test]
    fn test_list_memories_valid_only() {
        let (_dir, conn) = open_temp();
        let m = conn.insert_memory(&mem_params("a1", "old", "semantic")).unwrap();
        conn.consolidate_memory("a1", &m.id, &ConsolidateAction::Invalidate).unwrap();
        conn.insert_memory(&mem_params("a1", "valid", "semantic")).unwrap();

        let valid = conn.list_memories(&ListMemoriesParams {
            actor_id: "a1", namespace: None, namespace_prefix: None,
            strategy: None, valid_only: true, limit: 100, offset: 0,
        }).unwrap();
        assert_eq!(valid.len(), 1);
        assert_eq!(valid[0].content, "valid");

        let all = conn.list_memories(&ListMemoriesParams {
            actor_id: "a1", namespace: None, namespace_prefix: None,
            strategy: None, valid_only: false, limit: 100, offset: 0,
        }).unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn test_list_memories_pagination() {
        let (_dir, conn) = open_temp();
        for i in 0..5 {
            conn.insert_memory(&mem_params("a1", &format!("m{i}"), "semantic")).unwrap();
        }
        let results = conn.list_memories(&ListMemoriesParams {
            actor_id: "a1", namespace: None, namespace_prefix: None,
            strategy: None, valid_only: false, limit: 2, offset: 1,
        }).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_consolidate_update() {
        let (_dir, conn) = open_temp();
        let old = conn.insert_memory(&mem_params("a1", "old content", "semantic")).unwrap();

        let new = conn.consolidate_memory(
            "a1", &old.id,
            &ConsolidateAction::Update { content: "new content", embedding: None },
        ).unwrap();

        assert_ne!(new.id, old.id);
        assert_eq!(new.content, "new content");
        assert_eq!(new.namespace, old.namespace);
        assert_eq!(new.strategy, old.strategy);
        assert!(new.is_valid);

        // Old memory should be invalid with superseded_by
        let old_fetched = conn.get_memory("a1", &old.id).unwrap();
        assert!(!old_fetched.is_valid);
        assert_eq!(old_fetched.superseded_by.as_deref(), Some(new.id.as_str()));
    }

    #[test]
    fn test_consolidate_invalidate() {
        let (_dir, conn) = open_temp();
        let m = conn.insert_memory(&mem_params("a1", "content", "semantic")).unwrap();

        let invalidated = conn.consolidate_memory(
            "a1", &m.id, &ConsolidateAction::Invalidate,
        ).unwrap();

        assert!(!invalidated.is_valid);
        assert_eq!(invalidated.id, m.id);
    }

    #[test]
    fn test_consolidate_already_invalid() {
        let (_dir, conn) = open_temp();
        let m = conn.insert_memory(&mem_params("a1", "content", "semantic")).unwrap();
        conn.consolidate_memory("a1", &m.id, &ConsolidateAction::Invalidate).unwrap();

        // Second invalidation should fail
        let result = conn.consolidate_memory("a1", &m.id, &ConsolidateAction::Invalidate);
        assert!(matches!(result, Err(MemoryError::NotFound(_))));
    }

    #[test]
    fn test_delete_memory() {
        let (_dir, conn) = open_temp();
        let embedding = vec![0.1f32; 384];
        let m = conn.insert_memory(&InsertMemoryParams {
            embedding: Some(&embedding),
            ..mem_params("a1", "to delete", "semantic")
        }).unwrap();

        conn.delete_memory("a1", &m.id).unwrap();

        // Memory should be gone
        assert!(matches!(conn.get_memory("a1", &m.id), Err(MemoryError::NotFound(_))));

        // Embedding should be gone
        let count: i64 = conn.query_row(
            "SELECT count(*) FROM memory_vec WHERE memory_id = ?1",
            [&m.id],
            |r| r.get(0),
        ).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_delete_memory_not_found() {
        let (_dir, conn) = open_temp();
        let result = conn.delete_memory("a1", "nonexistent");
        assert!(matches!(result, Err(MemoryError::NotFound(_))));
    }

    #[test]
    fn test_delete_memory_wrong_actor() {
        let (_dir, conn) = open_temp();
        let m = conn.insert_memory(&mem_params("actor1", "content", "semantic")).unwrap();
        let result = conn.delete_memory("actor2", &m.id);
        assert!(matches!(result, Err(MemoryError::NotFound(_))));
    }

    // -- Search tests (Component 4) --

    use crate::search::{SearchFtsParams, SearchVectorParams};

    #[test]
    fn test_search_fts_basic() {
        let (_dir, conn) = open_temp();
        conn.insert_memory(&mem_params("a1", "Rust programming language", "semantic")).unwrap();
        conn.insert_memory(&mem_params("a1", "Python scripting", "semantic")).unwrap();
        conn.insert_memory(&mem_params("a1", "Go concurrency", "semantic")).unwrap();

        let results = conn.search_fts(&SearchFtsParams {
            actor_id: "a1",
            fts_query: "\"Rust\"",
            namespace: None,
            namespace_prefix: None,
            strategy: None,
            limit: 10,
        }).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].0.content.contains("Rust"));
        assert!(results[0].1 > 0.0);
    }

    #[test]
    fn test_search_fts_actor_scoping() {
        let (_dir, conn) = open_temp();
        conn.insert_memory(&mem_params("a1", "Rust programming", "semantic")).unwrap();
        conn.insert_memory(&mem_params("a2", "Rust scripting", "semantic")).unwrap();

        let results = conn.search_fts(&SearchFtsParams {
            actor_id: "a1",
            fts_query: "\"Rust\"",
            namespace: None,
            namespace_prefix: None,
            strategy: None,
            limit: 10,
        }).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0.actor_id, "a1");
    }

    #[test]
    fn test_search_fts_valid_only() {
        let (_dir, conn) = open_temp();
        let m = conn.insert_memory(&mem_params("a1", "Rust programming", "semantic")).unwrap();
        conn.consolidate_memory("a1", &m.id, &ConsolidateAction::Invalidate).unwrap();

        let results = conn.search_fts(&SearchFtsParams {
            actor_id: "a1",
            fts_query: "\"Rust\"",
            namespace: None,
            namespace_prefix: None,
            strategy: None,
            limit: 10,
        }).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_fts_namespace_filter() {
        let (_dir, conn) = open_temp();
        conn.insert_memory(&InsertMemoryParams {
            namespace: Some("/lang"),
            ..mem_params("a1", "Rust programming", "semantic")
        }).unwrap();
        conn.insert_memory(&InsertMemoryParams {
            namespace: Some("/other"),
            ..mem_params("a1", "Rust scripting", "semantic")
        }).unwrap();

        let results = conn.search_fts(&SearchFtsParams {
            actor_id: "a1",
            fts_query: "\"Rust\"",
            namespace: Some("/lang"),
            namespace_prefix: None,
            strategy: None,
            limit: 10,
        }).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0.namespace, "/lang");
    }

    #[test]
    fn test_search_fts_no_match() {
        let (_dir, conn) = open_temp();
        conn.insert_memory(&mem_params("a1", "Python scripting", "semantic")).unwrap();

        let results = conn.search_fts(&SearchFtsParams {
            actor_id: "a1",
            fts_query: "\"Rust\"",
            namespace: None,
            namespace_prefix: None,
            strategy: None,
            limit: 10,
        }).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_vector_basic() {
        let (_dir, conn) = open_temp();
        let mut emb1 = vec![0.0f32; 384];
        emb1[0] = 1.0;
        let mut emb2 = vec![0.0f32; 384];
        emb2[1] = 1.0;

        conn.insert_memory(&InsertMemoryParams {
            embedding: Some(&emb1),
            ..mem_params("a1", "memory one", "semantic")
        }).unwrap();
        conn.insert_memory(&InsertMemoryParams {
            embedding: Some(&emb2),
            ..mem_params("a1", "memory two", "semantic")
        }).unwrap();

        let results = conn.search_vector(&SearchVectorParams {
            actor_id: "a1",
            embedding: &emb1,
            namespace: None,
            namespace_prefix: None,
            strategy: None,
            limit: 10,
        }).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0.content, "memory one");
        assert!(results[0].1 < results[1].1);
    }

    #[test]
    fn test_search_vector_actor_scoping() {
        let (_dir, conn) = open_temp();
        let emb = vec![0.1f32; 384];
        conn.insert_memory(&InsertMemoryParams {
            embedding: Some(&emb),
            ..mem_params("a1", "actor1 memory", "semantic")
        }).unwrap();
        conn.insert_memory(&InsertMemoryParams {
            embedding: Some(&emb),
            ..mem_params("a2", "actor2 memory", "semantic")
        }).unwrap();

        let results = conn.search_vector(&SearchVectorParams {
            actor_id: "a1",
            embedding: &emb,
            namespace: None,
            namespace_prefix: None,
            strategy: None,
            limit: 10,
        }).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0.actor_id, "a1");
    }

    #[test]
    fn test_search_vector_no_embeddings() {
        let (_dir, conn) = open_temp();
        conn.insert_memory(&mem_params("a1", "no embedding", "semantic")).unwrap();
        let emb = vec![0.1f32; 384];

        let results = conn.search_vector(&SearchVectorParams {
            actor_id: "a1",
            embedding: &emb,
            namespace: None,
            namespace_prefix: None,
            strategy: None,
            limit: 10,
        }).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_vector_knn_roundtrip() {
        let (_dir, conn) = open_temp();
        let mut emb = vec![0.0f32; 384];
        emb[0] = 1.0;

        let m = conn.insert_memory(&InsertMemoryParams {
            embedding: Some(&emb),
            ..mem_params("a1", "roundtrip memory", "semantic")
        }).unwrap();

        let results = conn.search_vector(&SearchVectorParams {
            actor_id: "a1",
            embedding: &emb,
            namespace: None,
            namespace_prefix: None,
            strategy: None,
            limit: 1,
        }).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0.id, m.id);
        assert!(results[0].1 < 1e-5);
    }

    // -- Graph tests (Component 5) --

    use crate::graph::{Direction, InsertEdgeParams, UpdateEdgeParams};

    fn make_memory(conn: &Connection, actor: &str, content: &str) -> crate::memories::Memory {
        conn.insert_memory(&InsertMemoryParams {
            actor_id: actor,
            content,
            strategy: "semantic",
            namespace: None,
            metadata: None,
            source_session_id: None,
            embedding: None,
        }).unwrap()
    }

    #[test]
    fn test_insert_and_get_edge() {
        let (_dir, conn) = open_temp();
        let m1 = make_memory(&conn, "a1", "from");
        let m2 = make_memory(&conn, "a1", "to");
        let edge = conn.insert_edge(&InsertEdgeParams {
            actor_id: "a1",
            from_memory_id: &m1.id,
            to_memory_id: &m2.id,
            label: "depends_on",
            properties: Some(r#"{"weight": 1}"#),
        }).unwrap();
        assert_eq!(edge.from_memory_id, m1.id);
        assert_eq!(edge.to_memory_id, m2.id);
        assert_eq!(edge.label, "depends_on");
        assert_eq!(edge.properties.as_deref(), Some(r#"{"weight": 1}"#));
        assert!(!edge.id.is_empty());

        let fetched = conn.get_edge("a1", &edge.id).unwrap();
        assert_eq!(fetched.id, edge.id);
        assert_eq!(fetched.label, "depends_on");
    }

    #[test]
    fn test_insert_edge_missing_memory() {
        let (_dir, conn) = open_temp();
        let m1 = make_memory(&conn, "a1", "from");
        let result = conn.insert_edge(&InsertEdgeParams {
            actor_id: "a1",
            from_memory_id: &m1.id,
            to_memory_id: "nonexistent",
            label: "uses",
            properties: None,
        });
        assert!(matches!(result, Err(MemoryError::NotFound(_))));
    }

    #[test]
    fn test_insert_edge_wrong_actor() {
        let (_dir, conn) = open_temp();
        let m1 = make_memory(&conn, "a1", "from");
        let m2 = make_memory(&conn, "a2", "to");
        let result = conn.insert_edge(&InsertEdgeParams {
            actor_id: "a1",
            from_memory_id: &m1.id,
            to_memory_id: &m2.id,
            label: "uses",
            properties: None,
        });
        assert!(matches!(result, Err(MemoryError::NotFound(_))));
    }

    #[test]
    fn test_get_neighbors_out() {
        let (_dir, conn) = open_temp();
        let m1 = make_memory(&conn, "a1", "center");
        let m2 = make_memory(&conn, "a1", "out1");
        let m3 = make_memory(&conn, "a1", "out2");
        conn.insert_edge(&InsertEdgeParams {
            actor_id: "a1", from_memory_id: &m1.id, to_memory_id: &m2.id,
            label: "uses", properties: None,
        }).unwrap();
        conn.insert_edge(&InsertEdgeParams {
            actor_id: "a1", from_memory_id: &m1.id, to_memory_id: &m3.id,
            label: "uses", properties: None,
        }).unwrap();
        // Incoming edge should not appear in Out
        conn.insert_edge(&InsertEdgeParams {
            actor_id: "a1", from_memory_id: &m2.id, to_memory_id: &m1.id,
            label: "uses", properties: None,
        }).unwrap();

        let neighbors = conn.get_neighbors("a1", &m1.id, Direction::Out, None, 100).unwrap();
        assert_eq!(neighbors.len(), 2);
        let ids: Vec<&str> = neighbors.iter().map(|n| n.memory.id.as_str()).collect();
        assert!(ids.contains(&m2.id.as_str()));
        assert!(ids.contains(&m3.id.as_str()));
    }

    #[test]
    fn test_get_neighbors_in() {
        let (_dir, conn) = open_temp();
        let m1 = make_memory(&conn, "a1", "target");
        let m2 = make_memory(&conn, "a1", "source");
        conn.insert_edge(&InsertEdgeParams {
            actor_id: "a1", from_memory_id: &m2.id, to_memory_id: &m1.id,
            label: "uses", properties: None,
        }).unwrap();

        let neighbors = conn.get_neighbors("a1", &m1.id, Direction::In, None, 100).unwrap();
        assert_eq!(neighbors.len(), 1);
        assert_eq!(neighbors[0].memory.id, m2.id);
    }

    #[test]
    fn test_get_neighbors_both() {
        let (_dir, conn) = open_temp();
        let m1 = make_memory(&conn, "a1", "center");
        let m2 = make_memory(&conn, "a1", "out");
        let m3 = make_memory(&conn, "a1", "in");
        conn.insert_edge(&InsertEdgeParams {
            actor_id: "a1", from_memory_id: &m1.id, to_memory_id: &m2.id,
            label: "uses", properties: None,
        }).unwrap();
        conn.insert_edge(&InsertEdgeParams {
            actor_id: "a1", from_memory_id: &m3.id, to_memory_id: &m1.id,
            label: "uses", properties: None,
        }).unwrap();

        let neighbors = conn.get_neighbors("a1", &m1.id, Direction::Both, None, 100).unwrap();
        assert_eq!(neighbors.len(), 2);
        let ids: Vec<&str> = neighbors.iter().map(|n| n.memory.id.as_str()).collect();
        assert!(ids.contains(&m2.id.as_str()));
        assert!(ids.contains(&m3.id.as_str()));
    }

    #[test]
    fn test_get_neighbors_label_filter() {
        let (_dir, conn) = open_temp();
        let m1 = make_memory(&conn, "a1", "center");
        let m2 = make_memory(&conn, "a1", "dep");
        let m3 = make_memory(&conn, "a1", "rel");
        conn.insert_edge(&InsertEdgeParams {
            actor_id: "a1", from_memory_id: &m1.id, to_memory_id: &m2.id,
            label: "depends_on", properties: None,
        }).unwrap();
        conn.insert_edge(&InsertEdgeParams {
            actor_id: "a1", from_memory_id: &m1.id, to_memory_id: &m3.id,
            label: "related_to", properties: None,
        }).unwrap();

        let neighbors = conn.get_neighbors("a1", &m1.id, Direction::Out, Some("depends_on"), 100).unwrap();
        assert_eq!(neighbors.len(), 1);
        assert_eq!(neighbors[0].memory.id, m2.id);
    }

    #[test]
    fn test_traverse_basic() {
        let (_dir, conn) = open_temp();
        let a = make_memory(&conn, "a1", "A");
        let b = make_memory(&conn, "a1", "B");
        let c = make_memory(&conn, "a1", "C");
        conn.insert_edge(&InsertEdgeParams {
            actor_id: "a1", from_memory_id: &a.id, to_memory_id: &b.id,
            label: "uses", properties: None,
        }).unwrap();
        conn.insert_edge(&InsertEdgeParams {
            actor_id: "a1", from_memory_id: &b.id, to_memory_id: &c.id,
            label: "uses", properties: None,
        }).unwrap();

        let nodes = conn.traverse("a1", &a.id, 2, None, Direction::Out).unwrap();
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0].depth, 1);
        assert_eq!(nodes[0].memory.id, b.id);
        assert_eq!(nodes[1].depth, 2);
        assert_eq!(nodes[1].memory.id, c.id);
        assert_eq!(nodes[1].path.len(), 2);
    }

    #[test]
    fn test_traverse_cycle_detection() {
        let (_dir, conn) = open_temp();
        let a = make_memory(&conn, "a1", "A");
        let b = make_memory(&conn, "a1", "B");
        conn.insert_edge(&InsertEdgeParams {
            actor_id: "a1", from_memory_id: &a.id, to_memory_id: &b.id,
            label: "uses", properties: None,
        }).unwrap();
        conn.insert_edge(&InsertEdgeParams {
            actor_id: "a1", from_memory_id: &b.id, to_memory_id: &a.id,
            label: "uses", properties: None,
        }).unwrap();

        let nodes = conn.traverse("a1", &a.id, 5, None, Direction::Out).unwrap();
        // Should visit B once, then stop (A already visited)
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].memory.id, b.id);
    }

    #[test]
    fn test_traverse_max_depth() {
        let (_dir, conn) = open_temp();
        let a = make_memory(&conn, "a1", "A");
        let b = make_memory(&conn, "a1", "B");
        let c = make_memory(&conn, "a1", "C");
        conn.insert_edge(&InsertEdgeParams {
            actor_id: "a1", from_memory_id: &a.id, to_memory_id: &b.id,
            label: "uses", properties: None,
        }).unwrap();
        conn.insert_edge(&InsertEdgeParams {
            actor_id: "a1", from_memory_id: &b.id, to_memory_id: &c.id,
            label: "uses", properties: None,
        }).unwrap();

        let nodes = conn.traverse("a1", &a.id, 1, None, Direction::Out).unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].memory.id, b.id);
    }

    #[test]
    fn test_traverse_direction() {
        let (_dir, conn) = open_temp();
        let a = make_memory(&conn, "a1", "A");
        let b = make_memory(&conn, "a1", "B");
        conn.insert_edge(&InsertEdgeParams {
            actor_id: "a1", from_memory_id: &a.id, to_memory_id: &b.id,
            label: "uses", properties: None,
        }).unwrap();

        // In direction from B should find A
        let nodes = conn.traverse("a1", &b.id, 2, None, Direction::In).unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].memory.id, a.id);

        // Both direction from A should find B
        let nodes = conn.traverse("a1", &a.id, 2, None, Direction::Both).unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].memory.id, b.id);
    }

    #[test]
    fn test_traverse_nonexistent_start() {
        let (_dir, conn) = open_temp();
        let result = conn.traverse("a1", "nonexistent", 2, None, Direction::Out);
        assert!(matches!(result, Err(MemoryError::NotFound(_))));
    }

    #[test]
    fn test_traverse_label_filter() {
        let (_dir, conn) = open_temp();
        let a = make_memory(&conn, "a1", "A");
        let b = make_memory(&conn, "a1", "B");
        let c = make_memory(&conn, "a1", "C");
        conn.insert_edge(&InsertEdgeParams {
            actor_id: "a1", from_memory_id: &a.id, to_memory_id: &b.id,
            label: "uses", properties: None,
        }).unwrap();
        conn.insert_edge(&InsertEdgeParams {
            actor_id: "a1", from_memory_id: &a.id, to_memory_id: &c.id,
            label: "related_to", properties: None,
        }).unwrap();

        let nodes = conn.traverse("a1", &a.id, 2, Some("uses"), Direction::Out).unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].memory.id, b.id);
    }

    #[test]
    fn test_update_edge_label() {
        let (_dir, conn) = open_temp();
        let m1 = make_memory(&conn, "a1", "from");
        let m2 = make_memory(&conn, "a1", "to");
        let edge = conn.insert_edge(&InsertEdgeParams {
            actor_id: "a1", from_memory_id: &m1.id, to_memory_id: &m2.id,
            label: "old", properties: None,
        }).unwrap();

        let updated = conn.update_edge(&UpdateEdgeParams {
            actor_id: "a1", edge_id: &edge.id,
            label: Some("new"), properties: None,
        }).unwrap();
        assert_eq!(updated.label, "new");
        assert_eq!(updated.properties, None);
    }

    #[test]
    fn test_update_edge_properties() {
        let (_dir, conn) = open_temp();
        let m1 = make_memory(&conn, "a1", "from");
        let m2 = make_memory(&conn, "a1", "to");
        let edge = conn.insert_edge(&InsertEdgeParams {
            actor_id: "a1", from_memory_id: &m1.id, to_memory_id: &m2.id,
            label: "uses", properties: None,
        }).unwrap();

        let updated = conn.update_edge(&UpdateEdgeParams {
            actor_id: "a1", edge_id: &edge.id,
            label: None, properties: Some(r#"{"k":"v"}"#),
        }).unwrap();
        assert_eq!(updated.label, "uses");
        assert_eq!(updated.properties.as_deref(), Some(r#"{"k":"v"}"#));
    }

    #[test]
    fn test_delete_edge() {
        let (_dir, conn) = open_temp();
        let m1 = make_memory(&conn, "a1", "from");
        let m2 = make_memory(&conn, "a1", "to");
        let edge = conn.insert_edge(&InsertEdgeParams {
            actor_id: "a1", from_memory_id: &m1.id, to_memory_id: &m2.id,
            label: "uses", properties: None,
        }).unwrap();

        conn.delete_edge("a1", &edge.id).unwrap();
        assert!(matches!(conn.get_edge("a1", &edge.id), Err(MemoryError::NotFound(_))));
    }

    #[test]
    fn test_delete_edge_not_found() {
        let (_dir, conn) = open_temp();
        let result = conn.delete_edge("a1", "nonexistent");
        assert!(matches!(result, Err(MemoryError::NotFound(_))));
    }

    #[test]
    fn test_cascade_delete_memory() {
        let (_dir, conn) = open_temp();
        let m1 = make_memory(&conn, "a1", "from");
        let m2 = make_memory(&conn, "a1", "to");
        conn.insert_edge(&InsertEdgeParams {
            actor_id: "a1", from_memory_id: &m1.id, to_memory_id: &m2.id,
            label: "uses", properties: None,
        }).unwrap();

        conn.delete_memory("a1", &m1.id).unwrap();

        // Edge should be gone due to CASCADE
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM knowledge_edges", [], |r| r.get(0),
        ).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_list_edge_labels() {
        let (_dir, conn) = open_temp();
        let m1 = make_memory(&conn, "a1", "m1");
        let m2 = make_memory(&conn, "a1", "m2");
        conn.insert_edge(&InsertEdgeParams {
            actor_id: "a1", from_memory_id: &m1.id, to_memory_id: &m2.id,
            label: "uses", properties: None,
        }).unwrap();
        conn.insert_edge(&InsertEdgeParams {
            actor_id: "a1", from_memory_id: &m1.id, to_memory_id: &m2.id,
            label: "uses", properties: None,
        }).unwrap();
        conn.insert_edge(&InsertEdgeParams {
            actor_id: "a1", from_memory_id: &m1.id, to_memory_id: &m2.id,
            label: "related_to", properties: None,
        }).unwrap();

        let labels = conn.list_edge_labels("a1").unwrap();
        assert_eq!(labels.len(), 2);
        assert_eq!(labels[0].label, "uses");
        assert_eq!(labels[0].count, 2);
        assert_eq!(labels[1].label, "related_to");
        assert_eq!(labels[1].count, 1);
    }

    #[test]
    fn test_graph_stats() {
        let (_dir, conn) = open_temp();
        let m1 = make_memory(&conn, "a1", "m1");
        let m2 = make_memory(&conn, "a1", "m2");
        conn.insert_edge(&InsertEdgeParams {
            actor_id: "a1", from_memory_id: &m1.id, to_memory_id: &m2.id,
            label: "uses", properties: None,
        }).unwrap();

        let stats = conn.graph_stats("a1").unwrap();
        assert_eq!(stats.total_edges, 1);
        assert_eq!(stats.labels.len(), 1);
        assert!(!stats.most_connected.is_empty());
    }
}
