use rusqlite::Connection;
use std::path::Path;
use std::sync::OnceLock;
use crate::error::MemoryError;

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
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            expires_at TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_events_session ON events(actor_id, session_id, created_at);
        CREATE INDEX IF NOT EXISTS idx_events_branch ON events(session_id, branch_id, created_at);
        CREATE INDEX IF NOT EXISTS idx_events_actor ON events(actor_id, created_at);

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
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE INDEX IF NOT EXISTS idx_memories_actor ON memories(actor_id, namespace, is_valid);
        CREATE INDEX IF NOT EXISTS idx_memories_strategy ON memories(strategy, is_valid);

        CREATE TABLE IF NOT EXISTS knowledge_edges (
            id TEXT PRIMARY KEY,
            from_memory_id TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
            to_memory_id TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
            label TEXT NOT NULL,
            properties TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
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
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS checkpoints (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            actor_id TEXT NOT NULL,
            name TEXT NOT NULL,
            event_id TEXT NOT NULL,
            metadata TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_checkpoint_name ON checkpoints(session_id, name);

        CREATE TABLE IF NOT EXISTS branches (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            name TEXT,
            parent_branch_id TEXT,
            root_event_id TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
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
    // fn insert_event(...) -> Result<String, MemoryError>;
    // fn get_event(event_id: &str) -> Result<Event, MemoryError>;
    // fn get_events(...) -> Result<Vec<Event>, MemoryError>;
    // fn list_sessions(actor_id: &str, limit: u32, offset: u32) -> Result<Vec<SessionInfo>, MemoryError>;
    // fn delete_expired_events() -> Result<u64, MemoryError>;

    // -- Memories (Component 3) --
    // fn insert_memory(...) -> Result<String, MemoryError>;
    // fn get_memory(memory_id: &str) -> Result<Memory, MemoryError>;
    // fn list_memories(...) -> Result<Vec<Memory>, MemoryError>;
    // fn consolidate_memory(...) -> Result<(), MemoryError>; // atomic: invalidate old + insert new
    // fn delete_memory(memory_id: &str) -> Result<(), MemoryError>;

    // -- Search (Component 4) --
    // fn search_fts(actor_id: &str, query: &str, limit: u32) -> Result<Vec<Memory>, MemoryError>;
    // fn search_vector(actor_id: &str, embedding: &[f32], limit: u32) -> Result<Vec<Memory>, MemoryError>;

    // -- Graph (Component 5) --
    // fn insert_edge(...) -> Result<String, MemoryError>;
    // fn get_neighbors(memory_id: &str, direction: Direction, label: Option<&str>, limit: u32) -> Result<Vec<Neighbor>, MemoryError>;
    // fn traverse(start_memory_id: &str, max_depth: u32, label: Option<&str>, direction: Direction) -> Result<Vec<Memory>, MemoryError>;
    // fn delete_edge(edge_id: &str) -> Result<(), MemoryError>;

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
}
