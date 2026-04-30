# Component 3: Memory Tools — Detailed Design

## Scope

Long-term memory operations: store, get, consolidate, list, and delete memories. This component adds:

1. **Data types** — `Memory`, `InsertMemoryParams`, `ConsolidateAction`, `ListMemoriesParams` in `memories.rs`
2. **Db trait methods** — 5 methods added to the `Db` trait in `db.rs`
3. **Business logic** — validation + delegation layer in `memories.rs`

This component does NOT include:
- MCP tool definitions (Component 8)
- FTS5/vector search via `memory.retrieve_memory_records` (Component 4 — Search)
- Namespace CRUD (Component 7 — Namespace tools)

**Note on recall**: The `memory.retrieve_memory_records` MCP tool combines FTS5 and vector search. That logic lives in Component 4 (Search). This component provides the basic CRUD that recall depends on.

**Note on embeddings**: `memory.create_memory_record` accepts an optional `embedding` parameter. The Db layer stores the embedding in `memory_vec` as part of `insert_memory`. This keeps the insert atomic — the memory and its embedding are created in the same transaction. Component 4 handles the *query* side (searching `memory_vec`).

---

## Design Review Resolutions

### High

- **H1 — Missing actor_id scoping on get_memory, delete_memory, consolidate_memory**: Added `actor_id` as required parameter to all three methods. WHERE clauses include `AND actor_id = :actor_id`. Matches the pattern from `get_event(actor_id, event_id)`.
- **H2 — ConsolidateAction should carry its data**: Changed enum to `Update { content: &'a str, embedding: Option<&'a [f32]> }` and `Invalidate`. Eliminates separate `new_content`/`new_embedding` params from the Db trait method. Invalid states are now unrepresentable.
- **H3 — LIKE wildcard injection in namespace_prefix**: Escape `%` and `_` in user-supplied prefix before appending `%`. Use `LIKE :prefix ESCAPE '\'`.
- **H4 — delete_memory not wrapped in transaction**: Wrap both `DELETE FROM memory_vec` and `DELETE FROM memories` in an explicit transaction.

### Re-review (round 2) resolutions

- **H5 — delete_memory can delete another actor's embedding before rollback**: Reordered to delete from `memories` first (verifies actor ownership), check `changes()`, then delete from `memory_vec` only if ownership confirmed.
- **H6 — vec0 virtual table transaction semantics uncertain**: Documented the risk. Reordered consolidate_memory to do regular table ops first, vec0 ops last. Added requirement for a test verifying vec0 rollback behavior. Orphan embeddings are a minor inconsistency, not a data corruption issue.

### Medium (logged to TODO backlog)

- Extract shared validation helpers to `src/validation.rs`
- Split `db.rs` into module directory (`db/mod.rs`, `db/events.rs`, `db/memories.rs`)
- Fix `main.rs` to use library crate instead of re-declaring modules
- Use named column access in `row_to_memory` instead of positional indices
- Add CI pipeline (GitHub Actions)
- Add optional `new_metadata` to consolidate Update, or document metadata is NOT inherited
- Document consolidation embedding behavior: old embedding always deleted, provide new_embedding if replacement should be vector-searchable
- Add database size quota check before inserts
- Unify permission hardening in `with_base_dir`
- Log warning when `LOCAL_MEMORY_SYNC=normal` downgrades durability

---

## Constants

```rust
pub const MAX_MEMORY_CONTENT_SIZE: usize = 1_048_576;  // 1 MB
pub const MAX_NAMESPACE_LEN: usize = 512;
pub const MAX_STRATEGY_LEN: usize = 128;
```

Reuses from `events.rs`: `MAX_ACTOR_ID_LEN`, `MAX_METADATA_SIZE`, `MAX_PAGE_LIMIT`, `DEFAULT_PAGE_LIMIT`.

---

## Data Types

```rust
// src/memories.rs

use serde::{Deserialize, Serialize};

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
    pub namespace: Option<&'a str>,       // defaults to "default"
    pub metadata: Option<&'a str>,        // JSON object
    pub source_session_id: Option<&'a str>,
    pub embedding: Option<&'a [f32]>,     // 384-dim vector, stored in memory_vec
}

#[derive(Debug, Clone)]
pub enum ConsolidateAction<'a> {
    /// Replace content (and optionally embedding). Old memory marked invalid,
    /// superseded_by points to new memory.
    Update { content: &'a str, embedding: Option<&'a [f32]> },
    /// Mark memory as invalid. No replacement created.
    Invalidate,
}

#[derive(Debug, Clone)]
pub struct ListMemoriesParams<'a> {
    pub actor_id: &'a str,
    pub namespace: Option<&'a str>,
    pub namespace_prefix: Option<&'a str>,
    pub strategy: Option<&'a str>,
    pub valid_only: bool,                 // default true
    pub limit: u32,
    pub offset: u32,
}
```

### `is_valid` mapping

The database stores `is_valid` as `INTEGER` (0 or 1). The `Memory` struct uses `bool`. The `row_to_memory` helper maps `0 → false`, non-zero `→ true`.

---

## Db Trait Methods

Added to `pub trait Db` in `db.rs` (replacing the commented stubs):

```rust
// -- Memories (Component 3) --

/// Insert a memory. If embedding is provided, also inserts into memory_vec.
/// Returns the full Memory with generated id and timestamps.
fn insert_memory(&self, params: &InsertMemoryParams<'_>) -> Result<Memory, MemoryError>;

/// Get a single memory by ID, scoped to actor.
fn get_memory(&self, actor_id: &str, memory_id: &str) -> Result<Memory, MemoryError>;

/// List memories with filters. Ordered by created_at DESC.
fn list_memories(&self, params: &ListMemoriesParams<'_>) -> Result<Vec<Memory>, MemoryError>;

/// Consolidate a memory, scoped to actor. Atomic transaction:
/// - Update: mark old invalid + set superseded_by + insert new memory → returns new Memory
/// - Invalidate: mark old invalid → returns updated old Memory
fn consolidate_memory(
    &self,
    actor_id: &str,
    memory_id: &str,
    action: &ConsolidateAction<'_>,
) -> Result<Memory, MemoryError>;

/// Hard-delete a memory and its embedding, scoped to actor. Edges cascade via FK.
fn delete_memory(&self, actor_id: &str, memory_id: &str) -> Result<(), MemoryError>;
```

### Import in db.rs

```rust
use crate::memories::{Memory, InsertMemoryParams, ListMemoriesParams, ConsolidateAction};
```

---

## SQL Implementation

### row_to_memory helper

```rust
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
```

### insert_memory

```sql
INSERT INTO memories (id, actor_id, namespace, strategy, content, metadata, source_session_id)
VALUES (:id, :actor_id, :namespace, :strategy, :content, :metadata, :source_session_id)
RETURNING id, actor_id, namespace, strategy, content, metadata, source_session_id,
          is_valid, superseded_by, created_at, updated_at
```

If `embedding` is provided, also insert into `memory_vec`:

```sql
INSERT INTO memory_vec (memory_id, embedding) VALUES (:memory_id, :embedding)
```

Both statements run in a single transaction. The embedding is serialized as a blob via `sqlite-vec`'s float array format.

**Embedding dimension validation**: If `embedding.len() != EMBEDDING_DIM as usize`, return `InvalidInput`.

### get_memory

```sql
SELECT id, actor_id, namespace, strategy, content, metadata, source_session_id,
       is_valid, superseded_by, created_at, updated_at
FROM memories WHERE id = :id AND actor_id = :actor_id
```

Returns `MemoryError::NotFound` if no row.

### list_memories

Dynamic query with optional filters:

```sql
SELECT id, actor_id, namespace, strategy, content, metadata, source_session_id,
       is_valid, superseded_by, created_at, updated_at
FROM memories
WHERE actor_id = :actor_id
  [AND namespace = :namespace]                       -- if namespace provided
  [AND namespace LIKE :namespace_prefix ESCAPE '\']  -- if namespace_prefix provided (escaped, then '%' appended)
  [AND strategy = :strategy]                         -- if strategy provided
  [AND is_valid = 1]                                 -- if valid_only is true
ORDER BY created_at DESC
LIMIT :limit OFFSET :offset
```

`namespace` and `namespace_prefix` are mutually exclusive — validation rejects both being set.

**LIKE escaping**: Before appending `%`, escape `%` → `\%` and `_` → `\_` in the user-supplied prefix. Use `ESCAPE '\'` clause.

### consolidate_memory — Update

Atomic transaction. Regular table operations first, then `memory_vec` operations:

```sql
BEGIN;

-- 1. Verify old memory exists, is valid, and belongs to actor
SELECT id, actor_id, namespace, strategy FROM memories
WHERE id = :old_id AND actor_id = :actor_id AND is_valid = 1;
-- If no row → ROLLBACK, return NotFound

-- 2. Insert new memory (inherits actor_id, namespace, strategy from old)
INSERT INTO memories (id, actor_id, namespace, strategy, content, metadata, source_session_id)
VALUES (:new_id, :actor_id, :namespace, :strategy, :new_content, NULL, NULL)
RETURNING id, actor_id, namespace, strategy, content, metadata, source_session_id,
          is_valid, superseded_by, created_at, updated_at;

-- 3. Mark old memory invalid
UPDATE memories SET is_valid = 0, superseded_by = :new_id,
       updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
WHERE id = :old_id;

-- 4. Delete old embedding (always — new memory starts clean)
DELETE FROM memory_vec WHERE memory_id = :old_id;

-- 5. If new_embedding provided (from ConsolidateAction::Update { embedding: Some(...) }),
--    insert into memory_vec
INSERT INTO memory_vec (memory_id, embedding) VALUES (:new_id, :new_embedding);

COMMIT;
```

Returns the newly created `Memory`. New memory gets `NULL` metadata and `NULL` source_session_id (does not inherit from old).

**vec0 transaction note**: `sqlite-vec` `vec0` virtual tables may have different transaction semantics than regular tables. If a crash occurs after COMMIT, the `memory_vec` state should be consistent because SQLite's WAL ensures atomicity. However, if `vec0` does not fully participate in rollback, a failed transaction could leave orphan embeddings. The implementation should include a test that verifies `vec0` INSERT is rolled back when the transaction aborts. An orphan embedding (embedding without a corresponding memory) is a minor inconsistency — it wastes space but does not corrupt search results because search joins through the `memories` table.

### consolidate_memory — Invalidate

```sql
UPDATE memories SET is_valid = 0,
       updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
WHERE id = :id AND actor_id = :actor_id AND is_valid = 1
RETURNING id, actor_id, namespace, strategy, content, metadata, source_session_id,
          is_valid, superseded_by, created_at, updated_at
```

Returns `NotFound` if memory doesn't exist, doesn't belong to actor, or is already invalid.

### delete_memory

Wrapped in a transaction (H4). Verify ownership before touching `memory_vec` (re-review H5):

```sql
BEGIN;
-- 1. Delete from memories first (verifies actor ownership via WHERE clause)
DELETE FROM memories WHERE id = :id AND actor_id = :actor_id;
-- If 0 rows affected → ROLLBACK, return NotFound
-- 2. Delete embedding (memory_vec has no actor_id column, so we only reach here if ownership verified)
DELETE FROM memory_vec WHERE memory_id = :id;
COMMIT;
```

Returns `NotFound` if no row deleted from memories. Edges cascade via FK ON DELETE CASCADE.

**Implementation note**: Check `changes()` after the first DELETE. If 0, rollback and return `NotFound` before touching `memory_vec`.

---

## Input Validation (in memories.rs, before calling Db)

| Field | Rule |
|-------|------|
| `actor_id` | Non-empty, max `MAX_ACTOR_ID_LEN` |
| `content` | Non-empty, max `MAX_MEMORY_CONTENT_SIZE` |
| `strategy` | Non-empty, max `MAX_STRATEGY_LEN` |
| `namespace` | If present, non-empty, max `MAX_NAMESPACE_LEN` |
| `namespace_prefix` | If present, non-empty, max `MAX_NAMESPACE_LEN` |
| `namespace` + `namespace_prefix` | Mutually exclusive — reject if both set |
| `metadata` | If present, valid JSON object, max `MAX_METADATA_SIZE` |
| `source_session_id` | If present, non-empty |
| `embedding` | If present, length must equal `EMBEDDING_DIM` (384) |
| `memory_id` | Non-empty (for get, consolidate, delete) |
| `ConsolidateAction::Update.content` | Non-empty, max `MAX_MEMORY_CONTENT_SIZE` |
| `ConsolidateAction::Update.embedding` | If present, length must equal `EMBEDDING_DIM` |
| `limit` | Clamped to `1..=MAX_PAGE_LIMIT` |

---

## Business Logic Layer (memories.rs)

```rust
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
pub fn list_memories(db: &dyn Db, params: &ListMemoriesParams<'_>) -> Result<Vec<Memory>, MemoryError> {
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
```

---

## Implementation Plan

| # | Task | Acceptance Criteria |
|---|------|-------------------|
| 1 | Add `Memory`, `InsertMemoryParams`, `ConsolidateAction`, `ListMemoriesParams` to `memories.rs` with constants | Structs compile, serde derives work |
| 2 | Add 5 Db trait method signatures to `db.rs` | Trait compiles, `_assert_object_safe` passes |
| 3 | Implement `insert_memory` for Connection | Test: insert returns Memory with valid UUID, timestamps, default namespace; embedding stored in memory_vec |
| 4 | Implement `get_memory` for Connection | Test: returns NotFound for missing ID |
| 5 | Implement `list_memories` for Connection | Tests: filters by actor, namespace, namespace_prefix, strategy, valid_only; pagination |
| 6 | Implement `consolidate_memory` for Connection | Tests: Update creates new + invalidates old + sets superseded_by; Invalidate marks invalid; NotFound for missing/already-invalid |
| 7 | Implement `delete_memory` for Connection | Test: deletes memory + embedding; NotFound for missing |
| 8 | Add validation + business logic functions in `memories.rs` | Tests: rejects invalid inputs |
| 9 | Wire `pub mod memories;` in `lib.rs`, run `cargo check && cargo test && cargo clippy -- -D warnings` | All pass |

---

## DAG

```
[1: Data types + constants]     [2: Db trait signatures]
        │                              │
        └──────────┬───────────────────┘
                   ▼
          [3: insert_memory impl]
                   │
          [4: get_memory impl]
                   │
          ┌────────┼────────┐
          ▼        ▼        ▼
    [5: list]  [6: consolidate]  [7: delete]
          │        │        │
          └────────┼────────┘
                   ▼
          [8: Validation + business logic]
                   │
                   ▼
          [9: Wire module + verify]
```

Tasks 5, 6, 7 can run in parallel after task 4.

---

## Sub-Agent Instructions

### Pre-conditions
- `cargo check` and `cargo test` pass on current main
- Read: `src/db.rs`, `src/error.rs`, `src/events.rs`, `src/lib.rs`

### Step 1: Create `src/memories.rs` with data types

Create `src/memories.rs` with:
- Constants: `MAX_MEMORY_CONTENT_SIZE`, `MAX_NAMESPACE_LEN`, `MAX_STRATEGY_LEN`
- Structs: `Memory` (with Serialize/Deserialize), `InsertMemoryParams`, `ListMemoriesParams`
- Enum: `ConsolidateAction<'a>` with `Update { content: &'a str, embedding: Option<&'a [f32]> }` and `Invalidate`
- `Memory.is_valid` is `bool` (mapped from INTEGER in DB)

### Step 2: Add Db trait methods

In `src/db.rs`:
- Add `use crate::memories::{Memory, InsertMemoryParams, ListMemoriesParams, ConsolidateAction};`
- Replace the commented `// -- Memories (Component 3) --` stubs with the 5 method signatures
- `get_memory`, `consolidate_memory`, `delete_memory` all take `actor_id` as first param
- `consolidate_memory` takes `&ConsolidateAction<'_>` (no separate new_content/new_embedding params)
- Add `row_to_memory` helper function

### Step 3: Implement `insert_memory`

- Generate UUID with `uuid::Uuid::new_v4()`
- Use `INSERT...RETURNING` to get the full row
- If `embedding` is provided, validate length == `EMBEDDING_DIM`, then insert into `memory_vec`
- Wrap both inserts in a transaction when embedding is present
- Default namespace to `"default"` if None

### Step 4: Implement `get_memory`

- SELECT by id AND actor_id
- Return `NotFound` on no rows

### Step 5: Implement `list_memories`

- Build dynamic SQL with optional WHERE clauses
- Use positional parameters (same pattern as `get_events`)
- **LIKE escaping (H3)**: For `namespace_prefix`, escape `%` → `\%` and `_` → `\_` before appending `%`. Use `ESCAPE '\'` clause.
- Order by `created_at DESC`

### Step 6: Implement `consolidate_memory`

- Match on `ConsolidateAction` variant:
  - **Update { content, embedding }**: Use a transaction. Fetch old memory (with actor_id check) to get namespace/strategy. Insert new memory with content from the variant. Update old memory's `is_valid=0`, `superseded_by=new_id`. Delete old embedding. If embedding is Some, insert new embedding. Return new memory.
  - **Invalidate**: Single UPDATE...RETURNING with `AND actor_id = :actor_id AND is_valid = 1`. Return `NotFound` if 0 rows affected.

### Step 7: Implement `delete_memory`

- Wrap in a transaction (H4)
- Delete from `memory_vec` first (no FK from virtual table)
- Delete from `memories` with `AND actor_id = :actor_id` (edges cascade via FK)
- Return `NotFound` if 0 rows deleted from memories

### Step 8: Add validation and business logic

In `memories.rs`, add:
- Duplicate `validate_non_empty`, `validate_max_len` from events.rs (small helpers, ~10 lines each)
- `validate_insert_memory_params` — checks all InsertMemoryParams fields
- `validate_list_memories_params` — checks namespace/namespace_prefix mutual exclusion, actor_id
- `validate_consolidate_params` — validates content/embedding inside ConsolidateAction::Update
- Public functions: `store_memory`, `get_memory`, `list_memories`, `consolidate_memory`, `delete_memory`
- All get/consolidate/delete functions take `actor_id` as a parameter

### Step 9: Wire module

In `src/lib.rs`, add `pub mod memories;`. Run:
```bash
cargo check && cargo test && cargo clippy -- -D warnings
```

### Test expectations

In `db.rs` tests:
- `test_insert_and_get_memory` — insert, retrieve with actor_id, verify all fields
- `test_insert_memory_with_embedding` — verify embedding stored in memory_vec
- `test_insert_memory_default_namespace` — namespace defaults to "default"
- `test_get_memory_not_found` — returns NotFound
- `test_get_memory_wrong_actor` — returns NotFound when actor doesn't match
- `test_list_memories_by_actor` — filters by actor_id
- `test_list_memories_by_namespace` — exact namespace filter
- `test_list_memories_by_namespace_prefix` — LIKE prefix filter with escaping
- `test_list_memories_by_strategy` — strategy filter
- `test_list_memories_valid_only` — excludes invalid when valid_only=true
- `test_list_memories_pagination` — limit/offset
- `test_consolidate_update` — old invalid, new created, superseded_by set, actor-scoped
- `test_consolidate_invalidate` — memory marked invalid, actor-scoped
- `test_consolidate_already_invalid` — returns NotFound
- `test_delete_memory` — memory and embedding removed, actor-scoped
- `test_delete_memory_not_found` — returns NotFound
- `test_delete_memory_wrong_actor` — returns NotFound when actor doesn't match

In `memories.rs` tests:
- `test_validate_empty_actor` — rejects empty actor_id
- `test_validate_empty_content` — rejects empty content
- `test_validate_embedding_wrong_dim` — rejects wrong dimension
- `test_validate_namespace_mutual_exclusion` — rejects both namespace + namespace_prefix
- `test_validate_consolidate_update_content` — validates content inside Update variant
- `test_store_memory_validates` — full validation through store_memory
