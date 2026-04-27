# Component 1: Core DB Layer — Detailed Design

## Scope

Two files: `db.rs` (SQLite connection + schema migration) and `store.rs` (multi-store lifecycle). Together they provide the foundation every other component builds on.

### Review Resolution Log

Changes from design review round 1 (all Critical/High findings addressed):

| # | Finding | Resolution |
|---|---------|------------|
| C1 | Missing sqlite-vec in Cargo.toml | Added to Cargo.toml with exact pin. API uses `sqlite3_vec_init` + `register_auto_extension`. |
| C2 | Migration not transactional | Use explicit `conn.transaction()` per migration step |
| C3 | WAL checkpoint on close/switch | Added `close_active()` with `PRAGMA wal_checkpoint(TRUNCATE)` + `PRAGMA optimize` |
| C4 | FTS5 query injection | Documented as search.rs responsibility; added note in edge cases |
| C5 | Symlink/TOCTOU race in store ops | Added `canonicalize` + `symlink_metadata` checks before open/delete |
| C6 | LOCAL_MEMORY_HOME not validated | Validate absolute path, reject `..`, `~`, known-bad prefixes |
| H1 | StoreManager not Send | Wrap Connection in `std::sync::Mutex`, use `spawn_blocking` pattern. Document concurrency model. |
| H2 | No concurrency strategy | Added §2.1 Concurrency Model section |
| H3 | Schema version per-step | Update `schema_version` after each migration step |
| H4 | FTS5 rowid instability after VACUUM | Add explicit `memory_rowid INTEGER PRIMARY KEY` to memories table |
| H5 | MCP tool dotted names | Verified: rmcp `#[tool(name = "memory.add_event")]` works. Documented. |
| H6 | No integrity check on open | Added `PRAGMA quick_check` on first open |
| H7 | No file locking | Use `PRAGMA locking_mode = EXCLUSIVE` |
| H8 | PRAGMA synchronous=NORMAL risk | Changed default to FULL |
| H9 | WAL growth unbounded | Added `PRAGMA journal_size_limit = 67108864` (64MB) |
| H10 | Store deletion race with WAL | Checkpoint before close; delete in reverse order (shm, wal, db) |
| H11 | File permissions not set | Set 0o700 on base_dir, 0o600 on db files (Unix) |
| H12 | Error messages leak paths/SQL | Two-tier errors: internal (tracing) vs external (sanitized for MCP) |
| H13 | Inconsistent version pinning + no Cargo.lock | Pin critical deps exactly; commit Cargo.lock |
| H14 | No CI | Deferred to pre-coding checklist (minimal GH Actions workflow) |
| H15 | Windows reserved device names | Added to validate_name() |
| H16 | WAL file cleanup on Windows | Retry logic with short delay on delete |

Changes from design review round 2:

| # | Finding | Resolution |
|---|---------|------------|
| R2-1 | DESIGN.md schema divergence | Updated DESIGN.md: memories table, FTS5, memory_vec, removed _meta table |
| R2-2 | register_auto_extension missing Once guard | Replaced with OnceLock pattern + safety docs in code sample |
| R2-3 | Mutex poisoning (.unwrap()) | Changed to `.unwrap_or_else(\|e\| e.into_inner())` with rationale |
| R2-4 | From<rusqlite::Error> blanket impl | Removed. Use explicit .map_err() with tracing + generic messages |
| R2-5 | close_active() swallows errors on switch | Split into close_active() (returns Result) and close_active_best_effort() (for Drop) |
| R2-6 | quick_check blocks on large DBs | Only run on pre-existing DBs (user_version > 0), skip for new |
| R2-7 | EXCLUSIVE locking not acquired until I/O | Added `BEGIN IMMEDIATE; COMMIT;` after pragma to force lock |
| R2-8 | TOCTOU: must use canonicalized path for I/O | Documented: use canonicalized path for Connection::open and remove_file |
| R2-9 | LOCAL_MEMORY_HOME symlink bypass | Canonicalize before checking bad prefixes |
| R2-10 | Windows UNC path rejection | Added `\\` prefix rejection on Windows |
| R2-11 | memory_vec join path ambiguity | Documented key mapping in both DESIGN.md and core-db-layer.md |
| R2-12 | sqlite-vec transmute safety | Added `# Safety` comment and version coupling documentation |
| R2-13 | Cargo.toml not yet updated | Expected — Task 1 deliverable |
| R2-14 | sqlite-vec alpha yanking risk | Documented git fallback strategy |

Changes from API Contract principle addition:

| # | Finding | Resolution |
|---|---------|------------|
| AC-1 | Raw &Connection exposed to all components | Added `Db` trait in §1b; `StoreManager::db()` returns `&dyn Db` |
| AC-2 | Parallel agents would write conflicting SQL | All SQL centralized in `impl Db for Connection`; components code against trait |
| AC-3 | DESIGN.md missing principle | Added "Design Principle: API Contracts for Parallel Development" section |

Changes from design review round 3:

| # | Finding | Resolution |
|---|---------|------------|
| R3-1 | Db trait object safety not documented | Added Contract Rule 6: no generics, no impl Trait, no async fn. Compile-time assertion. |
| R3-2 | No transaction boundary for multi-step mutations | Added Contract Rule 7: composite ops are single trait methods with internal transactions |
| R3-3 | §5 Task 1 contradicts §3 (From impls) | Removed contradictory line, replaced with "no blanket From impls" |
| R3-4 | §5 Task 4 references conn() not db() | Updated heading and acceptance criteria to use db(), close_active_best_effort() |
| R3-5 | Integrity check runs after migrate | Moved before migrate in both §1 prose and §7 Task 2 instructions |
| R3-6 | Db trait Send/Sync + serial execution undocumented | Added "Send/Sync and Serial Execution" subsection to §1b and note in §2.1 |
| R3-7 | Windows delete retry error matching underspecified | Specified PermissionDenied + raw_os_error(32) |
| R3-8 | No method signature constraints (raw SQL/paths) | Added Contract Rule 8: typed domain values only |
| R3-9 | Trait growth breaks mocks | Added Contract Rule 9: default impls for new methods |

---

## 1. db.rs — Database Connection & Schema

### Responsibility

Open a SQLite connection to a given file path, apply schema migrations, configure pragmas, load extensions, and return a ready-to-use connection.

### Public API

```rust
use rusqlite::Connection;
use std::path::Path;
use crate::error::MemoryError;

/// Current schema version this binary understands.
pub const SCHEMA_VERSION: u32 = 1;

/// Embedding vector dimension.
pub const EMBEDDING_DIM: u32 = 384;

/// Open (or create) a SQLite database at `path`, apply pragmas,
/// load sqlite-vec, run migrations, and run integrity check.
/// Returns a configured connection.
pub fn open(path: &Path) -> Result<Connection, MemoryError> { ... }

/// Run schema migrations from the current version to SCHEMA_VERSION.
/// Called internally by `open`. Exposed for testing.
pub fn migrate(conn: &Connection) -> Result<(), MemoryError> { ... }
```

### Connection Configuration (Pragmas)

Applied immediately after opening, before any schema work:

```sql
PRAGMA journal_mode = WAL;
PRAGMA synchronous = FULL;            -- durability over speed (safe default)
PRAGMA foreign_keys = ON;
PRAGMA busy_timeout = 5000;
PRAGMA cache_size = -2000;            -- 2MB page cache
PRAGMA wal_autocheckpoint = 1000;
PRAGMA journal_size_limit = 67108864; -- 64MB WAL cap
PRAGMA locking_mode = EXCLUSIVE;      -- prevent concurrent instance conflicts
```

**Rationale for FULL sync**: The write volume for agent memory is low (human-speed interactions). `FULL` ensures no data loss on power failure. Can be overridden to `NORMAL` via `LOCAL_MEMORY_SYNC=normal` env var for bulk import scenarios.

**Rationale for EXCLUSIVE locking**: Prevents two binary instances from opening the same store simultaneously. Eliminates the need for advisory file locks. Tradeoff: external tools like `sqlite3` CLI cannot read the DB while the server is running. Acceptable for a single-user MCP server.

### Extension Loading

Load `sqlite-vec` via `rusqlite::auto_extension`, guarded by `OnceLock` to ensure single initialization with error propagation:

```rust
use std::sync::OnceLock;
use rusqlite::auto_extension::{RawAutoExtension, register_auto_extension};
use sqlite_vec::sqlite3_vec_init;

static VEC_INIT: OnceLock<Result<(), String>> = OnceLock::new();

fn ensure_sqlite_vec() -> Result<(), MemoryError> {
    let result = VEC_INIT.get_or_init(|| {
        // SAFETY: sqlite3_vec_init has the same signature as RawAutoExtension
        // (sqlite3_auto_extension callback). This transmute is valid only for
        // sqlite-vec 0.1.7-alpha.10 with rusqlite 0.35.0. Any version bump
        // of either crate requires re-verifying this cast.
        unsafe {
            let raw: RawAutoExtension = std::mem::transmute(sqlite3_vec_init as usize);
            register_auto_extension(raw).map_err(|e| e.to_string())
        }
    });
    result.as_ref().map(|_| ()).map_err(|e| MemoryError::SchemaError(
        format!("failed to register sqlite-vec: {e}")
    ))
}
```

Called at the start of every `db::open`. `OnceLock` ensures the registration runs exactly once and propagates the error to all callers if it fails. After registration, `open` also calls `SELECT vec_version()` as a smoke test to verify the extension is functional.

### Integrity Check

After pragmas, extension loading, and lock acquisition — but **before migration** — if this is a **pre-existing** database (`PRAGMA user_version > 0`), run:

```sql
PRAGMA quick_check;
```

If the result is not `"ok"`, return `MemoryError::DatabaseCorrupted(path)`. Skip for brand-new databases (user_version == 0) since there's nothing to check. This avoids blocking all MCP handlers during store switches on large databases.

### Lock Acquisition

After setting `PRAGMA locking_mode = EXCLUSIVE`, force immediate lock acquisition:

```sql
BEGIN IMMEDIATE; COMMIT;
```

If this fails with `SQLITE_BUSY`, another process holds the database. Return `MemoryError::StoreLocked(path)` with a clear message. This eliminates the race window where two instances could both set EXCLUSIVE mode before either acquires the actual lock.

### Schema Migration Strategy

**Version tracking**: `PRAGMA user_version` (built-in SQLite integer in the DB header — no extra table needed).

**Migration flow** (inside `open`):

1. Read `PRAGMA user_version` → `stored_version`
2. If `stored_version > SCHEMA_VERSION` → return `MemoryError::SchemaVersionTooNew`
3. If `stored_version < SCHEMA_VERSION` → run migrations sequentially
4. **Each migration step** uses `conn.transaction()` (explicit transaction)
5. **After each successful step**, update `PRAGMA user_version = N` inside the transaction
6. If a step fails, the transaction rolls back; version stays at the last successful step

**V0 → V1 migration** (initial schema):

Creates all tables and indexes defined in DESIGN.md:
- `events` + 3 indexes
- `memories` (with explicit `memory_rowid INTEGER PRIMARY KEY` for FTS5 stability) + 2 indexes
- `knowledge_edges` + 3 indexes
- `memory_vec` (sqlite-vec virtual table)
- `memory_fts` (FTS5 virtual table using `content_rowid=memory_rowid`) + triggers
- `namespaces`
- `checkpoints` + unique index
- `branches` + index

**Memories table change** (addresses FTS5 rowid instability):

```sql
CREATE TABLE IF NOT EXISTS memories (
    memory_rowid INTEGER PRIMARY KEY,  -- stable rowid for FTS5
    id TEXT UNIQUE NOT NULL,           -- UUID (logical primary key)
    actor_id TEXT NOT NULL,
    -- ... rest unchanged from DESIGN.md
);
```

**FTS5 definition** (uses stable `memory_rowid`):

```sql
CREATE VIRTUAL TABLE IF NOT EXISTS memory_fts USING fts5(
    content,
    content=memories,
    content_rowid=memory_rowid
);
```

**FTS5 sync triggers**:

```sql
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
```

### Error Handling

| Condition | Error |
|-----------|-------|
| Can't create/open file | `ConnectionFailed` (include OS error) |
| Pragma fails | `ConnectionFailed` |
| sqlite-vec load fails | `SchemaError("failed to load sqlite-vec: ...")` |
| Integrity check fails | `DatabaseCorrupted(path)` |
| Stored version > binary version | `SchemaVersionTooNew(stored, SCHEMA_VERSION)` |
| Migration SQL fails | `SchemaError` (include migration version + SQL error) |

**Two-tier error strategy**: Internal errors logged via `tracing::error!` with full detail (paths, SQL, OS errors). Errors returned to callers use the `MemoryError` enum. The MCP layer (Component 8) will sanitize these into generic messages before sending to the client — no paths or SQL in JSON-RPC responses.

### Edge Cases

- **New database**: `user_version` is 0 → run all migrations from V0.
- **Already current**: `stored_version == SCHEMA_VERSION` → no-op, return connection.
- **Partial migration failure**: Transaction rolls back. `user_version` stays at last successful step. Next `open` retries from that step.
- **FTS5 query injection**: Not handled here. The `search.rs` component (Component 4) must sanitize all FTS5 MATCH inputs by escaping special characters before interpolation. This is documented as a contract for that component.
- **VACUUM and rowid stability**: The `memory_rowid INTEGER PRIMARY KEY` column is an alias for SQLite's internal rowid, making it stable across VACUUM. Safe for FTS5 content-sync.
- **Virtual table key mapping**: `memory_vec` uses `memories.id` (TEXT UUID) as its primary key. `memory_fts` uses `memories.memory_rowid` (INTEGER) as its content_rowid. Combined search in search.rs must join through `memories`: `memory_fts.rowid = memories.memory_rowid` and `memory_vec.memory_id = memories.id`. This is documented in both DESIGN.md and here.

---

## 1b. db.rs — Db Trait (API Contract)

### Responsibility

Define the trait that all downstream components code against. This is the contract boundary for parallel development. All SQL lives in the trait implementation — no other module writes SQL directly.

### Why a Trait

Per the API Contract design principle in DESIGN.md: components communicate through trait methods, not raw SQL. This means:
- Agents coding events.rs and memories.rs in parallel cannot conflict on SQL
- Schema changes are a single-file concern (db.rs)
- The trait signature is the reviewed, stable contract

### Trait Definition

The trait is defined in Component 1 but populated incrementally as each component is designed. Component 1 ships the trait with **store management methods only**. Components 2-7 each add their methods to the trait during their own design phase.

```rust
/// API contract for all database operations. Implemented for rusqlite::Connection.
/// Downstream components (events.rs, memories.rs, etc.) accept &dyn Db.
pub trait Db {
    // -- Store management (Component 1) --

    /// Get database file size in bytes (for memory.stats).
    fn db_size(&self) -> Result<u64, MemoryError>;

    // -- Events (Component 2 will add) --
    // fn insert_event(...) -> Result<String, MemoryError>;
    // fn get_event(...) -> Result<Event, MemoryError>;
    // fn get_events(...) -> Result<Vec<Event>, MemoryError>;
    // fn list_sessions(...) -> Result<Vec<SessionInfo>, MemoryError>;
    // fn delete_expired_events(...) -> Result<u64, MemoryError>;

    // -- Memories (Component 3 will add) --
    // fn insert_memory(...) -> Result<String, MemoryError>;
    // fn get_memory(...) -> Result<Memory, MemoryError>;
    // fn list_memories(...) -> Result<Vec<Memory>, MemoryError>;
    // fn consolidate_memory(...) -> Result<(), MemoryError>;
    // fn delete_memory(...) -> Result<(), MemoryError>;

    // -- Search (Component 4 will add) --
    // fn search_fts(...) -> Result<Vec<Memory>, MemoryError>;
    // fn search_vector(...) -> Result<Vec<Memory>, MemoryError>;

    // -- Graph (Component 5 will add) --
    // fn insert_edge(...) -> Result<String, MemoryError>;
    // fn get_neighbors(...) -> Result<Vec<Neighbor>, MemoryError>;
    // fn traverse(...) -> Result<Vec<Memory>, MemoryError>;
    // ... etc

    // -- Sessions (Component 6 will add) --
    // fn create_checkpoint(...) -> Result<String, MemoryError>;
    // fn create_branch(...) -> Result<String, MemoryError>;
    // ... etc

    // -- Namespaces (Component 7 will add) --
    // fn create_namespace(...) -> Result<(), MemoryError>;
    // fn list_namespaces(...) -> Result<Vec<Namespace>, MemoryError>;
    // fn delete_namespace(...) -> Result<u64, MemoryError>;
}

impl Db for Connection {
    fn db_size(&self) -> Result<u64, MemoryError> {
        let page_count: u64 = self.pragma_query_value(None, "page_count", |r| r.get(0))
            .map_err(|e| { tracing::error!("page_count query failed: {e}"); MemoryError::QueryFailed("failed to query database size".into()) })?;
        let page_size: u64 = self.pragma_query_value(None, "page_size", |r| r.get(0))
            .map_err(|e| { tracing::error!("page_size query failed: {e}"); MemoryError::QueryFailed("failed to query database size".into()) })?;
        Ok(page_count * page_size)
    }
}
```

### Contract Rules

1. **Each component's design phase** defines the exact method signatures it needs added to `Db`
2. **The signatures are reviewed** as part of that component's design review — object safety and method rules (below) are checklist items
3. **Implementation** of the trait methods happens in `db.rs` during that component's coding phase
4. **Downstream code** receives `&dyn Db` — never `&Connection`
5. **Data types** (Event, Memory, SessionInfo, etc.) are defined in their respective modules and imported by db.rs
6. **Object safety** — all methods must be object-safe: no generic type parameters, no `Self` in return position, no `impl Trait` returns, no `async fn`. Collection returns use `Vec<T>`. Async is handled externally via `spawn_blocking` — trait methods are synchronous. Add a compile-time assertion in tests: `fn _assert_object_safe(_: &dyn Db) {}`
7. **Transactional atomicity** — operations requiring multiple SQL statements that must succeed or fail together MUST be a single trait method. The implementation wraps them in `conn.transaction()` internally. Callers never manage transactions. Example: `consolidate_memory()` atomically marks old memory invalid, inserts new memory, and updates `superseded_by` — all in one trait method, one transaction.
8. **No raw SQL or paths** — no method may accept raw SQL strings or filesystem paths. All parameters must be typed domain values (IDs, enums, structs).
9. **Default implementations** — newly added methods should provide a default that returns `MemoryError::QueryFailed("not implemented")`. This makes trait growth non-breaking for test mocks. Remove the default once the component is fully implemented.

### Send/Sync and Serial Execution

`Db` is intentionally NOT `Send`/`Sync` because the sole implementation wraps `rusqlite::Connection` which is `!Send`. `&dyn Db` must only be obtained and used within the same `spawn_blocking` closure that holds the `StoreManager` mutex lock. Do not capture `&dyn Db` across `.await` points or thread boundaries.

Because `&dyn Db` borrows from the `MutexGuard`, the lock is held for the entire duration of each tool call. **All MCP tool invocations execute serially.** This is acceptable for a single-user server with sub-millisecond SQLite operations.

### How StoreManager Exposes the Trait

```rust
impl StoreManager {
    /// Get a reference to the active database, as the Db trait.
    /// Returns `MemoryError::Disconnected` if no store is open.
    pub fn db(&self) -> Result<&dyn Db, MemoryError> { ... }
}
```

The concurrency model from §2.1 becomes:

```rust
let result = tokio::task::spawn_blocking({
    let store = store.clone();
    move || {
        let mgr = store.lock().unwrap_or_else(|e| e.into_inner());
        let db = mgr.db()?;
        // call trait methods, not raw SQL
        db.insert_event(...)
    }
}).await?;
```

---

## 2. store.rs — StoreManager

### Responsibility

Manage the lifecycle of memory stores: resolve paths, open/close connections, switch between stores, list available stores, delete stores.

### Public API

```rust
use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::Mutex;
use crate::error::MemoryError;

pub struct StoreManager {
    base_dir: PathBuf,
    active_store: Option<ActiveStore>,
}

struct ActiveStore {
    name: String,
    conn: Connection,
}

impl StoreManager {
    /// Create a StoreManager. Resolves base_dir from:
    /// 1. `LOCAL_MEMORY_HOME` env var (validated as absolute path)
    /// 2. Otherwise platform-appropriate default:
    ///    - Unix: `~/.local-memory-mcp/`
    ///    - Windows: `%LOCALAPPDATA%/local-memory-mcp/`
    /// Creates the directory with restrictive permissions if it doesn't exist.
    pub fn new() -> Result<Self, MemoryError> { ... }

    /// Create a StoreManager with an explicit base directory. For testing.
    pub fn with_base_dir(base_dir: PathBuf) -> Result<Self, MemoryError> { ... }

    /// Open the default store ("default"). Called once at startup.
    pub fn open_default(&mut self) -> Result<(), MemoryError> { ... }

    /// Get a reference to the active database as the Db trait.
    /// Returns `MemoryError::Disconnected` if no store is open.
    pub fn db(&self) -> Result<&dyn Db, MemoryError> { ... }

    /// Name of the active store. None if disconnected.
    pub fn active_name(&self) -> Option<&str> { ... }

    /// Close the current store (with WAL checkpoint) and open a different one.
    /// Creates the store file if it doesn't exist.
    pub fn switch(&mut self, name: &str) -> Result<(), MemoryError> { ... }

    /// List all stores: name and file size in bytes.
    pub fn list(&self) -> Result<Vec<StoreInfo>, MemoryError> { ... }

    /// Delete a store by name. Cannot delete the active store.
    pub fn delete(&self, name: &str) -> Result<(), MemoryError> { ... }
}

pub struct StoreInfo {
    pub name: String,
    pub size_bytes: u64,
}
```

### 2.1 Concurrency Model

`rusqlite::Connection` is `!Send`. The MCP server runs on tokio multi-thread. The integration pattern:

```rust
// In the MCP server (Component 8):
let store = Arc<std::sync::Mutex<StoreManager>>;

// In each tool handler:
let result = tokio::task::spawn_blocking({
    let store = store.clone();
    move || {
        let mgr = store.lock().unwrap_or_else(|e| e.into_inner());
        let db = mgr.db()?;
        // call trait methods, not raw SQL
        db.insert_event(...)
    }
}).await?;
```

**Why `std::sync::Mutex` not `tokio::sync::Mutex`**: SQLite operations are CPU-bound and fast (sub-ms). `std::sync::Mutex` doesn't hold across `.await` points. `spawn_blocking` moves the work off the async runtime. This is the idiomatic pattern for rusqlite + tokio.

**Mutex poisoning strategy**: Use `.unwrap_or_else(|e| e.into_inner())` to recover from poisoned mutexes. A panic in one `spawn_blocking` task (e.g., from an unexpected SQLite error) should not permanently kill the server. SQLite transactions roll back on panic, so the recovered `StoreManager` is in a consistent state. This is safe because `StoreManager` has no invariants that can be violated by a partial operation — the worst case is an uncommitted transaction, which SQLite handles via rollback.

**Serial execution**: Because `&dyn Db` borrows from the `MutexGuard`, the lock is held for the entire duration of each tool call. All MCP tool invocations execute serially. This is acceptable for a single-user server with sub-millisecond SQLite operations.

This design keeps `StoreManager` unaware of async — it's a synchronous API. The async boundary is the MCP server's responsibility (Component 8).

### 2.2 Store Name Validation

Store names must satisfy ALL of:
- Length: 1–64 characters
- First character: ASCII alphanumeric
- Remaining characters: ASCII alphanumeric, underscore, or hyphen
- Not a Windows reserved device name (case-insensitive): `CON`, `PRN`, `AUX`, `NUL`, `COM1`–`COM9`, `LPT1`–`LPT9`

Regex equivalent: `^[a-zA-Z0-9][a-zA-Z0-9_-]{0,63}$` plus reserved name check.

Implemented as manual character checks (no `regex` crate). Returns `MemoryError::InvalidName` on failure.

### 2.3 Path Resolution

```
{base_dir}/{name}.db
```

**Base directory resolution**:
1. If `LOCAL_MEMORY_HOME` is set and non-empty:
   - Validate it is an absolute path (`Path::is_absolute()`)
   - Reject if it contains `..` components
   - On Windows: reject paths starting with `\\` (UNC/device paths)
   - Reject known-bad prefixes: `/dev/`, `/proc/`, `/sys/` (Unix)
   - If the path exists, **canonicalize** it and re-check the known-bad prefixes against the resolved path (prevents symlink bypass: e.g., `/tmp/innocent` → `/proc/self/fd`)
   - If validation fails → `MemoryError::InvalidPath` with message including `LOCAL_MEMORY_HOME`
2. Otherwise:
   - Unix: `dirs::home_dir()` + `.local-memory-mcp/`
   - Windows: `dirs::data_local_dir()` + `local-memory-mcp/`
   - If neither returns a value → `MemoryError::InvalidPath("Cannot determine home directory. Set LOCAL_MEMORY_HOME environment variable.")`

**Directory creation**:
- `std::fs::create_dir_all(&base_dir)`
- On Unix: set permissions to `0o700` via `std::os::unix::fs::PermissionsExt`

**Symlink protection** (before open or delete):
1. Resolve the full path: `base_dir.join(format!("{name}.db"))`
2. If the file exists, check with `std::fs::symlink_metadata()`:
   - If `file_type().is_symlink()` → return `MemoryError::InvalidPath("store path is a symlink")`
3. Canonicalize the parent directory and verify the resolved path is still under `base_dir`
4. **Use the canonicalized path** for the actual I/O operation (`Connection::open`, `fs::remove_file`) — not the original joined path. This closes the TOCTOU window between check and use.

### 2.4 Store Lifecycle

**Startup**:
1. `StoreManager::new()` — resolve and create base_dir
2. `open_default()` — open `default.db` via `db::open`

**Switch**:
1. Validate name
2. If same as active store → no-op, return Ok
3. Call `close_active()` (checkpoint + drop)
4. Symlink check on new path
5. Open new store via `db::open`
6. Set as active

**close_active()** (called on switch — propagates errors):
1. If active store exists:
   - Execute `PRAGMA wal_checkpoint(TRUNCATE)` — if this fails, return `MemoryError::ConnectionFailed` (data loss risk)
   - Execute `PRAGMA optimize` — swallow errors (optimization, not critical)
   - Drop the connection
   - Set `active_store = None`

**close_active_best_effort()** (called from Drop — swallows all errors):
1. If active store exists:
   - Execute `PRAGMA wal_checkpoint(TRUNCATE)` and `PRAGMA optimize` — log warnings on failure via `tracing::warn!`, do not propagate
   - Drop the connection
   - Set `active_store = None`

**Connection access**:
- `db()` returns `&dyn Db` or `Disconnected` error
- All tool implementations call `store_manager.db()` to get the trait object

**Drop implementation**:
- `impl Drop for StoreManager` calls `close_active_best_effort()` to ensure clean shutdown without panicking

### 2.5 List Stores

1. Read `base_dir` directory entries
2. Filter for files matching `*.db` that are regular files (not symlinks, via `symlink_metadata`)
3. Strip `.db` extension → store name
4. Get file size: sum of `.db` + `.db-wal` + `.db-shm` sizes (if they exist)
5. Sort alphabetically
6. Skip entries that fail metadata reads (log warning via `tracing::warn!`)

### 2.6 Delete Store

1. Validate name
2. If name == active store name → return `MemoryError::ActiveStoreDeletion`
3. Resolve path, run symlink check
4. If `.db` file doesn't exist → return `MemoryError::NotFound`
5. Delete in reverse order (prevents stale WAL recovery on re-create):
   - Remove `.db-shm` (ignore not-found)
   - Remove `.db-wal` (ignore not-found)
   - Remove `.db`
6. On Windows: if remove fails with `io::Error::kind() == PermissionDenied` OR `io::Error::raw_os_error() == Some(32)` (ERROR_SHARING_VIOLATION), retry up to 3 times with 100ms delay
7. Return Ok

### 2.7 Error Handling

| Condition | Error |
|-----------|-------|
| Invalid store name | `InvalidName(name)` |
| No home dir and no env var | `InvalidPath(message including LOCAL_MEMORY_HOME hint)` |
| LOCAL_MEMORY_HOME not absolute | `InvalidPath("LOCAL_MEMORY_HOME must be an absolute path")` |
| Path is a symlink | `InvalidPath("store path is a symlink")` |
| Can't create base_dir | `ConnectionFailed(os_error)` |
| No active connection | `Disconnected` |
| Delete active store | `ActiveStoreDeletion(name)` |
| Delete non-existent store | `NotFound(name)` |
| File delete fails | `DeleteFailed(os_error)` |
| Database corrupted | `DatabaseCorrupted(path)` |
| Store locked by another process | `StoreLocked(path)` |
| db::open fails | Propagated from db module |

### 2.8 Edge Cases

- **`LOCAL_MEMORY_HOME` with `~`**: Not expanded. Rejected by `is_absolute()` check (on Unix, `~/foo` is relative). Error message tells user to use an absolute path.
- **Base dir deleted while running**: Next `switch` will fail with OS error on `db::open`. Acceptable.
- **Store file locked by another process**: `PRAGMA locking_mode = EXCLUSIVE` in `db::open` will fail immediately. Returns `ConnectionFailed`.
- **Switching to the same store**: No-op.
- **Symlinks in base_dir**: Detected and rejected before open/delete.

---

## 3. error.rs — Updates Required

Add the following variants to `MemoryError`:

```rust
#[error("Database corrupted: {0}")]
DatabaseCorrupted(String),

#[error("Store is locked by another process: {0}")]
StoreLocked(String),
```

**No blanket `From` implementations.** Use explicit `.map_err()` at each call site to map to the correct variant. This preserves the error taxonomy and prevents raw SQL/paths from leaking into error strings. Each call site chooses the appropriate variant and controls what context is included:

```rust
// Example: map to ConnectionFailed for open errors
Connection::open(path).map_err(|e| {
    tracing::error!("SQLite open failed: {e}");
    MemoryError::ConnectionFailed(format!("failed to open database"))
})?;

// Example: map to SchemaError for migration errors
tx.execute_batch(ddl).map_err(|e| {
    tracing::error!("Migration V1 failed: {e}");
    MemoryError::SchemaError(format!("migration to V1 failed"))
})?;
```

This is more verbose but ensures: (1) full detail goes to tracing logs, (2) only generic messages go into the error enum, (3) the MCP layer's sanitization is defense-in-depth, not the only line of defense.

---

## 4. Cargo.toml — Updates Required

Add missing dependency and pin critical crates:

```toml
[dependencies]
rmcp = { version = "=1.5.0", features = ["transport-io", "server"] }
rusqlite = { version = "=0.35.0", features = ["bundled", "vtab"] }
sqlite-vec = "=0.1.7-alpha.10"
tokio = { version = "1", features = ["macros", "rt-multi-thread", "sync"] }
thiserror = "2"
dirs = "6"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
uuid = { version = "1", features = ["v4"] }
```

Also: run `cargo generate-lockfile` and commit `Cargo.lock`.

**sqlite-vec alpha risk**: The `0.1.7-alpha.10` version could be yanked from crates.io. If this happens, fallback to a git dependency: `sqlite-vec = { git = "https://github.com/asg017/sqlite-vec", rev = "<commit>" }`. Track sqlite-vec releases and upgrade to stable when available.

---

## 5. Implementation Plan

### Task 1: Cargo.toml + error.rs updates

**Acceptance criteria**:
- `sqlite-vec` added to Cargo.toml with exact pin
- `rusqlite` pinned to exact version
- `Cargo.lock` generated and committed
- `DatabaseCorrupted` and `StoreLocked` variants added to `MemoryError`
- No blanket `From` impls — all error mapping uses explicit `.map_err()`
- `cargo check` passes

### Task 2: db::open — connection + pragmas + extension + integrity check

**Acceptance criteria**:
- Opens or creates a SQLite file at the given path
- All pragmas applied (FULL sync, EXCLUSIVE locking, WAL, 64MB journal limit)
- sqlite-vec loaded via `register_auto_extension`
- `PRAGMA quick_check` run; returns `DatabaseCorrupted` on failure
- Calls `migrate()`
- Returns `Connection` or typed error
- Tests: open new file, verify pragmas, verify sqlite-vec, verify integrity check

### Task 2b: Db trait definition + impl for Connection

**Acceptance criteria**:
- `pub trait Db` defined in `db.rs` with `db_size()` method
- `impl Db for Connection` with working `db_size()` implementation
- Commented placeholder method signatures for Components 2-7
- Object safety compile-time assertion: `fn _assert_object_safe(_: &dyn Db) {}`
- Doc comments on trait and `db()` method documenting Send/Sync constraints
- Test: call `db_size()` on a fresh connection, verify returns > 0

### Task 3: db::migrate — schema versioning + V1 migration

**Acceptance criteria**:
- Uses `PRAGMA user_version` (not a `_meta` table)
- V0→V1 creates all tables/indexes/triggers in an explicit transaction
- Updates `user_version` to 1 inside the transaction
- Returns `SchemaVersionTooNew` if stored > binary
- No-op if already at current version
- `memories` table has `memory_rowid INTEGER PRIMARY KEY` + `id TEXT UNIQUE NOT NULL`
- FTS5 uses `content_rowid=memory_rowid`
- Tests: fresh DB gets all tables; re-open is no-op; future version rejected; FTS trigger sync works

### Task 4: StoreManager::new + open_default + db + close_active + Drop

**Acceptance criteria**:
- Resolves base_dir from env var (validated) or platform default
- Creates base_dir with 0o700 permissions on Unix
- `open_default()` opens `default.db` via `db::open`
- `db()` returns `&dyn Db` or `Disconnected`
- `close_active()` returns `Result`, runs WAL checkpoint (propagates errors) + optimize
- `close_active_best_effort()` swallows all errors, logs warnings
- `Drop` calls `close_active_best_effort()`
- Symlink check before opening
- Tests: create in temp dir, verify dir permissions, verify default.db, verify db() works, verify env var validation

### Task 5: StoreManager::switch + validate_name

**Acceptance criteria**:
- Validates name (length, chars, Windows reserved names)
- Switching to same store is no-op
- Calls `close_active()` then opens new store
- Symlink check on new path
- Tests: switch to new store, invalid names rejected, reserved names rejected, same-store no-op

### Task 6: StoreManager::list + delete

**Acceptance criteria**:
- `list()` returns regular files only (no symlinks), sizes include WAL/SHM
- `delete()` rejects active store, rejects non-existent, validates name
- Deletes in reverse order: shm, wal, db
- Symlink check before delete
- Tests: create multiple stores, list them, delete one, verify WAL/SHM cleanup

---

## 6. Dependency DAG

```
Task 1 (Cargo.toml + error.rs)
    │
    ▼
Task 2 (db::open)
    │
    ├──────────────┐
    ▼              ▼
Task 2b         Task 3
(Db trait)      (db::migrate)    ← both depend on Task 2, can run in parallel
    │              │
    └──────┬───────┘
           ▼
Task 4 (StoreManager core)      ← depends on Task 2b + Task 3
    │
    ├──────────────┐
    ▼              ▼
Task 5          Task 6           ← both depend on Task 4, can run in parallel
(switch)        (list+delete)
```

---

## 7. Sub-Agent Build Instructions

### Prerequisites

- Rust toolchain installed
- Working directory: project root
- Read `src/error.rs`, `Cargo.toml`, `design/DESIGN.md` for context

### Task 1: Cargo.toml + error.rs

**File**: `Cargo.toml`, `src/error.rs`

1. Add `sqlite-vec = "=0.1.7-alpha.10"` to `[dependencies]`
2. Change `rusqlite` to `"=0.35.0"`
3. Run `cargo generate-lockfile`
4. In `src/error.rs`:
   - Add `#[error("Database corrupted: {0}")] DatabaseCorrupted(String)` variant
   - Add `#[error("Store is locked by another process: {0}")] StoreLocked(String)` variant
   - Do NOT add blanket `From` impls — use explicit `.map_err()` at each call site
5. Run `cargo check`

### Task 2: db::open

**File**: `src/db.rs`

1. Add imports: `rusqlite::Connection`, `std::path::Path`, `crate::error::MemoryError`
2. Add `use sqlite_vec::sqlite3_vec_init;` and `use rusqlite::auto_extension::{RawAutoExtension, register_auto_extension};`
3. Define `pub const SCHEMA_VERSION: u32 = 1;` and `pub const EMBEDDING_DIM: u32 = 384;`
4. Implement `fn ensure_sqlite_vec()` using `std::sync::OnceLock` as shown in §1 Extension Loading. Wrap the unsafe transmute with `# Safety` comment documenting the version coupling.
5. Implement `pub fn open(path: &Path) -> Result<Connection, MemoryError>`:
   - Call `ensure_sqlite_vec()?`
   - `Connection::open(path)` — use `.map_err()` to `ConnectionFailed` (log full error, store generic message)
   - Execute each pragma, map errors to `ConnectionFailed`
   - Check `LOCAL_MEMORY_SYNC` env var: if `"normal"`, use `synchronous = NORMAL`; otherwise `FULL`
   - Force lock acquisition: `BEGIN IMMEDIATE; COMMIT;` — if SQLITE_BUSY, return `StoreLocked`
   - Run `SELECT vec_version()` as smoke test — if fails, return `SchemaError`
   - If `PRAGMA user_version > 0` (pre-existing DB), run `PRAGMA quick_check` — if not "ok", return `DatabaseCorrupted`
   - Call `migrate(&conn)?`
   - Return `Ok(conn)`
6. Tests:
   - `test_open_creates_file`: open in tempdir, assert file exists
   - `test_open_wal_mode`: query `PRAGMA journal_mode`, assert "wal"
   - `test_open_foreign_keys`: query `PRAGMA foreign_keys`, assert 1
   - `test_open_exclusive_locking`: query `PRAGMA locking_mode`, assert "exclusive"
   - `test_sqlite_vec_loaded`: execute `SELECT vec_version()`, assert no error
   - `test_open_locked_db`: open same file twice, second should return `StoreLocked`

### Task 2b: Db trait

**File**: `src/db.rs` (append after `open`)

1. Define `pub trait Db`:
   - `fn db_size(&self) -> Result<u64, MemoryError>;`
   - Add commented placeholder signatures for Components 2-7 (as shown in §1b)
2. Implement `impl Db for Connection`:
   - `db_size()`: query `PRAGMA page_count` and `PRAGMA page_size`, multiply, return
3. Tests:
   - `test_db_size`: open fresh DB, call `db_size()`, assert > 0

### Task 3: db::migrate

**File**: `src/db.rs` (append)

1. Implement `pub fn migrate(conn: &Connection) -> Result<(), MemoryError>`:
   - `let version: u32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;`
   - If `version > SCHEMA_VERSION` → return `SchemaVersionTooNew(version, SCHEMA_VERSION)`
   - If `version < 1` → call `migrate_v1(conn)?`
   - Return Ok
2. Implement `fn migrate_v1(conn: &Connection) -> Result<(), MemoryError>`:
   - Use `let tx = conn.transaction().map_err(|e| MemoryError::SchemaError(...))?;`
   - Execute all CREATE TABLE/INDEX/TRIGGER statements via `tx.execute_batch(...)`
   - Note: `memories` table uses `memory_rowid INTEGER PRIMARY KEY, id TEXT UNIQUE NOT NULL`
   - FTS5 uses `content_rowid=memory_rowid`
   - `tx.pragma_update(None, "user_version", 1)?;`
   - `tx.commit()?`
3. Tests:
   - `test_migrate_fresh_db`: open fresh, verify all tables exist via `sqlite_master`
   - `test_migrate_idempotent`: open twice, second is no-op
   - `test_migrate_future_version`: set `user_version` to 999, re-open, assert `SchemaVersionTooNew`
   - `test_fts_trigger_sync`: insert into `memories`, verify row in `memory_fts`

### Task 4: StoreManager core

**File**: `src/store.rs`

1. Add imports
2. Define `StoreInfo`, `ActiveStore`, `StoreManager` structs
3. Implement `StoreManager::new()`:
   - Check `LOCAL_MEMORY_HOME`: validate absolute, no `..`, no `/dev/` etc., no `\\` prefix on Windows
   - If path exists, canonicalize and re-check bad prefixes (prevents symlink bypass)
   - Fallback: Unix `home_dir()/.local-memory-mcp/`, Windows `data_local_dir()/local-memory-mcp/`
   - `create_dir_all`, then on Unix set permissions to 0o700
4. Implement `with_base_dir(base_dir)` for testing
5. Implement `fn check_not_symlink(path: &Path) -> Result<(), MemoryError>`:
   - If path exists and `symlink_metadata` shows symlink → `InvalidPath`
6. Implement `fn resolve_and_verify(base_dir: &Path, name: &str) -> Result<PathBuf, MemoryError>`:
   - Build path, check_not_symlink, canonicalize parent, verify under base_dir
   - Return the **canonicalized** path for use in actual I/O
7. Implement `open_default()` → calls `open_store("default")`
8. Implement `fn open_store(&mut self, name: &str)`:
   - `validate_name(name)?`
   - `let path = resolve_and_verify(&self.base_dir, name)?`
   - Call `db::open(&path)` with the canonicalized path
   - Set `active_store`
9. Implement `db(&self)` → `Result<&dyn Db, MemoryError>`:
   - Match on `self.active_store`, return `&conn` (Connection implements Db) or `Disconnected`
10. Implement `active_name()`
10. Implement `fn close_active(&mut self) -> Result<(), MemoryError>`:
    - If active: run `PRAGMA wal_checkpoint(TRUNCATE)` — return error on failure
    - Run `PRAGMA optimize` — swallow errors
    - Set `active_store = None`
11. Implement `fn close_active_best_effort(&mut self)`:
    - Same as above but swallow all errors, log warnings
12. Implement `Drop for StoreManager` → calls `close_active_best_effort()`
11. Tests:
    - `test_new_creates_dir`: verify dir exists
    - `test_open_default`: verify `active_name() == Some("default")`
    - `test_db_before_open`: new manager, call `db()`, assert `Disconnected`
    - `test_env_var_validation`: relative path rejected, `..` rejected
    - `test_symlink_rejected`: create symlink, try to open, assert error

### Task 5: StoreManager::switch + validate_name

**File**: `src/store.rs` (append)

1. Implement `fn validate_name(name: &str) -> Result<(), MemoryError>`:
   - Check length 1-64
   - First char alphanumeric
   - Remaining chars alphanumeric/underscore/hyphen
   - Check against Windows reserved names (case-insensitive)
2. Implement `switch(&mut self, name: &str)`:
   - `validate_name(name)?`
   - If same as active → return Ok
   - `self.close_active()?` (propagates checkpoint errors)
   - `self.open_store(name)`
3. Tests:
   - `test_switch_creates_new_store`
   - `test_switch_same_store_noop`
   - `test_switch_invalid_names`: `"../evil"`, `".hidden"`, `""`, 65-char string
   - `test_switch_reserved_names`: `"CON"`, `"nul"`, `"com1"`
   - `test_switch_valid_names`: `"a"`, `"my-store"`, `"store_123"`

### Task 6: StoreManager::list + delete

**File**: `src/store.rs` (append)

1. Implement `list()`:
   - `read_dir`, filter `*.db` regular files (not symlinks via `symlink_metadata`)
   - Size = sum of `.db` + `.db-wal` + `.db-shm`
   - Sort by name
2. Implement `delete(&self, name: &str)`:
   - `validate_name`, check not active, check not symlink
   - Check `.db` exists → `NotFound` if not
   - Delete: `.db-shm`, `.db-wal`, `.db` (reverse order)
   - On Windows: retry with 100ms delay up to 3 times on sharing violation
3. Tests:
   - `test_list_stores`: create multiple, verify sorted list with sizes
   - `test_list_skips_symlinks`: create symlink .db, verify not in list
   - `test_delete_store`: create, switch away, delete, verify files gone
   - `test_delete_active_store`: assert `ActiveStoreDeletion`
   - `test_delete_nonexistent`: assert `NotFound`
   - `test_delete_removes_wal_shm`: verify all 3 files removed

### Build Verification

After all tasks:
```bash
cargo check
cargo test
cargo clippy -- -D warnings
```

All must pass with zero warnings.
