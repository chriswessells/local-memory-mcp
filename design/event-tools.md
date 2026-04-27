# Component 2: Event Tools — Detailed Design

## Scope

Short-term memory operations: add, get, list, and expire events. This component adds:

1. **Data types** — `Event`, `SessionInfo`, `BranchFilter`, `InsertEventParams`, `GetEventsParams` in `events.rs`
2. **Db trait methods** — 5 methods added to the `Db` trait in `db.rs`
3. **Error variant** — `InvalidInput(String)` on `MemoryError`
4. **Business logic** — thin validation + delegation layer in `events.rs`
5. **Schema addition** — partial index on `expires_at`, CHECK constraints on `event_type` and `role`

This component does NOT include MCP tool definitions (Component 8) or search (Component 4).

---

## Design Review Resolutions

### Critical

**C1 — Payload model mismatch with AgentCore**: AgentCore supports multi-item payloads per event. We keep the flat model (one content/blob per event). In practice agents send one message per event. The MCP tool layer (Component 8) can accept a `payload` array and create one event per item if needed. Documented as a transparent difference.

### High

- **H1**: `insert_event` now takes `&InsertEventParams` struct instead of 9 positional params.
- **H2**: `branch_id` filter replaced with `BranchFilter` enum.
- **H3**: Timestamps use `strftime('%Y-%m-%dT%H:%M:%SZ', 'now')`. All inputs normalized to `YYYY-MM-DDTHH:MM:SSZ`.
- **H4**: `SessionInfo` now includes `actor_id`.
- **H5**: Db trait keeps limit/offset internally. MCP layer (Component 8) will implement cursor pagination on top using `after` as cursor.
- **H6**: `insert_event` returns full `Event` (constructed in-memory from inputs + generated id + captured timestamp).
- **H7**: Added partial index `idx_events_expires ON events(expires_at) WHERE expires_at IS NOT NULL`.
- **H8**: `get_event` now requires `actor_id` parameter.
- **H9**: Size limits added: content max 1MB, blob_data max 10MB, metadata max 64KB.

### Medium (logged to TODO backlog)

- Use `serde_bytes` for `blob_data` serialization
- Add `metadata_filter` parameter to `get_events` (reserved as `Option<&str>`, initial impl returns error if used)
- Add CHECK constraints on `event_type` and `role` in schema
- Add immutability trigger on events table
- Batch `delete_expired_events` with LIMIT
- Use named SQL parameters for dynamic queries
- Define constants for magic numbers (MAX_ACTOR_ID_LEN, MAX_PAGE_LIMIT, etc.)
- Custom `Debug` impl for `Event` that redacts blob_data
- Restrict `actor_id`/`session_id` to printable ASCII
- Validate metadata as JSON object with max depth/key limits
- Enforce `expires_at` must be in the future
- Handle cascading deletes for checkpoints/branches referencing expired events
- Log deleted event IDs at debug level in `delete_expired_events`

### Re-review (round 2) resolutions

- **H10**: Timestamp DEFAULT divergence — update ALL table DEFAULTs in migrate_v1 to `strftime('%Y-%m-%dT%H:%M:%SZ', 'now')`. Safe since no production databases exist.
- Removed `AddEventParams` — business logic uses `InsertEventParams` directly.
- Changed `insert_event` to use `INSERT...RETURNING` instead of separate SELECT + in-memory construction.
- Added `#[derive(Debug, Clone)]` requirement for param structs.
- Documented that Db trait methods have precondition: params must be pre-validated.

---

## Constants

```rust
pub const MAX_ACTOR_ID_LEN: usize = 256;
pub const MAX_SESSION_ID_LEN: usize = 256;
pub const MAX_CONTENT_SIZE: usize = 1_048_576;     // 1 MB
pub const MAX_BLOB_SIZE: usize = 10_485_760;       // 10 MB
pub const MAX_METADATA_SIZE: usize = 65_536;       // 64 KB
pub const MAX_PAGE_LIMIT: u32 = 1000;
pub const DEFAULT_PAGE_LIMIT: u32 = 100;
```

---

## Data Types

```rust
// src/events.rs

use serde::{Deserialize, Serialize};

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
    pub metadata: Option<String>,
    pub branch_id: Option<String>,
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
```

---

## Db Trait Methods

Added to `pub trait Db` in `db.rs`:

```rust
/// Insert an immutable event. Returns the full Event with generated id and created_at.
fn insert_event(&self, params: &InsertEventParams<'_>) -> Result<Event, MemoryError>;

/// Get a single event by ID, scoped to actor.
fn get_event(&self, actor_id: &str, event_id: &str) -> Result<Event, MemoryError>;

/// Get events for an actor+session, ordered by created_at ASC.
fn get_events(&self, params: &GetEventsParams<'_>) -> Result<Vec<Event>, MemoryError>;

/// List distinct sessions for an actor with event counts and date ranges.
fn list_sessions(&self, actor_id: &str, limit: u32, offset: u32) -> Result<Vec<SessionInfo>, MemoryError>;

/// Delete events past their expires_at. Returns count of deleted rows.
fn delete_expired_events(&self) -> Result<u64, MemoryError>;
```

### Parameter Structs (in events.rs, used by Db trait)

```rust
pub struct InsertEventParams<'a> {
    pub actor_id: &'a str,
    pub session_id: &'a str,
    pub event_type: &'a str,
    pub role: Option<&'a str>,
    pub content: Option<&'a str>,
    pub blob_data: Option<&'a [u8]>,
    pub metadata: Option<&'a str>,
    pub branch_id: Option<&'a str>,
    pub expires_at: Option<&'a str>,
}

pub struct GetEventsParams<'a> {
    pub actor_id: &'a str,
    pub session_id: &'a str,
    pub branch_id: BranchFilter<'a>,
    pub limit: u32,
    pub offset: u32,
    pub before: Option<&'a str>,
    pub after: Option<&'a str>,
}
```

---

## SQL Implementation

### Timestamp format

All timestamps use: `strftime('%Y-%m-%dT%H:%M:%SZ', 'now')` — ISO 8601 with UTC `Z` suffix.

### Schema changes (in migrate_v1)

**MANDATORY**: Update the `created_at` DEFAULT on the events table from `datetime('now')` to `strftime('%Y-%m-%dT%H:%M:%SZ', 'now')`. No production databases exist yet, so this is safe. Also update all other tables' DEFAULTs to the same format for consistency (memories, knowledge_edges, namespaces, checkpoints, branches).

```sql
-- All created_at/updated_at DEFAULTs in migrate_v1 must use:
DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))

-- Add after events table creation:
CREATE INDEX IF NOT EXISTS idx_events_expires ON events(expires_at) WHERE expires_at IS NOT NULL;
```

### insert_event

```rust
fn insert_event(&self, params: &InsertEventParams<'_>) -> Result<Event, MemoryError> {
    let id = uuid::Uuid::new_v4().to_string();
    // Use INSERT...RETURNING to atomically insert and retrieve the stored row.
    // This avoids a separate SELECT and guarantees the returned Event matches storage.
    self.query_row(
        "INSERT INTO events (id, actor_id, session_id, event_type, role, content, blob_data, metadata, branch_id, expires_at)
         VALUES (:id, :actor_id, :session_id, :event_type, :role, :content, :blob_data, :metadata, :branch_id, :expires_at)
         RETURNING id, actor_id, session_id, event_type, role, content, blob_data, metadata, branch_id, created_at, expires_at",
        named_params! {
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
        |row| { /* map row to Event */ }
    )?
}
```

Note: `created_at` uses the column DEFAULT `strftime('%Y-%m-%dT%H:%M:%SZ', 'now')` — not passed as a parameter. The RETURNING clause retrieves the actual stored value.

### get_event

```sql
SELECT id, actor_id, session_id, event_type, role, content, blob_data, metadata, branch_id, created_at, expires_at
FROM events WHERE id = :id AND actor_id = :actor_id
```

Returns `MemoryError::NotFound` if no row.

### get_events

Dynamic query using named parameters:

```sql
SELECT id, actor_id, session_id, event_type, role, content, blob_data, metadata, branch_id, created_at, expires_at
FROM events
WHERE actor_id = :actor_id AND session_id = :session_id
  [AND branch_id IS NULL]       -- BranchFilter::MainOnly
  [AND branch_id = :branch_id]  -- BranchFilter::Specific
  [AND created_at < :before]    -- if before provided
  [AND created_at > :after]     -- if after provided
ORDER BY created_at ASC, rowid ASC
LIMIT :limit OFFSET :offset
```

Note: `ORDER BY created_at ASC, rowid ASC` uses rowid as tiebreaker for events within the same second.

### list_sessions

```sql
SELECT session_id, :actor_id as actor_id, COUNT(*) as event_count,
       MIN(created_at) as first_event_at,
       MAX(created_at) as last_event_at
FROM events
WHERE actor_id = :actor_id
GROUP BY session_id
ORDER BY last_event_at DESC
LIMIT :limit OFFSET :offset
```

### delete_expired_events

```sql
DELETE FROM events WHERE expires_at IS NOT NULL AND expires_at < strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
```

Returns `changes()` count.

---

## Error Variant

Added to `MemoryError` in `error.rs`:

```rust
#[error("Invalid input: {0}")]
InvalidInput(String),
```

Error messages reference field names and allowed values, never echo back rejected input.

---

## Input Validation (in events.rs, before calling Db)

| Field | Rule |
|-------|------|
| `actor_id` | Non-empty, max `MAX_ACTOR_ID_LEN` chars |
| `session_id` | Non-empty, max `MAX_SESSION_ID_LEN` chars |
| `event_type` | Must be `"conversation"` or `"blob"` |
| `role` | If present, must be one of: `"user"`, `"assistant"`, `"tool"`, `"system"` |
| `content` | Required if conversation, forbidden if blob. Max `MAX_CONTENT_SIZE` bytes |
| `blob_data` | Required if blob, forbidden if conversation. Max `MAX_BLOB_SIZE` bytes |
| `metadata` | If present, must be valid JSON object. Max `MAX_METADATA_SIZE` bytes |
| `expires_at` | If present, must match `YYYY-MM-DDTHH:MM:SSZ` format |
| `limit` | Clamped to `1..=MAX_PAGE_LIMIT` (default `DEFAULT_PAGE_LIMIT`) |
| `offset` | No max, default 0 |
| `before`/`after` | If present, must match `YYYY-MM-DDTHH:MM:SSZ` format |

---

## Business Logic Layer (events.rs)

```rust
pub fn add_event(db: &dyn Db, params: &InsertEventParams<'_>) -> Result<Event, MemoryError> {
    validate_insert_params(params)?;
    db.insert_event(params)
}

pub fn get_event(db: &dyn Db, actor_id: &str, event_id: &str) -> Result<Event, MemoryError> {
    validate_non_empty(actor_id, "actor_id")?;
    db.get_event(actor_id, event_id)
}

pub fn get_events(db: &dyn Db, params: &GetEventsParams<'_>) -> Result<Vec<Event>, MemoryError> {
    // validate actor_id, session_id, clamp limit, validate timestamps
    db.get_events(params)
}

pub fn list_sessions(db: &dyn Db, actor_id: &str, limit: u32, offset: u32) -> Result<Vec<SessionInfo>, MemoryError> {
    validate_non_empty(actor_id, "actor_id")?;
    let limit = limit.clamp(1, MAX_PAGE_LIMIT);
    db.list_sessions(actor_id, limit, offset)
}

pub fn delete_expired(db: &dyn Db) -> Result<u64, MemoryError> {
    db.delete_expired_events()
}
```

---

## Implementation Plan

| # | Task | Acceptance Criteria |
|---|------|-------------------|
| 1 | Add `InvalidInput` variant to `MemoryError` | Compiles, existing tests pass |
| 2 | Add `Event`, `SessionInfo`, `BranchFilter`, `InsertEventParams`, `GetEventsParams` to `events.rs` | Structs compile |
| 3 | Add 5 Db trait method signatures + partial index in migration | Trait compiles, `_assert_object_safe` passes, new DB gets index |
| 4 | Implement `insert_event` for Connection | Test: insert returns Event with valid UUID and created_at |
| 5 | Implement `get_event` for Connection | Test: returns NotFound for missing ID; actor-scoped |
| 6 | Implement `get_events` for Connection | Tests: chronological order, BranchFilter variants, time range, limit/offset |
| 7 | Implement `list_sessions` for Connection | Test: correct counts, date ranges, actor_id in result |
| 8 | Implement `delete_expired_events` for Connection | Test: expired deleted, non-expired preserved |
| 9 | Add validation + business logic functions in `events.rs` | Tests: rejects invalid inputs, validates before inserting |
| 10 | Wire `pub mod events;` in `lib.rs` | `cargo check`, `cargo test`, `cargo clippy` all pass |

---

## DAG

```
[1: InvalidInput variant]
        │
        ├──────────────────┐
        ▼                  ▼
[2: Structs]         [3: Trait sigs + index]
        │                  │
        └────────┬─────────┘
                 ▼
        [4: insert_event impl]
                 │
                 ▼
        [5: get_event impl]
                 │
        ┌────────┼────────┐
        ▼        ▼        ▼
  [6: get_events] [7: list_sessions] [8: delete_expired]
        │        │        │
        └────────┼────────┘
                 ▼
        [9: Validation + business logic]
                 │
                 ▼
        [10: Wire module in lib.rs]
```

Tasks 6, 7, 8 can run in parallel after task 5.

---

## Sub-Agent Instructions

### Pre-conditions
- `cargo check` and `cargo test` pass on current main
- Read: `src/db.rs`, `src/error.rs`, `src/events.rs` (if exists), `src/lib.rs`

### Step 1: Add InvalidInput error variant

In `src/error.rs`, add to `MemoryError`:
```rust
#[error("Invalid input: {0}")]
InvalidInput(String),
```

### Step 2: Create `src/events.rs` with data types

Create `src/events.rs` with `Event`, `SessionInfo`, `BranchFilter`, `InsertEventParams`, `GetEventsParams`, and constants. Use `#[serde(with = "serde_bytes")]` on `blob_data`.

### Step 3: Add Db trait methods + schema index

In `src/db.rs`:
- Add `use crate::events::{Event, SessionInfo, InsertEventParams, GetEventsParams, BranchFilter};`
- Replace commented Event stubs with the 5 method signatures
- Add the partial index to `migrate_v1`
- Update `created_at` DEFAULT to `strftime('%Y-%m-%dT%H:%M:%SZ', 'now')` in the events CREATE TABLE

### Steps 4-8: Implement each Db method

Use `rusqlite::named_params!` for all queries. Use `uuid::Uuid::new_v4()` for IDs. For `insert_event`, capture timestamp with `SELECT strftime(...)` then construct Event in-memory. For `get_events`, build dynamic SQL with named parameters and match on `BranchFilter`. Use `ORDER BY created_at ASC, rowid ASC`.

### Step 9: Validation and business logic

Add validation functions and public API functions in `events.rs`. Error messages must not echo user input.

### Step 10: Wire module

Add `pub mod events;` to `lib.rs`. Run `cargo check`, `cargo test`, `cargo clippy -- -D warnings`.

### Test expectations

In `db.rs` tests:
- `test_insert_and_get_event` — insert, retrieve, verify all fields match
- `test_get_event_not_found` — returns NotFound for missing ID
- `test_get_event_wrong_actor` — returns NotFound when actor doesn't match
- `test_get_events_chronological` — multiple events returned in order (use explicit timestamps)
- `test_get_events_branch_filter` — All vs MainOnly vs Specific
- `test_get_events_time_range` — before/after filtering
- `test_get_events_limit_offset` — pagination
- `test_list_sessions` — multiple sessions, correct counts, actor_id present
- `test_delete_expired` — expired deleted, non-expired kept
- `test_insert_event_blob` — blob event with blob_data

In `events.rs` tests:
- `test_validate_event_type` — rejects invalid event_type
- `test_validate_empty_actor` — rejects empty actor_id
- `test_validate_content_blob_mismatch` — conversation needs content, blob needs blob_data
- `test_validate_content_size` — rejects oversized content
- `test_add_event_validates` — full validation through add_event
