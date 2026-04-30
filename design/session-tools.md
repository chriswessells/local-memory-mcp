# Component 6: Session Tools — Detailed Design

## Scope

Checkpoint and branch management for session navigation. This component adds:

1. **Data types** — `Checkpoint`, `Branch`, `InsertCheckpointParams`, `InsertBranchParams`, `ListCheckpointsParams`, `ListBranchesParams` in `sessions.rs`
2. **Db trait methods** — 4 methods added to the `Db` trait in `db.rs`
3. **Business logic** — validation + delegation layer in `sessions.rs`
4. **MCP tools** — 4 new tools added to the `#[tool_router]` block in `tools.rs`

This component does NOT add new schema tables (checkpoints and branches were defined in Component 1).

---

## Design Review Resolutions

### High

- **A1**: DESIGN.md Session management tool table updated to show authoritative signatures with `actor_id` on `memory.create_branch`, `memory.list_checkpoints`, and `memory.list_branches`. Single source of truth is this document; DESIGN.md table updated accordingly.
- **R1**: `create_branch` wrapped in `unchecked_transaction` (same as `create_checkpoint`). Eliminates TOCTOU window on parent_branch_id check + INSERT, and prevents dangling references if the process crashes between the verification read and the write.

### Medium (logged to TODO backlog)

- S1: Consistency — `create_branch` now in transaction (resolved as High)
- S2: Metadata depth bound — use same validation as events.rs (backlog)
- A2: Add pagination (limit/offset) to `list_branches` — added to this design below
- A3: Document orphan-branch behavior on event TTL expiry (backlog, needs ADR decision)
- A4: Document EXCLUSIVE lock dependency in `create_branch` comment (backlog)
- A5: Parent branch actor cross-check (backlog — session_id is assumed actor-unique in practice)
- A6: Metadata wire format as string vs JSON object — matches events.rs existing pattern (backlog)
- M1: Shared validation module `src/validation.rs` (pre-existing backlog item)
- M2: Prune tests to Critical/High paths only
- M3: Named column access in row mapping (backlog)
- R2: Map SQLITE_FULL to MemoryError::DiskFull (backlog)
- R3: Document unchecked_transaction commit flow (addressed in sub-agent instructions below)
- R4: Orphan branches on delete_expired — document behavior (backlog, linked to A3)
- I1: Use `c.is_control()` over byte < 0x20 (updated in validation section below)
- I2: Dotted tool names — consistent with all other tools, already confirmed working in Kiro
- I3: Add `id` as tiebreaker on ORDER BY clauses (applied below)

### Low (logged to TODO backlog)

- S3: UUID format validation — document that length-only is intentional
- S4: serde_json error never propagated as-is (enforced in sub-agent instructions)
- S5: Logging policy — no user content at any log level
- A7: Tool naming verb consistency (memory.create_checkpoint mirrors AgentCore convention)
- M4: Consolidate MAX_CHECKPOINT_NAME_LEN / MAX_BRANCH_NAME_LEN to shared constant
- M5: Add doc comment to sessions.rs clarifying scope
- R5: Add tracing spans (backlog)
- R6: Map SQLITE_BUSY to MemoryError::StoreLocked (backlog)
- I4: Future migration footnote added to scope section
- I6: Verify branch_id contract with memory.create_event (confirmed: events.branch_id = branches.id UUID)

---

## Context

### Schema (from Component 1)

```sql
CREATE TABLE IF NOT EXISTS checkpoints (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    actor_id TEXT NOT NULL,
    name TEXT NOT NULL,
    event_id TEXT NOT NULL,        -- the event this checkpoint points to
    metadata TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_checkpoint_name ON checkpoints(session_id, name);

CREATE TABLE IF NOT EXISTS branches (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    name TEXT,                     -- optional human-readable branch name
    parent_branch_id TEXT,         -- NULL = forked from main
    root_event_id TEXT NOT NULL,   -- the event from which this branch forks
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
CREATE INDEX IF NOT EXISTS idx_branches_session ON branches(session_id);
```

Note: `branches` has no `actor_id` column. Actor scoping for branches is enforced by verifying that `root_event_id` belongs to `actor_id` (via the `events` table) both at creation time and at query time via JOIN.

### Commented stubs in db.rs (to be replaced)

```rust
// -- Sessions (Component 6) --
// fn create_checkpoint(...) -> Result<String, MemoryError>;
// fn create_branch(...) -> Result<String, MemoryError>;
// fn list_checkpoints(session_id: &str) -> Result<Vec<Checkpoint>, MemoryError>;
// fn list_branches(session_id: &str) -> Result<Vec<Branch>, MemoryError>;
```

---

## Constants

```rust
// src/sessions.rs
pub const MAX_CHECKPOINT_NAME_LEN: usize = 256;
pub const MAX_BRANCH_NAME_LEN: usize = 256;
pub const DEFAULT_CHECKPOINT_LIMIT: u32 = 100;
pub const MAX_CHECKPOINT_LIMIT: u32 = 1000;
```

Reuse existing constants from `events.rs`: `MAX_ACTOR_ID_LEN`, `MAX_SESSION_ID_LEN`, `MAX_METADATA_SIZE`.

---

## Data Types

```rust
// src/sessions.rs

use serde::{Deserialize, Serialize};

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
```

---

## Db Trait Methods

Replace the commented stubs in `db.rs` with:

```rust
// -- Sessions (Component 6) --

/// Create a checkpoint pointing to a specific event within a session.
/// Precondition: params must be pre-validated. event_id must exist in events for (actor_id, session_id).
/// Returns InvalidInput if (session_id, name) already exists.
fn create_checkpoint(
    &self,
    params: &InsertCheckpointParams<'_>,
) -> Result<Checkpoint, MemoryError>;

/// Fork a conversation by creating a branch from a specific event.
/// Precondition: params must be pre-validated. root_event_id must exist in events for actor_id.
/// If parent_branch_id provided, it must exist with matching session_id.
fn create_branch(
    &self,
    params: &InsertBranchParams<'_>,
) -> Result<Branch, MemoryError>;

/// List checkpoints for a session, scoped to actor. Ordered by created_at ASC.
fn list_checkpoints(
    &self,
    params: &ListCheckpointsParams<'_>,
) -> Result<Vec<Checkpoint>, MemoryError>;

/// List branches for a session, scoped to actor via root_event_id JOIN. Ordered by created_at ASC, id ASC.
fn list_branches(
    &self,
    params: &ListBranchesParams<'_>,
) -> Result<Vec<Branch>, MemoryError>;
```

---

## SQL Implementation

### create_checkpoint

Two steps within a single `unchecked_transaction`:

**Step 1 — verify event belongs to actor+session:**
```sql
SELECT COUNT(*) FROM events
WHERE id = :event_id AND actor_id = :actor_id AND session_id = :session_id
```
If count = 0: return `MemoryError::NotFound("event not found for this actor and session".into())`.

**Step 2 — insert checkpoint:**
```sql
INSERT INTO checkpoints (id, session_id, actor_id, name, event_id, metadata)
VALUES (:id, :session_id, :actor_id, :name, :event_id, :metadata)
RETURNING id, session_id, actor_id, name, event_id, metadata, created_at
```

On `SQLITE_CONSTRAINT_UNIQUE` (rusqlite `ErrorCode::ConstraintViolation` with message containing "idx_checkpoint_name"):
- Return `MemoryError::InvalidInput("a checkpoint with this name already exists for the session".into())`

Do NOT echo user-supplied name in the error message.

Use `INSERT...RETURNING` to atomically get the stored row including `created_at`.

### create_branch

Three steps wrapped in `unchecked_transaction` (same pattern as `create_checkpoint`):

**Step 1 — verify root event belongs to actor:**
```sql
SELECT COUNT(*) FROM events WHERE id = :root_event_id AND actor_id = :actor_id
```
If count = 0: return `MemoryError::NotFound("root event not found for this actor".into())`.

**Step 2 (conditional) — verify parent_branch_id if provided:**
```sql
SELECT COUNT(*) FROM branches WHERE id = :parent_branch_id AND session_id = :session_id
```
If count = 0: return `MemoryError::NotFound("parent branch not found for this session".into())`.

**Step 3 — insert branch:**
```sql
INSERT INTO branches (id, session_id, name, parent_branch_id, root_event_id)
VALUES (:id, :session_id, :name, :parent_branch_id, :root_event_id)
RETURNING id, session_id, name, parent_branch_id, root_event_id, created_at
```

Wrapping in `unchecked_transaction` prevents a TOCTOU window where the parent branch could be deleted between the check and the INSERT, and ensures the INSERT is either committed or rolled back atomically. The transaction must be explicitly committed after a successful INSERT; if any step returns an error, the transaction is dropped without commit (implicit rollback).

### list_checkpoints

```sql
SELECT id, session_id, actor_id, name, event_id, metadata, created_at
FROM checkpoints
WHERE actor_id = :actor_id AND session_id = :session_id
ORDER BY created_at ASC, id ASC
LIMIT :limit OFFSET :offset
```

`id` as tiebreaker ensures deterministic ordering when two checkpoints share the same `created_at` second.

### list_branches

Actor scoping via JOIN to events (branches table has no actor_id):

```sql
SELECT b.id, b.session_id, b.name, b.parent_branch_id, b.root_event_id, b.created_at
FROM branches b
JOIN events e ON e.id = b.root_event_id AND e.actor_id = :actor_id
WHERE b.session_id = :session_id
ORDER BY b.created_at ASC, b.id ASC
LIMIT :limit OFFSET :offset
```

`id` as tiebreaker ensures deterministic ordering. Pagination added for consistency with `list_checkpoints` and to prevent unbounded results on pathological sessions with many branches.

---

## Input Validation (in sessions.rs, before calling Db)

### create_checkpoint validation

| Field | Rule |
|-------|------|
| `actor_id` | Non-empty, max `MAX_ACTOR_ID_LEN` (256) chars |
| `session_id` | Non-empty, max `MAX_SESSION_ID_LEN` (256) chars |
| `name` | Non-empty, max `MAX_CHECKPOINT_NAME_LEN` (256) chars, no control characters (`c.is_control()` — covers ASCII C0, DEL, and Unicode control block U+0080–U+009F) |
| `event_id` | Non-empty, max 36 chars (UUID format: 8-4-4-4-12) |
| `metadata` | If present: valid JSON object, max `MAX_METADATA_SIZE` (64 KB) bytes |

### create_branch validation

| Field | Rule |
|-------|------|
| `actor_id` | Non-empty, max `MAX_ACTOR_ID_LEN` (256) chars |
| `session_id` | Non-empty, max `MAX_SESSION_ID_LEN` (256) chars |
| `root_event_id` | Non-empty, max 36 chars |
| `name` | If present: non-empty, max `MAX_BRANCH_NAME_LEN` (256) chars, no control characters (`c.is_control()`) |
| `parent_branch_id` | If present: non-empty, max 36 chars |

### list_checkpoints / list_branches validation

| Field | Rule |
|-------|------|
| `actor_id` | Non-empty, max `MAX_ACTOR_ID_LEN` (256) chars |
| `session_id` | Non-empty, max `MAX_SESSION_ID_LEN` (256) chars |
| `limit` | Clamped to `1..=MAX_CHECKPOINT_LIMIT` (default `DEFAULT_CHECKPOINT_LIMIT`) |
| `offset` | No max, default 0 |

Error messages must not echo back user-supplied values.

---

## Business Logic Layer (sessions.rs)

```rust
pub fn create_checkpoint(
    db: &dyn Db,
    params: &InsertCheckpointParams<'_>,
) -> Result<Checkpoint, MemoryError> {
    validate_checkpoint_params(params)?;
    db.create_checkpoint(params)
}

pub fn create_branch(
    db: &dyn Db,
    params: &InsertBranchParams<'_>,
) -> Result<Branch, MemoryError> {
    validate_branch_params(params)?;
    db.create_branch(params)
}

pub fn list_checkpoints(
    db: &dyn Db,
    params: &ListCheckpointsParams<'_>,
) -> Result<Vec<Checkpoint>, MemoryError> {
    validate_non_empty(params.actor_id, "actor_id")?;
    validate_non_empty(params.session_id, "session_id")?;
    let limit = params.limit.clamp(1, MAX_CHECKPOINT_LIMIT);
    db.list_checkpoints(&ListCheckpointsParams { limit, ..*params })
}

pub fn list_branches(
    db: &dyn Db,
    params: &ListBranchesParams<'_>,
) -> Result<Vec<Branch>, MemoryError> {
    validate_non_empty(params.actor_id, "actor_id")?;
    validate_non_empty(params.session_id, "session_id")?;
    let limit = params.limit.clamp(1, MAX_CHECKPOINT_LIMIT);
    db.list_branches(&ListBranchesParams { limit, ..*params })
}
```

---

## MCP Tool Definitions

### Parameter structs (in tools.rs)

```rust
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CheckpointToolParams {
    actor_id: String,
    session_id: String,
    name: String,
    event_id: String,
    #[serde(default)]
    metadata: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct BranchToolParams {
    actor_id: String,
    session_id: String,
    root_event_id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    parent_branch_id: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListCheckpointsToolParams {
    actor_id: String,
    session_id: String,
    #[serde(default)]
    limit: Option<u32>,
    #[serde(default)]
    offset: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListBranchesToolParams {
    actor_id: String,
    session_id: String,
    #[serde(default)]
    limit: Option<u32>,
    #[serde(default)]
    offset: Option<u32>,
}
```

### Tool implementations (in `#[tool_router(server_handler)] impl MemoryServer`)

```rust
#[tool(
    name = "memory.create_checkpoint",
    description = "Create a named checkpoint at a specific event within a session. \
                   Checkpoints are named snapshots used for workflow resumption and \
                   conversation bookmarks. Name must be unique per session. Returns \
                   the created checkpoint object."
)]
pub async fn checkpoint(
    &self,
    Parameters(params): Parameters<CheckpointToolParams>,
) -> Result<String, String> {
    self.run(move |mgr| {
        let db = mgr.db()?;
        let p = sessions::InsertCheckpointParams {
            actor_id: &params.actor_id,
            session_id: &params.session_id,
            name: &params.name,
            event_id: &params.event_id,
            metadata: params.metadata.as_deref(),
        };
        sessions::create_checkpoint(db, &p)
            .map(|cp| serde_json::json!({ "checkpoint": cp }))
    })
    .await
}

#[tool(
    name = "memory.create_branch",
    description = "Fork a conversation by creating a branch from a specific event. \
                   Branches enable alternative conversation paths, message editing, \
                   and what-if scenarios. Returns the created branch object with its ID \
                   to use as branch_id in memory.create_event."
)]
pub async fn branch(
    &self,
    Parameters(params): Parameters<BranchToolParams>,
) -> Result<String, String> {
    self.run(move |mgr| {
        let db = mgr.db()?;
        let p = sessions::InsertBranchParams {
            actor_id: &params.actor_id,
            session_id: &params.session_id,
            root_event_id: &params.root_event_id,
            name: params.name.as_deref(),
            parent_branch_id: params.parent_branch_id.as_deref(),
        };
        sessions::create_branch(db, &p)
            .map(|br| serde_json::json!({ "branch": br }))
    })
    .await
}

#[tool(
    name = "memory.list_checkpoints",
    description = "List all checkpoints for a session, ordered by creation time. \
                   Returns an array of checkpoint objects with names and event IDs."
)]
pub async fn list_checkpoints(
    &self,
    Parameters(params): Parameters<ListCheckpointsToolParams>,
) -> Result<String, String> {
    self.run(move |mgr| {
        let db = mgr.db()?;
        let p = sessions::ListCheckpointsParams {
            actor_id: &params.actor_id,
            session_id: &params.session_id,
            limit: params.limit.unwrap_or(sessions::DEFAULT_CHECKPOINT_LIMIT),
            offset: params.offset.unwrap_or(0),
        };
        sessions::list_checkpoints(db, &p)
            .map(|cps| serde_json::json!({ "checkpoints": cps }))
    })
    .await
}

#[tool(
    name = "memory.list_branches",
    description = "List branches for a session, ordered by creation time. \
                   Returns an array of branch objects including their root event IDs \
                   and optional names. Use the returned branch id as branch_id in memory.create_event."
)]
pub async fn list_branches(
    &self,
    Parameters(params): Parameters<ListBranchesToolParams>,
) -> Result<String, String> {
    self.run(move |mgr| {
        let db = mgr.db()?;
        let p = sessions::ListBranchesParams {
            actor_id: &params.actor_id,
            session_id: &params.session_id,
            limit: params.limit.unwrap_or(sessions::DEFAULT_CHECKPOINT_LIMIT),
            offset: params.offset.unwrap_or(0),
        };
        sessions::list_branches(db, &p)
            .map(|brs| serde_json::json!({ "branches": brs }))
    })
    .await
}
```

### Notes on deviation from DESIGN.md

The original DESIGN.md spec for `memory.create_branch`, `memory.list_checkpoints`, and `memory.list_branches` did not include `actor_id`. This design adds `actor_id` to all four tools for actor-level authorization:
- Without `actor_id`, anyone knowing a `session_id` could list its checkpoints/branches or create branches on sessions they don't own.
- The `branches` table has no `actor_id` column — scoping at query time via JOIN requires it.
- This matches the pattern used consistently across all other tools in the server (every tool that operates on session data takes `actor_id`).

---

## Implementation Plan

| # | Task | Acceptance Criteria |
|---|------|-------------------|
| 1 | Add `Checkpoint`, `Branch`, param structs, and constants to `sessions.rs` | Structs compile; `#[derive(Debug, Clone, Serialize, Deserialize)]` on data types |
| 2 | Add 4 Db trait method signatures to `db.rs` (replace commented stubs) | Trait compiles; `_assert_object_safe` passes |
| 3 | Implement `create_checkpoint` for Connection | Test: creates checkpoint; returns NotFound for wrong event/actor; returns InvalidInput on name conflict |
| 4 | Implement `create_branch` for Connection | Test: creates branch; returns NotFound for missing root event or wrong actor; validates parent_branch_id |
| 5 | Implement `list_checkpoints` for Connection | Test: returns checkpoints in created_at ASC order; actor-scoped; pagination works |
| 6 | Implement `list_branches` for Connection | Test: returns branches in created_at ASC order; actor-scoped via JOIN; empty when actor doesn't match |
| 7 | Add validation + business logic functions in `sessions.rs` | Tests: rejects invalid inputs (empty fields, oversized, control chars, bad metadata JSON) |
| 8 | Add 4 MCP tools + param structs to `tools.rs`; add `use crate::sessions;` import | Tests: tool happy paths + error paths via `server.run(...)` |
| 9 | Wire `pub mod sessions;` in `lib.rs` | `cargo check`, `cargo test`, `cargo clippy -- -D warnings` all pass |

Tasks 3, 4, 5, 6 can run in parallel after tasks 1 and 2.

---

## DAG

```
[1: Types + constants (sessions.rs)]
             │
             ▼
[2: Db trait sigs (db.rs)]
             │
    ┌────────┼────────┬────────┐
    ▼        ▼        ▼        ▼
[3: create  [4: create  [5: list_  [6: list_
 _checkpoint] _branch]   checkpoints] branches]
    │        │        │        │
    └────────┴────────┴────────┘
                      │
                      ▼
         [7: Validation + business logic]
                      │
                      ▼
             [8: MCP tools (tools.rs)]
                      │
                      ▼
            [9: Wire module (lib.rs)]
```

---

## Sub-Agent Instructions

### Pre-conditions

- Read: `src/db.rs`, `src/error.rs`, `src/events.rs` (for constants to reuse), `src/sessions.rs` (does not exist yet — create it), `src/lib.rs`, `src/tools.rs`
- `cargo check` and `cargo test` pass on current main

### Step 1: Create `src/sessions.rs` with data types and constants

Create `src/sessions.rs`. Add `Checkpoint`, `Branch`, `InsertCheckpointParams<'a>`, `InsertBranchParams<'a>`, `ListCheckpointsParams<'a>`, `ListBranchesParams<'a>`, and the four constants. All data types use `#[derive(Debug, Clone, Serialize, Deserialize)]`. Param structs use `#[derive(Debug, Clone)]`. Import `serde::{Deserialize, Serialize}`.

### Step 2: Add Db trait method signatures to `db.rs`

In `src/db.rs`:
- Add `use crate::sessions::{Branch, Checkpoint, InsertBranchParams, InsertCheckpointParams, ListBranchesParams, ListCheckpointsParams};` to the imports
- Replace the four commented-out stubs under `// -- Sessions (Component 6) --` with the four method signatures exactly as specified above

### Steps 3-6: Implement each Db method for Connection

All implementations go in `src/db.rs` in `impl Db for Connection`.

**Task 3 — `create_checkpoint`**:
- Wrap both the SELECT check and INSERT in `self.unchecked_transaction(|tx| { ... })` (same pattern used in `consolidate_memory`)
- Step 1: `tx.query_row("SELECT COUNT(*) FROM events WHERE id = ?1 AND actor_id = ?2 AND session_id = ?3", params![event_id, actor_id, session_id], |r| r.get::<_, i64>(0))?` — if 0, return `MemoryError::NotFound("event not found for this actor and session".into())`
- Step 2: `tx.query_row("INSERT INTO checkpoints (id, session_id, actor_id, name, event_id, metadata) VALUES (:id, :session_id, :actor_id, :name, :event_id, :metadata) RETURNING id, session_id, actor_id, name, event_id, metadata, created_at", named_params!{...}, |row| {...})`
- For UNIQUE constraint violation: match `rusqlite::Error::SqliteFailure(e, _)` where `e.code == rusqlite::ErrorCode::ConstraintViolation` → return `MemoryError::InvalidInput("a checkpoint with this name already exists for the session".into())`
- Generate UUID with `uuid::Uuid::new_v4().to_string()`

**Task 4 — `create_branch`**:
- Wrap all three steps in `self.unchecked_transaction(|tx| { ... })` — same pattern as Task 3 (`create_checkpoint`). This prevents TOCTOU on parent_branch_id and ensures atomic commit/rollback.
- Step 1: `tx.query_row("SELECT COUNT(*) FROM events WHERE id = ?1 AND actor_id = ?2", params![root_event_id, actor_id], |r| r.get::<_, i64>(0))?` — if 0, return `MemoryError::NotFound("root event not found for this actor".into())`
- Step 2: if `parent_branch_id` is Some, `tx.query_row("SELECT COUNT(*) FROM branches WHERE id = ?1 AND session_id = ?2", params![parent_branch_id, session_id], |r| r.get::<_, i64>(0))?` — if 0, return `MemoryError::NotFound("parent branch not found for this session".into())`
- Step 3: `tx.query_row("INSERT INTO branches (id, session_id, name, parent_branch_id, root_event_id) VALUES (:id, :session_id, :name, :parent_branch_id, :root_event_id) RETURNING id, session_id, name, parent_branch_id, root_event_id, created_at", named_params!{...}, |row| {...})?`
- Commit: `tx.commit()?`; return the Branch
- Generate UUID with `uuid::Uuid::new_v4().to_string()`

**Task 5 — `list_checkpoints`**:
- `SELECT id, session_id, actor_id, name, event_id, metadata, created_at FROM checkpoints WHERE actor_id = ?1 AND session_id = ?2 ORDER BY created_at ASC, id ASC LIMIT ?3 OFFSET ?4`
- Map rows to `Vec<Checkpoint>`, return empty vec if none

**Task 6 — `list_branches`**:
- `SELECT b.id, b.session_id, b.name, b.parent_branch_id, b.root_event_id, b.created_at FROM branches b JOIN events e ON e.id = b.root_event_id AND e.actor_id = ?1 WHERE b.session_id = ?2 ORDER BY b.created_at ASC, b.id ASC LIMIT ?3 OFFSET ?4`
- Map rows to `Vec<Branch>`, return empty vec if none

### Step 7: Validation + business logic in `sessions.rs`

Add these validation helpers (reuse `validate_non_empty` and `validate_max_len` patterns from existing modules):

```rust
fn validate_no_control_chars(s: &str, field: &str) -> Result<(), MemoryError> {
    if s.chars().any(|c| c.is_control()) {
        return Err(MemoryError::InvalidInput(format!("{field} must not contain control characters")));
    }
    Ok(())
}
```

Implement:
- `fn validate_checkpoint_params(params: &InsertCheckpointParams) -> Result<(), MemoryError>` — checks actor_id, session_id, name (non-empty, max len, no control chars), event_id (non-empty, max 36), metadata (valid JSON object if present, max 64KB)
- `fn validate_branch_params(params: &InsertBranchParams) -> Result<(), MemoryError>` — checks actor_id, session_id, root_event_id (non-empty, max 36), name (if Some: non-empty, max len, no control chars), parent_branch_id (if Some: non-empty, max 36)
- Public functions: `create_checkpoint`, `create_branch`, `list_checkpoints`, `list_branches` as specified above

For metadata JSON validation, use: `serde_json::from_str::<serde_json::Value>(s).ok().and_then(|v| v.as_object().map(|_| ())).ok_or_else(|| MemoryError::InvalidInput("metadata must be a JSON object".into()))`

### Step 8: Add MCP tools to `tools.rs`

In `src/tools.rs`:
- Add `use crate::sessions;` to imports
- Add the four param structs (`CheckpointToolParams`, `BranchToolParams`, `ListCheckpointsToolParams`, `ListBranchesToolParams`) in the param structs section (after existing param structs, before `#[tool_router]` impl)
- Add the four tool methods (`checkpoint`, `branch`, `list_checkpoints`, `list_branches`) inside the `#[tool_router(server_handler)] impl MemoryServer` block, after the existing graph tools

### Step 9: Wire module in `lib.rs`

Add `pub mod sessions;` to `src/lib.rs` (alongside existing `pub mod events;`, `pub mod graph;`, etc.).

Run: `cargo check && cargo test && cargo clippy -- -D warnings`

### Test expectations

**In `db.rs` `#[cfg(test)] mod tests`:**

- `test_create_checkpoint_basic` — insert event, create checkpoint, verify all fields round-trip
- `test_create_checkpoint_wrong_actor` — event exists but for different actor → NotFound
- `test_create_checkpoint_wrong_session` — event exists but for different session → NotFound
- `test_create_checkpoint_name_conflict` — second checkpoint with same (session_id, name) → InvalidInput
- `test_create_checkpoint_missing_event` — event_id doesn't exist → NotFound
- `test_list_checkpoints_empty` — no checkpoints → empty vec
- `test_list_checkpoints_multiple` — multiple checkpoints, verify ASC order
- `test_list_checkpoints_actor_scoped` — checkpoints for actor A not visible to actor B
- `test_create_branch_basic` — insert event, create branch, verify all fields
- `test_create_branch_with_parent` — create parent branch, create child branch pointing to it
- `test_create_branch_missing_root_event` — root_event_id doesn't exist → NotFound
- `test_create_branch_wrong_actor` — root event exists for different actor → NotFound
- `test_create_branch_invalid_parent` — parent_branch_id for wrong session → NotFound
- `test_list_branches_empty` — no branches → empty vec
- `test_list_branches_multiple` — multiple branches, verify ASC order
- `test_list_branches_actor_scoped` — branches for actor A not returned for actor B query

**In `sessions.rs` `#[cfg(test)] mod tests`:**

- `test_validate_empty_actor_id` — rejects empty actor_id
- `test_validate_checkpoint_name_control_chars` — rejects name with control character (e.g. `\x01`)
- `test_validate_checkpoint_name_empty` — rejects empty name
- `test_validate_metadata_not_object` — rejects metadata `"[]"` (array, not object)
- `test_validate_metadata_invalid_json` — rejects metadata `"not json"`
- `test_validate_branch_name_control_chars` — rejects name with control chars
- `test_validate_event_id_too_long` — rejects event_id > 36 chars

**In `tools.rs` `#[cfg(test)] mod tests` (using `make_server()` helper):**

- `test_tool_checkpoint_basic` — create event via db, call `server.run(...)` with checkpoint params, verify response has `checkpoint.name`
- `test_tool_checkpoint_conflict` — create same checkpoint twice, second returns error with "invalid_input" code
- `test_tool_branch_basic` — create event, create branch, verify response has `branch.root_event_id`
- `test_tool_list_checkpoints_empty` — no checkpoints → `{"checkpoints": []}`
- `test_tool_list_branches_empty` — no branches → `{"branches": []}`
