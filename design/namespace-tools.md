# Component 8: Namespace Tools — Detailed Design

## Design Review Resolutions (Round 1)

### Critical / High

**C-1 — `delete_namespace` had no `actor_id` scoping (cross-actor data destruction)**
All five reviewers flagged this. The original design deleted `WHERE namespace = ?1` with no actor filter, allowing any caller to wipe memories belonging to all actors in a namespace.

Resolution: `delete_namespace` now requires `actor_id`. The Db trait method signature is `fn delete_namespace(&self, actor_id: &str, name: &str) -> Result<u64, MemoryError>`. The DELETE scopes to `WHERE namespace = ?1 AND actor_id = ?2`. The namespace registry entry is deleted unconditionally after the scoped memory delete (it is global metadata; other actors' memories in the same namespace path continue to exist, just in an unregistered namespace — consistent with the stated semantics). The MCP tool's param struct adds `actor_id: String`.

**H-1 — `delete_namespace` did not clean up `memory_vec` orphan rows**
The original design noted orphan rows are acceptable. Reviewers disagreed — they grow unbounded across delete/recreate cycles.

Resolution: The delete transaction now executes `DELETE FROM memory_vec WHERE memory_id IN (SELECT id FROM memories WHERE namespace = ?1 AND actor_id = ?2)` before deleting the memories rows. This cleans up vector rows in the same atomic transaction.

**H-2 — `delete_namespace` was unbounded — could block the server for large namespaces**
A single `DELETE FROM memories WHERE namespace = ?1` on a large namespace would hold the EXCLUSIVE lock for seconds or minutes.

Resolution: The delete loops in chunks of `NAMESPACE_DELETE_CHUNK_SIZE = 500` rows, committing each chunk before starting the next. This yields the lock between chunks so other tool calls can land. A `tracing::warn!` fires before the first chunk, logging namespace name and approximate memory count.

**H-3 — `validate_name` allowed control characters and homoglyph Unicode**
Resolution: `validate_name` now rejects any byte with value `< 0x20` (ASCII control chars), `0x7F` (DEL), and `\0` (already rejected). This blocks log-injection and terminal escape sequences. A full charset allowlist is deliberately not enforced — namespace names may include `/`, `-`, `.`, `:`, `@`, emoji, and other UTF-8 to stay compatible with AgentCore-style paths and diverse agent use cases. The tradeoff is documented in a code comment. The same control-char check is applied in `memories.rs::validate_insert_memory_params` when validating the `namespace` field, so the two paths agree.

**H-4 — `unchecked_transaction` not justified**
Resolution: Added a comment block above every `unchecked_transaction` call explaining the invariant: safe because `locking_mode = EXCLUSIVE` means no concurrent writers, and this code runs inside `spawn_blocking` which holds the `Mutex<StoreManager>`. No other thread can access the connection during the call. (This is the existing codebase pattern in graph.rs and memories.rs.)

### Medium / Low (logged to TODO backlog — no design change required)

- `create_namespace` silently ignores description updates on re-create (idempotent semantics). Add `update_namespace` in a future component or document clearly.
- Internal error message strings forwarded to MCP response body — project-wide existing issue, not specific to Component 8.
- No `memory.stats` breakdown including namespace-level edge counts.
- `delete_namespace` should optionally log `edges_deleted` count alongside `memories_deleted`.
- `MAX_NAMESPACE_LEN` and `MAX_DESCRIPTION_LEN` should eventually move to a shared `limits.rs` module.
- `delete_namespace` return type could be a `DeleteNamespaceOutcome` struct to allow adding `edges_deleted` later without breaking the trait.
- UTF-8 byte-length error message should say "bytes (UTF-8)" not just "bytes" for non-ASCII user clarity.
- `list_namespaces` LIKE prefix match is case-sensitive for non-ASCII — document this.
- Return `created: bool` from `create_namespace` so callers can distinguish create-vs-already-existed.
- Offset pagination has no upper bound — add a cap or document the limit.

---

## Scope

Namespace CRUD operations exposed as MCP tools. This component adds:

1. **Data types** — `Namespace`, `CreateNamespaceParams`, `ListNamespacesParams` in `src/namespaces.rs` (new file)
2. **Db trait methods** — 3 methods added to the `Db` trait in `src/db.rs`
3. **Business logic** — validation + delegation in `src/namespaces.rs`
4. **Db implementation** — SQL implementation of the 3 methods in `src/db.rs`
5. **MCP tool handlers** — 3 `#[tool]` methods in `src/tools.rs`
6. **Module wiring** — `pub mod namespaces` in `src/lib.rs`

This component does NOT include:
- Namespace filtering in `memory.list_memory_records` or `memory.retrieve_memory_records` — that already exists in Component 3/4
- Deletion of memories across sub-namespace prefixes — `delete_namespace` is exact-match only
- Schema changes — the `namespaces` table already exists in migration v1

---

## Namespace Semantics

The `namespaces` table holds **explicitly registered** namespaces. Memories can reference any namespace string without a corresponding entry — the namespace column on `memories` is a free-form label, not a foreign key.

Explicit registration serves two purposes:
1. Attach a human-readable `description` to a namespace path
2. Enable bulk deletion of a namespace and all its memories

An agent that stores memories with `namespace = '/user/alice/prefs'` without calling `create_namespace` will not see that namespace in `list_namespaces`. This is intentional — the list is a registry, not a scan of all namespaces in use.

`delete_namespace` targets **exact name matches** only. It does not delete sub-namespaces (e.g., deleting `/user` does not touch `/user/alice`). Agents that need prefix-based bulk deletion must first list and delete each namespace individually.

`create_namespace` is **idempotent**: if the namespace already exists, it returns the existing entry unchanged. This avoids errors when agents call `create_namespace` defensively at the start of each session.

---

## Schema (already exists — no migration needed)

```sql
CREATE TABLE IF NOT EXISTS namespaces (
    name TEXT PRIMARY KEY,
    description TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
```

No index needed beyond the primary key — queries are by exact name or LIKE prefix scan. The table is expected to be small (hundreds of rows at most).

---

## Constants

Reuse existing constants from `memories.rs` and `events.rs`:

```rust
// In namespaces.rs
pub const MAX_DESCRIPTION_LEN: usize = 1_024;
pub const NAMESPACE_DELETE_CHUNK_SIZE: usize = 500;

// Reused from memories.rs (already pub):
// MAX_NAMESPACE_LEN = 512

// Reused from events.rs (already pub):
// MAX_PAGE_LIMIT = 1_000
// DEFAULT_PAGE_LIMIT = 100
```

---

## Data Types

```rust
// src/namespaces.rs

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Namespace {
    pub name: String,
    pub description: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct CreateNamespaceParams<'a> {
    pub name: &'a str,
    pub description: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct ListNamespacesParams<'a> {
    pub prefix: Option<&'a str>,
    pub limit: u32,
    pub offset: u32,
}
```

---

## Db Trait Methods

Add to the `Db` trait in `src/db.rs`, replacing the commented-out placeholder block for Component 7 (namespaces):

```rust
// -- Namespaces (Component 8) --

/// Insert a namespace entry. Idempotent: if the name already exists, returns
/// the existing entry unchanged (description is NOT updated on conflict).
fn create_namespace(
    &self,
    name: &str,
    description: Option<&str>,
) -> Result<Namespace, MemoryError>;

/// List registered namespaces ordered by name ASC. Optional prefix filter
/// uses LIKE with escaped wildcards. Returns at most `params.limit` rows.
fn list_namespaces(
    &self,
    params: &ListNamespacesParams<'_>,
) -> Result<Vec<Namespace>, MemoryError>;

/// Delete all memories belonging to `actor_id` in the named namespace,
/// clean up their memory_vec rows, and remove the namespace registry entry.
/// Scoped to actor_id — does not touch other actors' memories.
/// Returns the count of memories deleted.
/// Returns NotFound if the namespace entry does not exist in the namespaces table.
fn delete_namespace(&self, actor_id: &str, name: &str) -> Result<u64, MemoryError>;
```

---

## SQL Implementation

### `create_namespace`

```sql
-- Step 1: Insert, ignoring conflict
INSERT INTO namespaces(name, description)
VALUES (:name, :description)
ON CONFLICT(name) DO NOTHING;

-- Step 2: Return current state (works whether insert happened or not)
SELECT name, description, created_at
FROM namespaces
WHERE name = :name;
```

Use `conn.execute(...)` for step 1 (returns affected rows — we don't use the value), then `conn.query_row(...)` for step 2. Two queries, no explicit transaction needed (the INSERT is atomic; the subsequent SELECT reflects committed state since we use EXCLUSIVE locking mode).

### `list_namespaces`

Without prefix:
```sql
SELECT name, description, created_at
FROM namespaces
ORDER BY name ASC
LIMIT :limit OFFSET :offset;
```

With prefix (use `escape_like(prefix) + "%"` as the bind value, `ESCAPE '\'`):
```sql
SELECT name, description, created_at
FROM namespaces
WHERE name LIKE :pattern ESCAPE '\'
ORDER BY name ASC
LIMIT :limit OFFSET :offset;
```

`escape_like` is already defined in `db.rs` as a private function. Promote it to `pub(crate)` so `namespaces.rs` can use it, or keep the SQL construction inside `db.rs`.

Since all SQL lives in `db.rs` (API contract principle), keep `escape_like` private and construct the LIKE pattern inside the `list_namespaces` impl method.

### `delete_namespace`

```rust
// In impl Db for Connection:
fn delete_namespace(&self, actor_id: &str, name: &str) -> Result<u64, MemoryError> {
    // Verify the namespace exists in the registry first
    let exists: bool = self
        .query_row(
            "SELECT 1 FROM namespaces WHERE name = ?1",
            [name],
            |_| Ok(true),
        )
        .optional()
        .map_err(|e| MemoryError::QueryFailed(format!("delete_namespace existence check failed: {e}")))?
        .unwrap_or(false);

    if !exists {
        return Err(MemoryError::NotFound(name.to_string()));
    }

    // Get an approximate count for observability logging
    let approx_count: u64 = self
        .query_row(
            "SELECT COUNT(*) FROM memories WHERE namespace = ?1 AND actor_id = ?2",
            rusqlite::params![name, actor_id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    tracing::warn!(namespace = name, actor_id, memories = approx_count, "deleting namespace");

    let mut total_deleted: u64 = 0;

    // Chunk the delete to avoid holding the EXCLUSIVE lock too long.
    // Each iteration commits one batch, letting other tool calls land between chunks.
    // SAFETY: unchecked_transaction is safe here — locking_mode = EXCLUSIVE means
    // no concurrent writers, and this fn runs inside spawn_blocking under Mutex<StoreManager>.
    loop {
        let tx = self.unchecked_transaction().map_err(|e| {
            MemoryError::QueryFailed(format!("delete_namespace begin tx failed: {e}"))
        })?;

        // Collect IDs to delete this chunk
        let ids: Vec<String> = {
            let mut stmt = tx
                .prepare(
                    "SELECT id FROM memories WHERE namespace = ?1 AND actor_id = ?2 LIMIT ?3",
                )
                .map_err(|e| {
                    MemoryError::QueryFailed(format!("delete_namespace prepare chunk failed: {e}"))
                })?;
            stmt.query_map(
                rusqlite::params![name, actor_id, NAMESPACE_DELETE_CHUNK_SIZE],
                |r| r.get(0),
            )
            .map_err(|e| {
                MemoryError::QueryFailed(format!("delete_namespace query chunk failed: {e}"))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| {
                MemoryError::QueryFailed(format!("delete_namespace collect chunk failed: {e}"))
            })?
        };

        if ids.is_empty() {
            tx.commit().map_err(|e| {
                MemoryError::QueryFailed(format!("delete_namespace final commit failed: {e}"))
            })?;
            break;
        }

        // Build IN clause for this batch
        let placeholders: String = ids
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 1))
            .collect::<Vec<_>>()
            .join(", ");

        // Delete memory_vec rows first (no FK, no trigger)
        let vec_sql = format!(
            "DELETE FROM memory_vec WHERE memory_id IN ({placeholders})"
        );
        let params: Vec<&dyn rusqlite::types::ToSql> =
            ids.iter().map(|id| id as &dyn rusqlite::types::ToSql).collect();
        tx.execute(&vec_sql, params.as_slice()).map_err(|e| {
            MemoryError::QueryFailed(format!("delete_namespace memory_vec delete failed: {e}"))
        })?;

        // Delete memories (FTS5 delete triggers fire; knowledge_edges cascade via FK)
        let mem_sql = format!(
            "DELETE FROM memories WHERE id IN ({placeholders})"
        );
        let params: Vec<&dyn rusqlite::types::ToSql> =
            ids.iter().map(|id| id as &dyn rusqlite::types::ToSql).collect();
        let deleted = tx.execute(&mem_sql, params.as_slice()).map_err(|e| {
            MemoryError::QueryFailed(format!("delete_namespace memories delete failed: {e}"))
        })? as u64;

        total_deleted += deleted;

        tx.commit().map_err(|e| {
            MemoryError::QueryFailed(format!("delete_namespace chunk commit failed: {e}"))
        })?;
    }

    // Remove the namespace registry entry (global metadata — other actors' memories
    // in this namespace path continue to exist, just in an unregistered namespace)
    self.execute("DELETE FROM namespaces WHERE name = ?1", [name])
        .map_err(|e| {
            MemoryError::QueryFailed(format!("delete_namespace registry delete failed: {e}"))
        })?;

    tracing::info!(namespace = name, actor_id, memories_deleted = total_deleted, "namespace deleted");
    Ok(total_deleted)
}
```

**Chunked delete rationale**: Each chunk of `NAMESPACE_DELETE_CHUNK_SIZE` (500) rows is committed independently. This releases the write between batches, allowing other tool calls to proceed. The outer loop exits when no IDs remain.

**Cascade behavior**: `knowledge_edges` has `ON DELETE CASCADE` on both `from_memory_id` and `to_memory_id`. Deleting memories in the namespace will automatically remove all edges referencing those memories. No explicit edge deletion is needed.

**FTS5 sync**: `memory_fts` uses content-sync triggers (`memory_fts_delete` AFTER DELETE ON memories). Deleting via the `id IN (...)` form fires the trigger for each row, keeping FTS5 in sync.

**memory_vec sync**: `memory_vec` has no FK or trigger. We explicitly delete its rows first (before the memories delete), using the same ID set. Deleted first because if the memories delete succeeds but memory_vec delete fails, orphan vec rows result — deleting vec first means on retry we re-collect IDs and re-attempt cleanly.

**Registry entry**: The namespace registry row is deleted after all memory chunks are processed. If the process dies after all chunks but before deleting the registry row, a re-run of `delete_namespace` will find zero memories to delete (loop exits immediately), then delete the registry entry — correct behavior.

---

## Validation Rules

```rust
// src/namespaces.rs

/// Validate a namespace name. Also called from memories.rs when validating the
/// namespace field on memory inserts, so both paths enforce the same rules.
pub fn validate_namespace_name(name: &str) -> Result<(), MemoryError> {
    if name.is_empty() {
        return Err(MemoryError::InvalidInput("namespace name must not be empty".into()));
    }
    if name.len() > MAX_NAMESPACE_LEN {
        return Err(MemoryError::InvalidInput(format!(
            "namespace name exceeds maximum length of {MAX_NAMESPACE_LEN} bytes (UTF-8)"
        )));
    }
    // Reject null bytes and ASCII control characters (blocks log/terminal injection).
    // We do NOT enforce a strict charset allowlist to stay compatible with AgentCore-style
    // paths (which allow '/', '-', '.', ':', '@', emoji, etc.). Control chars are the
    // practical security boundary; printable Unicode is permitted intentionally.
    if name.bytes().any(|b| b == 0x00 || b < 0x20 || b == 0x7F) {
        return Err(MemoryError::InvalidInput(
            "namespace name must not contain control characters or null bytes".into(),
        ));
    }
    Ok(())
}

fn validate_description(description: &str) -> Result<(), MemoryError> {
    if description.len() > MAX_DESCRIPTION_LEN {
        return Err(MemoryError::InvalidInput(
            format!("description exceeds maximum length of {MAX_DESCRIPTION_LEN} bytes")
        ));
    }
    Ok(())
}

fn validate_prefix(prefix: &str) -> Result<(), MemoryError> {
    if prefix.is_empty() {
        return Err(MemoryError::InvalidInput("prefix must not be empty".into()));
    }
    if prefix.len() > MAX_NAMESPACE_LEN {
        return Err(MemoryError::InvalidInput(
            format!("prefix exceeds maximum length of {MAX_NAMESPACE_LEN} bytes")
        ));
    }
    Ok(())
}
```

---

## Business Logic Functions

```rust
// src/namespaces.rs

pub fn create_namespace(
    db: &dyn Db,
    params: &CreateNamespaceParams<'_>,
) -> Result<Namespace, MemoryError> {
    validate_namespace_name(params.name)?;
    if let Some(desc) = params.description {
        validate_description(desc)?;
    }
    db.create_namespace(params.name, params.description)
}

pub fn list_namespaces(
    db: &dyn Db,
    params: &ListNamespacesParams<'_>,
) -> Result<Vec<Namespace>, MemoryError> {
    if let Some(prefix) = params.prefix {
        validate_prefix(prefix)?;
    }
    let clamped = ListNamespacesParams {
        limit: params.limit.clamp(1, MAX_PAGE_LIMIT),
        ..*params
    };
    db.list_namespaces(&clamped)
}

pub fn delete_namespace(
    db: &dyn Db,
    actor_id: &str,
    name: &str,
) -> Result<u64, MemoryError> {
    validate_non_empty(actor_id, "actor_id")?;
    validate_namespace_name(name)?;
    db.delete_namespace(actor_id, name)
}
```

---

## MCP Tool Interfaces

### Param structs (in `tools.rs`)

Note: name these to avoid collision with business-logic structs — follow the existing convention
(`ListMemoriesToolParams`, `GetEventsToolParams`):

```rust
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateNamespaceToolParams {
    /// Namespace path, e.g. "/user/alice/preferences". Up to 512 bytes (UTF-8).
    /// Must not contain control characters.
    #[schemars(length(max = 512))]
    name: String,
    #[serde(default)]
    #[schemars(length(max = 1024))]
    description: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListNamespacesToolParams {
    /// If provided, return only namespaces whose name starts with this prefix.
    #[serde(default)]
    prefix: Option<String>,
    #[serde(default)]
    limit: Option<u32>,
    #[serde(default)]
    offset: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DeleteNamespaceToolParams {
    actor_id: String,
    name: String,
}
```

### Tool handlers (in `MemoryServer` impl block)

```rust
#[tool(
    name = "memory.create_namespace",
    description = "Register a namespace with optional description. Idempotent — if the namespace already exists, returns the existing entry unchanged. Namespace names are UTF-8 strings up to 512 bytes (e.g., '/user/alice/preferences'). Must not contain control characters."
)]
pub async fn create_namespace(
    &self,
    Parameters(params): Parameters<CreateNamespaceToolParams>,
) -> Result<String, String> {
    self.run(move |mgr| {
        let db = mgr.db()?;
        let p = namespaces::CreateNamespaceParams {
            name: &params.name,
            description: params.description.as_deref(),
        };
        let ns = namespaces::create_namespace(db, &p)?;
        Ok(serde_json::json!({ "namespace": ns }))
    })
    .await
}

#[tool(
    name = "memory.list_namespaces",
    description = "List registered namespaces ordered alphabetically. Only namespaces explicitly created via memory.create_namespace are returned — not all namespaces referenced by memories. Supports optional prefix filter and limit/offset pagination."
)]
pub async fn list_namespaces(
    &self,
    Parameters(params): Parameters<ListNamespacesToolParams>,
) -> Result<String, String> {
    self.run(move |mgr| {
        let db = mgr.db()?;
        let p = namespaces::ListNamespacesParams {
            prefix: params.prefix.as_deref(),
            limit: params.limit.unwrap_or(DEFAULT_PAGE_LIMIT),
            offset: params.offset.unwrap_or(0),
        };
        let list = namespaces::list_namespaces(db, &p)?;
        Ok(serde_json::json!({ "namespaces": list }))
    })
    .await
}

#[tool(
    name = "memory.delete_namespace",
    description = "Delete all memories belonging to actor_id in the named namespace, clean up their vector rows, and remove the namespace registry entry. Scoped to actor_id — other actors' memories in the same namespace path are not affected. Deletes in chunks of 500 to avoid blocking the server. Returns not_found if the namespace is not registered."
)]
pub async fn delete_namespace(
    &self,
    Parameters(params): Parameters<DeleteNamespaceToolParams>,
) -> Result<String, String> {
    self.run(move |mgr| {
        let db = mgr.db()?;
        let memories_deleted = namespaces::delete_namespace(db, &params.actor_id, &params.name)?;
        Ok(serde_json::json!({ "deleted": true, "memories_deleted": memories_deleted }))
    })
    .await
}
```

### Response shapes

| Tool | Success response |
|------|-----------------|
| `memory.create_namespace` | `{"namespace": {"name": "...", "description": "...", "created_at": "..."}}` |
| `memory.list_namespaces` | `{"namespaces": [{...}, ...]}` |
| `memory.delete_namespace` | `{"deleted": true, "memories_deleted": 42}` |

### Error mapping in `run()`

`MemoryError::NotFound` → `"not_found"` (already handled by the existing `run()` match arm)

---

## Error Handling

| Scenario | Error | MCP code |
|----------|-------|----------|
| Empty or too-long name | `InvalidInput` | `invalid_input` |
| Null byte or control char in name | `InvalidInput` | `invalid_input` |
| Description too long | `InvalidInput` | `invalid_input` |
| Empty prefix | `InvalidInput` | `invalid_input` |
| Empty actor_id | `InvalidInput` | `invalid_input` |
| `delete_namespace` on unknown name | `NotFound` | `not_found` |
| SQLite failure | `QueryFailed` | `internal` |

---

## Implementation Plan

### Task 1 — Data types, Db trait, and Db implementation

**Files**: `src/namespaces.rs` (new), `src/db.rs`

1. Create `src/namespaces.rs` with:
   - `use` imports: `serde`, `crate::db::Db`, `crate::error::MemoryError`, `crate::events::{MAX_PAGE_LIMIT, DEFAULT_PAGE_LIMIT}`, `crate::memories::MAX_NAMESPACE_LEN`
   - `pub const MAX_DESCRIPTION_LEN: usize = 1_024;`
   - `pub struct Namespace { pub name, pub description, pub created_at }`
   - `pub struct CreateNamespaceParams<'a>`
   - `pub struct ListNamespacesParams<'a>`

2. In `src/db.rs`:
   - Add `use crate::namespaces::{Namespace, ListNamespacesParams};` to the imports
   - Remove the commented-out namespace placeholder block
   - Add `fn create_namespace`, `fn list_namespaces`, `fn delete_namespace` to the `Db` trait (signatures as specified above)
   - Add `fn row_to_namespace(row: &rusqlite::Row<'_>) -> rusqlite::Result<Namespace>` private helper
   - Implement all three methods on `impl Db for Connection`

**Acceptance criteria**: `cargo check` passes.

---

### Task 2 — Business logic and unit tests

**Files**: `src/namespaces.rs`

1. Add private validation functions: `validate_name`, `validate_description`, `validate_prefix`
2. Add public business logic functions: `create_namespace`, `list_namespaces`, `delete_namespace`
3. Add `#[cfg(test)]` module with tests (see Test Plan below)

**Acceptance criteria**: `cargo test -p local-memory-mcp namespaces` passes, all tests green.

---

### Task 3 — MCP tool handlers and module wiring

**Files**: `src/tools.rs`, `src/lib.rs`

1. In `src/lib.rs`: add `pub mod namespaces;`
2. In `src/tools.rs`:
   - Add `use crate::namespaces;` and `use crate::events::DEFAULT_PAGE_LIMIT;` (if not already imported)
   - Add `CreateNamespaceParams`, `ListNamespacesParams`, `DeleteNamespaceParams` structs
   - Add `create_namespace`, `list_namespaces`, `delete_namespace` methods to `MemoryServer` inside the `#[tool_router]` impl block
3. Add `#[cfg(test)]` cases for tool-layer behavior (see Test Plan)

**Acceptance criteria**: `cargo check` passes, `cargo test` passes.

---

### Task 4 — Final verification

1. `cargo check`
2. `cargo test -- --test-output immediate`
3. `cargo clippy -- -D warnings`

Fix any warnings or test failures before marking done.

**Acceptance criteria**: All three commands exit 0 with no warnings.

---

## DAG

```
Task 1 (namespaces.rs types + db.rs trait + db.rs impl)
    │
    ▼
Task 2 (namespaces.rs business logic + tests)
    │
    ▼
Task 3 (tools.rs handlers + lib.rs wiring)
    │
    ▼
Task 4 (cargo check + test + clippy)
```

All tasks are sequential. Task 2 requires the Db trait from Task 1. Task 3 requires the public functions from Task 2. Task 4 verifies the whole stack.

---

## Test Plan

### Task 2 tests (namespaces.rs unit tests)

Use the same `open_db()` helper pattern from other modules:

```rust
fn open_db() -> (TempDir, rusqlite::Connection) {
    let dir = TempDir::new().unwrap();
    let conn = db::open(&dir.path().join("test.db")).unwrap();
    (dir, conn)
}
```

| Test | What it covers |
|------|---------------|
| `test_validate_name_empty` | `validate_namespace_name("")` → `InvalidInput` |
| `test_validate_name_too_long` | name of 513 bytes → `InvalidInput` |
| `test_validate_name_null_byte` | name with `\0` → `InvalidInput` |
| `test_validate_name_control_char` | name with `\n`, `\t`, `\x1b` → `InvalidInput` |
| `test_validate_name_del_char` | name with `\x7f` → `InvalidInput` |
| `test_validate_description_too_long` | description of 1025 bytes → `InvalidInput` |
| `test_validate_prefix_empty` | `validate_prefix("")` → `InvalidInput` |
| `test_create_namespace_basic` | create → returns Namespace with correct fields |
| `test_create_namespace_idempotent` | create twice with different description → second call returns first entry (description unchanged) |
| `test_create_namespace_no_description` | `description: None` → stored and returned as None |
| `test_list_namespaces_empty` | no namespaces → returns empty vec |
| `test_list_namespaces_ordered` | insert "b", "a", "c" → returned in alphabetical order |
| `test_list_namespaces_prefix` | prefix "/user" → only matching namespaces returned |
| `test_list_namespaces_prefix_escaping` | prefix containing `%` or `_` or `\` → treated as literals, not wildcards |
| `test_list_namespaces_pagination` | limit=2, offset=1 → correct slice |
| `test_delete_namespace_not_found` | delete unknown name → `NotFound` |
| `test_delete_namespace_no_memories` | create then delete (no memories) → 0 memories_deleted |
| `test_delete_namespace_with_memories` | create, store 3 memories for actor1, delete → 3 memories_deleted, memories gone |
| `test_delete_namespace_actor_scoped` | store memories for actor1 and actor2 in same namespace, delete for actor1 → only actor1's memories removed, actor2's intact |
| `test_delete_namespace_exact_match_only` | create "/a" and "/a/b" with memories in both, delete "/a" → "/a/b" actor's memories untouched |
| `test_delete_namespace_cascades_edges` | store 2 memories in ns, add edge, delete ns → edge gone |
| `test_delete_namespace_cleans_memory_vec` | store memory with embedding in ns, delete ns → memory_vec row removed |

### Task 3 tests (tools.rs unit tests)

Add to the existing `#[cfg(test)] mod tests` in `tools.rs`:

| Test | What it covers |
|------|---------------|
| `test_tool_create_namespace` | end-to-end tool call returns `{"namespace": {...}}` |
| `test_tool_list_namespaces_empty` | returns `{"namespaces": []}` |
| `test_tool_delete_namespace_not_found` | returns error JSON with `"code":"not_found"` |

---

## Sub-Agent Instructions

### Before writing any code

Confirm all of the following:
- `src/namespaces.rs` does not exist yet
- `src/lib.rs` does not yet declare `pub mod namespaces`
- `src/db.rs` has the commented-out placeholder block `// -- Namespaces (Component 7) --` (lines ~379-382)
- `src/tools.rs` does not yet contain `create_namespace`, `list_namespaces`, or `delete_namespace` tool methods

If any of these are already present, stop and report.

---

### Task 1 instructions

**Step 1.1 — Create `src/namespaces.rs`**

Create the file with exactly these contents (data types only — no business logic yet):

```rust
use serde::{Deserialize, Serialize};

use crate::db::Db;
use crate::error::MemoryError;
use crate::events::{DEFAULT_PAGE_LIMIT, MAX_PAGE_LIMIT};
use crate::memories::MAX_NAMESPACE_LEN;

pub const MAX_DESCRIPTION_LEN: usize = 1_024;
pub const NAMESPACE_DELETE_CHUNK_SIZE: usize = 500;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Namespace {
    pub name: String,
    pub description: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct CreateNamespaceParams<'a> {
    pub name: &'a str,
    pub description: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct ListNamespacesParams<'a> {
    pub prefix: Option<&'a str>,
    pub limit: u32,
    pub offset: u32,
}
```

**Step 1.2 — Update `src/db.rs` imports**

Add to the existing `use crate::...` import block at the top of `db.rs`:

```rust
use crate::namespaces::{ListNamespacesParams, Namespace};
```

**Step 1.3 — Add `row_to_namespace` helper to `db.rs`**

Add this private function alongside `row_to_event`, `row_to_memory`, etc.:

```rust
fn row_to_namespace(row: &rusqlite::Row<'_>) -> rusqlite::Result<Namespace> {
    Ok(Namespace {
        name: row.get(0)?,
        description: row.get(1)?,
        created_at: row.get(2)?,
    })
}
```

**Step 1.4 — Add namespace methods to the `Db` trait**

Remove the existing comment block:
```rust
// -- Namespaces (Component 7) --
// fn create_namespace(name: &str, description: Option<&str>) -> Result<(), MemoryError>;
// fn list_namespaces(prefix: Option<&str>) -> Result<Vec<Namespace>, MemoryError>;
// fn delete_namespace(name: &str) -> Result<u64, MemoryError>; // returns count of deleted memories
```

Replace with:
```rust
// -- Namespaces (Component 8) --

/// Insert a namespace. Idempotent: ON CONFLICT(name) DO NOTHING, then SELECT.
/// Description is NOT updated if namespace already exists.
fn create_namespace(
    &self,
    name: &str,
    description: Option<&str>,
) -> Result<Namespace, MemoryError>;

/// List registered namespaces. Optional LIKE prefix filter (wildcards escaped).
/// Ordered by name ASC. Limit/offset for pagination.
fn list_namespaces(
    &self,
    params: &ListNamespacesParams<'_>,
) -> Result<Vec<Namespace>, MemoryError>;

/// Delete memories for actor_id in the named namespace (chunked), clean up memory_vec,
/// and remove the namespace registry entry.
/// Returns count of memories deleted. Returns NotFound if namespace not registered.
fn delete_namespace(&self, actor_id: &str, name: &str) -> Result<u64, MemoryError>;
```

**Step 1.5 — Implement the three methods on `impl Db for Connection`**

Add these three methods to the `impl Db for Connection` block in `db.rs`. Place them at the end of the impl block, before the closing `}`.

```rust
fn create_namespace(
    &self,
    name: &str,
    description: Option<&str>,
) -> Result<Namespace, MemoryError> {
    self.execute(
        "INSERT INTO namespaces(name, description) VALUES(?1, ?2) ON CONFLICT(name) DO NOTHING",
        rusqlite::params![name, description],
    )
    .map_err(|e| {
        tracing::error!("create_namespace insert failed: {e}");
        MemoryError::QueryFailed("failed to create namespace".into())
    })?;

    self.query_row(
        "SELECT name, description, created_at FROM namespaces WHERE name = ?1",
        [name],
        row_to_namespace,
    )
    .map_err(|e| {
        tracing::error!("create_namespace select failed: {e}");
        MemoryError::QueryFailed("failed to retrieve namespace after insert".into())
    })
}

fn list_namespaces(
    &self,
    params: &ListNamespacesParams<'_>,
) -> Result<Vec<Namespace>, MemoryError> {
    let mut stmt;
    let rows: Vec<Namespace>;

    if let Some(prefix) = params.prefix {
        let pattern = format!("{}%", escape_like(prefix));
        stmt = self.prepare(
            "SELECT name, description, created_at FROM namespaces \
             WHERE name LIKE ?1 ESCAPE '\\' \
             ORDER BY name ASC LIMIT ?2 OFFSET ?3",
        )
        .map_err(|e| MemoryError::QueryFailed(format!("list_namespaces prepare failed: {e}")))?;
        rows = stmt
            .query_map(
                rusqlite::params![pattern, params.limit, params.offset],
                row_to_namespace,
            )
            .map_err(|e| MemoryError::QueryFailed(format!("list_namespaces query failed: {e}")))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| MemoryError::QueryFailed(format!("list_namespaces row failed: {e}")))?;
    } else {
        stmt = self.prepare(
            "SELECT name, description, created_at FROM namespaces \
             ORDER BY name ASC LIMIT ?1 OFFSET ?2",
        )
        .map_err(|e| MemoryError::QueryFailed(format!("list_namespaces prepare failed: {e}")))?;
        rows = stmt
            .query_map(
                rusqlite::params![params.limit, params.offset],
                row_to_namespace,
            )
            .map_err(|e| MemoryError::QueryFailed(format!("list_namespaces query failed: {e}")))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| MemoryError::QueryFailed(format!("list_namespaces row failed: {e}")))?;
    }

    Ok(rows)
}

fn delete_namespace(&self, name: &str) -> Result<u64, MemoryError> {
    let exists: bool = self
        .query_row(
            "SELECT 1 FROM namespaces WHERE name = ?1",
            [name],
            |_| Ok(true),
        )
        .optional()
        .map_err(|e| MemoryError::QueryFailed(format!("delete_namespace existence check failed: {e}")))?
        .unwrap_or(false);

    if !exists {
        return Err(MemoryError::NotFound(name.to_string()));
    }

    // unchecked_transaction is safe here: this code runs inside spawn_blocking,
    // which holds the StoreManager mutex. No other thread accesses the connection.
    let tx = self.unchecked_transaction().map_err(|e| {
        MemoryError::QueryFailed(format!("delete_namespace begin tx failed: {e}"))
    })?;

    // Delete memories (FTS5 delete triggers fire per row, keeping memory_fts in sync).
    // knowledge_edges cascade via ON DELETE CASCADE FK on both from_memory_id and to_memory_id.
    // Note: memory_vec rows for deleted memories are NOT cleaned up here — they become
    // orphans but will never be matched in vector searches (no corresponding memory row).
    let memories_deleted = tx
        .execute("DELETE FROM memories WHERE namespace = ?1", [name])
        .map_err(|e| {
            MemoryError::QueryFailed(format!("delete_namespace memories delete failed: {e}"))
        })? as u64;

    tx.execute("DELETE FROM namespaces WHERE name = ?1", [name])
        .map_err(|e| {
            MemoryError::QueryFailed(format!("delete_namespace namespace delete failed: {e}"))
        })?;

    tx.commit().map_err(|e| {
        MemoryError::QueryFailed(format!("delete_namespace commit failed: {e}"))
    })?;

    Ok(memories_deleted)
}
```

**Step 1.6 — Verify Task 1**

Run `cargo check`. Fix any compilation errors before proceeding to Task 2.

---

### Task 2 instructions

**Step 2.1 — Add business logic to `src/namespaces.rs`**

Append to `src/namespaces.rs` (after the data type definitions):

```rust
// --- Validation ---

fn validate_non_empty(value: &str, field: &str) -> Result<(), MemoryError> {
    if value.is_empty() {
        return Err(MemoryError::InvalidInput(format!("{field} must not be empty")));
    }
    Ok(())
}

/// Validate a namespace name. Exported so memories.rs can call it when validating
/// the namespace field on memory inserts, ensuring both paths enforce the same rules.
pub fn validate_namespace_name(name: &str) -> Result<(), MemoryError> {
    if name.is_empty() {
        return Err(MemoryError::InvalidInput("namespace name must not be empty".into()));
    }
    if name.len() > MAX_NAMESPACE_LEN {
        return Err(MemoryError::InvalidInput(format!(
            "namespace name exceeds maximum length of {MAX_NAMESPACE_LEN} bytes (UTF-8)"
        )));
    }
    // Reject null bytes and ASCII control characters (blocks log/terminal injection).
    // We do NOT enforce a strict charset allowlist to stay compatible with AgentCore-style
    // paths (which allow '/', '-', '.', ':', '@', emoji, etc.). Control chars are the
    // practical security boundary; printable Unicode is permitted intentionally.
    if name.bytes().any(|b| b == 0x00 || b < 0x20 || b == 0x7F) {
        return Err(MemoryError::InvalidInput(
            "namespace name must not contain control characters or null bytes".into(),
        ));
    }
    Ok(())
}

fn validate_description(description: &str) -> Result<(), MemoryError> {
    if description.len() > MAX_DESCRIPTION_LEN {
        return Err(MemoryError::InvalidInput(format!(
            "description exceeds maximum length of {MAX_DESCRIPTION_LEN} bytes"
        )));
    }
    Ok(())
}

fn validate_prefix(prefix: &str) -> Result<(), MemoryError> {
    if prefix.is_empty() {
        return Err(MemoryError::InvalidInput("prefix must not be empty".into()));
    }
    if prefix.len() > MAX_NAMESPACE_LEN {
        return Err(MemoryError::InvalidInput(format!(
            "prefix exceeds maximum length of {MAX_NAMESPACE_LEN} bytes"
        )));
    }
    Ok(())
}

// --- Business logic ---

pub fn create_namespace(
    db: &dyn Db,
    params: &CreateNamespaceParams<'_>,
) -> Result<Namespace, MemoryError> {
    validate_namespace_name(params.name)?;
    if let Some(desc) = params.description {
        validate_description(desc)?;
    }
    db.create_namespace(params.name, params.description)
}

pub fn list_namespaces(
    db: &dyn Db,
    params: &ListNamespacesParams<'_>,
) -> Result<Vec<Namespace>, MemoryError> {
    if let Some(prefix) = params.prefix {
        validate_prefix(prefix)?;
    }
    let clamped = ListNamespacesParams {
        limit: params.limit.clamp(1, MAX_PAGE_LIMIT),
        ..*params
    };
    db.list_namespaces(&clamped)
}

pub fn delete_namespace(db: &dyn Db, actor_id: &str, name: &str) -> Result<u64, MemoryError> {
    validate_non_empty(actor_id, "actor_id")?;
    validate_namespace_name(name)?;
    db.delete_namespace(actor_id, name)
}
```

**Step 2.2 — Update `src/memories.rs` to use `validate_namespace_name`**

In `src/memories.rs`, inside `validate_insert_memory_params`, replace the inline namespace validation:
```rust
// Before:
if let Some(ns) = params.namespace {
    validate_non_empty(ns, "namespace")?;
    validate_max_len(ns, MAX_NAMESPACE_LEN, "namespace")?;
}
```
With:
```rust
// After:
if let Some(ns) = params.namespace {
    crate::namespaces::validate_namespace_name(ns)?;
}
```

This ensures that memory inserts reject the same invalid namespace strings that `create_namespace` would reject. Remove `MAX_NAMESPACE_LEN` from the `use crate::db::{Db, EMBEDDING_DIM};` import if it was imported solely for the old validation (check carefully — it may still be used for error messages).

**Step 2.4 — Add tests to `src/namespaces.rs`**

Append the full test module as specified in the Test Plan above. Use `tempfile::TempDir` and `crate::db::open`. All 22 test cases must be present and pass.

**Step 2.5 — Verify Task 2**

Run `cargo test namespaces`. All tests must pass.

---

### Task 3 instructions

**Step 3.1 — Add `pub mod namespaces` to `src/lib.rs`**

Add `pub mod namespaces;` to `src/lib.rs` alongside the existing module declarations.

**Step 3.2 — Update imports in `src/tools.rs`**

Add to the existing `use crate::...` block:
```rust
use crate::namespaces;
use crate::events::DEFAULT_PAGE_LIMIT;  // add only if not already present
```

**Step 3.3 — Add param structs to `src/tools.rs`**

Add the three param structs `CreateNamespaceToolParams`, `ListNamespacesToolParams`, and `DeleteNamespaceToolParams` to the param structs section. Use the `*ToolParams` suffix to avoid conflicts with the identically-named structs in `namespaces.rs` (consistent with `ListMemoriesToolParams` and `GetEventsToolParams`).

**Step 3.4 — Add tool handlers to `src/tools.rs`**

Add the three `#[tool(...)]` methods to the `MemoryServer` impl block inside `#[tool_router(server_handler)]`. Place them after the `delete_store` handler and before the graph tools section.

**Step 3.5 — Add tool-layer tests**

In the existing `#[cfg(test)] mod tests` in `tools.rs`, add the 3 tests from the Test Plan.

**Step 3.6 — Verify Task 3**

Run `cargo check` and `cargo test`. All tests must pass.

---

### Task 4 instructions

Run in order:
1. `cargo check` — must exit 0
2. `cargo test -- --test-output immediate` — all tests must pass
3. `cargo clippy -- -D warnings` — must exit 0 with no warnings

Fix any issues found. Do not suppress warnings with `#[allow(...)]` without justification.
