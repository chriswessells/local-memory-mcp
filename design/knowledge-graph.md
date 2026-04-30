# Component 5: Knowledge Graph — Detailed Design

## Overview

The knowledge graph adds typed, directed relationships between memories. It enables the agent to link concepts, track dependencies, and traverse connected knowledge. All graph operations share the same SQLite file and ACID transactions as the memory system.

This component adds:
- `Edge` data type and supporting structs
- 7 `Db` trait methods for graph operations
- `graph.rs` business logic layer with validation
- 7 MCP tools (`graph.create_edge`, `graph.get_neighbors`, `graph.traverse`, `graph.update_edge`, `graph.delete_edge`, `graph.list_labels`, `graph.get_stats`)

---

## Schema (already created in V1 migration)

```sql
CREATE TABLE IF NOT EXISTS knowledge_edges (
    id TEXT PRIMARY KEY,           -- UUID
    from_memory_id TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
    to_memory_id TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
    label TEXT NOT NULL,           -- 'uses', 'depends_on', 'related_to', etc.
    properties TEXT,               -- JSON object for edge metadata
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
CREATE INDEX IF NOT EXISTS idx_edges_from ON knowledge_edges(from_memory_id, label);
CREATE INDEX IF NOT EXISTS idx_edges_to ON knowledge_edges(to_memory_id, label);
CREATE INDEX IF NOT EXISTS idx_edges_label ON knowledge_edges(label);
```

The `ON DELETE CASCADE` on both FKs means deleting a memory (via `memory.delete_memory_record`) automatically removes all edges referencing it. No additional cleanup needed in graph.rs.

---

## Data Types

### Edge

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub id: String,
    pub from_memory_id: String,
    pub to_memory_id: String,
    pub label: String,
    pub properties: Option<String>,  // JSON object
    pub created_at: String,
}
```

### Neighbor

Returned by `get_neighbors` — an edge plus the connected memory.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Neighbor {
    pub edge: Edge,
    pub memory: Memory,
}
```

### TraversalNode

Returned by `traverse` — a memory with its depth and the path of memory IDs from start to this node.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraversalNode {
    pub memory: Memory,
    pub depth: u32,
    pub path: Vec<String>,  // memory IDs from start to this node (excluding start)
}
```

### Direction

Used internally in graph.rs and as a serde-deserializable enum in MCP tool params (matching the `EventType`/`Role` pattern in tools.rs).

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Out,   // from_memory_id = given ID
    In,    // to_memory_id = given ID
    Both,  // either direction
}

impl Default for Direction {
    fn default() -> Self { Direction::Out }
}
```

### LabelCount

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LabelCount {
    pub label: String,
    pub count: u64,
}
```

### GraphStats

```rust
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
```

### InsertEdgeParams

```rust
#[derive(Debug, Clone)]
pub struct InsertEdgeParams<'a> {
    pub actor_id: &'a str,
    pub from_memory_id: &'a str,
    pub to_memory_id: &'a str,
    pub label: &'a str,
    pub properties: Option<&'a str>,
}
```

### UpdateEdgeParams

```rust
#[derive(Debug, Clone)]
pub struct UpdateEdgeParams<'a> {
    pub actor_id: &'a str,
    pub edge_id: &'a str,
    pub label: Option<&'a str>,
    pub properties: Option<&'a str>,
}
```

---

## Db Trait Additions

```rust
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

/// List all distinct edge labels with counts (global, not actor-scoped).
fn list_edge_labels(&self) -> Result<Vec<LabelCount>, MemoryError>;

/// Graph statistics (global, not actor-scoped).
fn graph_stats(&self) -> Result<GraphStats, MemoryError>;
```

---

## SQL Implementations

### insert_edge

```sql
INSERT INTO knowledge_edges (id, from_memory_id, to_memory_id, label, properties)
VALUES (:id, :from_memory_id, :to_memory_id, :label, :properties)
RETURNING id, from_memory_id, to_memory_id, label, properties, created_at
```

Before inserting, verify both memory IDs exist and belong to the actor:
```sql
SELECT COUNT(*) FROM memories WHERE id IN (:from_id, :to_id) AND actor_id = :actor_id
```
If count != 2, return `MemoryError::NotFound` for the missing memory.

### get_edge

```sql
SELECT e.id, e.from_memory_id, e.to_memory_id, e.label, e.properties, e.created_at
FROM knowledge_edges e
JOIN memories m ON m.id = e.from_memory_id
WHERE e.id = :edge_id AND m.actor_id = :actor_id
```

### get_neighbors

Direction determines the WHERE clause:

- **Out**: `WHERE e.from_memory_id = :memory_id` → join `memories ON m.id = e.to_memory_id`
- **In**: `WHERE e.to_memory_id = :memory_id` → join `memories ON m.id = e.from_memory_id`
- **Both**: `WHERE e.from_memory_id = :memory_id OR e.to_memory_id = :memory_id` → join on the "other" side using CASE

For **Both**, the connected memory is the one that is NOT the given memory_id:

```sql
SELECT e.id, e.from_memory_id, e.to_memory_id, e.label, e.properties, e.created_at,
       m.id, m.actor_id, m.namespace, m.strategy, m.content, m.metadata,
       m.source_session_id, m.is_valid, m.superseded_by, m.created_at, m.updated_at
FROM knowledge_edges e
JOIN memories m ON m.id = CASE
    WHEN e.from_memory_id = :memory_id THEN e.to_memory_id
    ELSE e.from_memory_id
END
WHERE (e.from_memory_id = :memory_id OR e.to_memory_id = :memory_id)
  AND m.actor_id = :actor_id
  [AND e.label = :label]
ORDER BY e.created_at DESC
LIMIT :limit
```

For **Out** and **In**, simpler joins:

```sql
-- Out
SELECT e.*, m.*
FROM knowledge_edges e
JOIN memories m ON m.id = e.to_memory_id
WHERE e.from_memory_id = :memory_id
  AND m.actor_id = :actor_id
  [AND e.label = :label]
ORDER BY e.created_at DESC LIMIT :limit

-- In
SELECT e.*, m.*
FROM knowledge_edges e
JOIN memories m ON m.id = e.from_memory_id
WHERE e.to_memory_id = :memory_id
  AND m.actor_id = :actor_id
  [AND e.label = :label]
ORDER BY e.created_at DESC LIMIT :limit
```

### traverse (recursive CTE)

BFS traversal with cycle detection and depth limiting. Three static SQL variants (one per direction) — selected via `match direction`, never built dynamically.

**Out direction** (`SQL_TRAVERSE_OUT`):
```sql
WITH RECURSIVE graph_walk(memory_id, depth, path, visited) AS (
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
      AND json_array_length(gw.visited) < :max_visited
      [AND e.label = :label]
      AND NOT EXISTS (
          SELECT 1 FROM json_each(gw.visited) WHERE value = e.to_memory_id
      )
)
SELECT gw.memory_id, gw.depth, gw.path,
       m.id, m.actor_id, m.namespace, m.strategy, m.content, m.metadata,
       m.source_session_id, m.is_valid, m.superseded_by, m.created_at, m.updated_at
FROM graph_walk gw
JOIN memories m ON m.id = gw.memory_id AND m.actor_id = :actor_id
WHERE gw.depth > 0
ORDER BY gw.depth ASC, m.created_at DESC
LIMIT 1000
```

**In direction** (`SQL_TRAVERSE_IN`): Same structure but join on `e.to_memory_id = gw.memory_id` and select `e.from_memory_id`.

**Both direction** (`SQL_TRAVERSE_BOTH`): Same structure but join on `e.from_memory_id = gw.memory_id OR e.to_memory_id = gw.memory_id` and select the "other" side via CASE.

The path contains **memory IDs** (not edge IDs) — the sequence of memories visited from start to this node. This is more useful to agents than edge IDs.

Direction-dependent join conditions:
- **Out**: `e.from_memory_id = gw.memory_id`
- **In**: `e.to_memory_id = gw.memory_id`
- **Both**: `e.from_memory_id = gw.memory_id OR e.to_memory_id = gw.memory_id`

**Limits**:
- `max_depth`: default 2, max 5 (enforced in validation)
- `max_visited`: 1000 nodes (hard cap to prevent runaway CTEs on dense graphs)

### update_edge

Actor scoping via join to memories. Use separate SQL paths based on which fields are provided (avoids sentinel pattern — see MAINT-2 resolution):

```sql
-- When updating label only:
UPDATE knowledge_edges SET label = :label
WHERE id = :edge_id
  AND EXISTS (SELECT 1 FROM memories WHERE id = knowledge_edges.from_memory_id AND actor_id = :actor_id)
RETURNING id, from_memory_id, to_memory_id, label, properties, created_at

-- When updating properties only:
UPDATE knowledge_edges SET properties = :properties
WHERE id = :edge_id
  AND EXISTS (SELECT 1 FROM memories WHERE id = knowledge_edges.from_memory_id AND actor_id = :actor_id)
RETURNING id, from_memory_id, to_memory_id, label, properties, created_at

-- When updating both:
UPDATE knowledge_edges SET label = :label, properties = :properties
WHERE id = :edge_id
  AND EXISTS (SELECT 1 FROM memories WHERE id = knowledge_edges.from_memory_id AND actor_id = :actor_id)
RETURNING id, from_memory_id, to_memory_id, label, properties, created_at
```

### delete_edge

```sql
DELETE FROM knowledge_edges
WHERE id = :edge_id
  AND EXISTS (SELECT 1 FROM memories WHERE id = knowledge_edges.from_memory_id AND actor_id = :actor_id)
```

Return `NotFound` if no rows affected.

### list_edge_labels

```sql
SELECT label, COUNT(*) as count
FROM knowledge_edges
GROUP BY label
ORDER BY count DESC
```

### graph_stats

```sql
-- Total edges
SELECT COUNT(*) FROM knowledge_edges;

-- Label distribution (reuse list_edge_labels)

-- Most connected memories (top 10 by total edge count)
SELECT memory_id, COUNT(*) as edge_count FROM (
    SELECT from_memory_id AS memory_id FROM knowledge_edges
    UNION ALL
    SELECT to_memory_id AS memory_id FROM knowledge_edges
)
GROUP BY memory_id
ORDER BY edge_count DESC
LIMIT 10
```

---

## graph.rs Business Logic

### Constants

```rust
pub const MAX_LABEL_LEN: usize = 256;
pub const MAX_PROPERTIES_SIZE: usize = 65_536;  // 64 KB (same as metadata)
pub const MAX_EDGE_ID_LEN: usize = 256;
pub const MAX_MEMORY_ID_LEN: usize = 256;
pub const MAX_TRAVERSE_DEPTH: u32 = 5;
pub const DEFAULT_TRAVERSE_DEPTH: u32 = 2;
pub const MAX_TRAVERSE_VISITED: u32 = 1000;
pub const MAX_NEIGHBOR_LIMIT: u32 = 1000;
pub const DEFAULT_NEIGHBOR_LIMIT: u32 = 100;
```

### Validation

Each public function validates inputs before delegating to `Db`:

- `add_edge`: validate `actor_id`, `from_memory_id`, `to_memory_id`, `label` non-empty + max length. Validate `properties` is valid JSON object if provided. Reject self-edges (`from == to`).
- `get_neighbors`: validate `actor_id`, `memory_id` non-empty. Clamp limit.
- `traverse`: validate `actor_id`, `start_memory_id` non-empty. Clamp `max_depth` to 1..=MAX_TRAVERSE_DEPTH.
- `update_edge`: validate `actor_id`, `edge_id` non-empty. At least one of `label` or `properties` must be provided. Validate `label` non-empty + max length if provided. Validate `properties` is valid JSON object if provided.
- `delete_edge`: validate `actor_id`, `edge_id` non-empty.
- `list_labels`: no validation needed.
- `graph_stats`: no validation needed.

### Public API

```rust
pub fn add_edge(db: &dyn Db, params: &InsertEdgeParams<'_>) -> Result<Edge, MemoryError>;
pub fn get_neighbors(db: &dyn Db, actor_id: &str, memory_id: &str, direction: Direction, label: Option<&str>, limit: u32) -> Result<Vec<Neighbor>, MemoryError>;
pub fn traverse(db: &dyn Db, actor_id: &str, start_memory_id: &str, max_depth: u32, label: Option<&str>, direction: Direction) -> Result<Vec<TraversalNode>, MemoryError>;
pub fn update_edge(db: &dyn Db, params: &UpdateEdgeParams<'_>) -> Result<Edge, MemoryError>;
pub fn delete_edge(db: &dyn Db, actor_id: &str, edge_id: &str) -> Result<(), MemoryError>;
pub fn list_labels(db: &dyn Db) -> Result<Vec<LabelCount>, MemoryError>;
pub fn graph_stats(db: &dyn Db) -> Result<GraphStats, MemoryError>;
```

---

## MCP Tool Definitions

### graph.create_edge

```rust
#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct AddEdgeParams {
    actor_id: String,
    from_memory_id: String,
    to_memory_id: String,
    label: String,
    #[serde(default)]
    properties: Option<String>,
}
```

Returns: the full `Edge` object as JSON.

### graph.get_neighbors

```rust
#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct GetNeighborsParams {
    actor_id: String,
    memory_id: String,
    /// Direction: "out" (default), "in", or "both"
    #[serde(default)]
    direction: Option<Direction>,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    limit: Option<u32>,
}
```

Returns: array of `Neighbor` objects (edge + memory).

### graph.traverse

```rust
#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct TraverseParams {
    actor_id: String,
    start_memory_id: String,
    /// Max traversal depth (default 2, max 5)
    #[serde(default)]
    max_depth: Option<u32>,
    #[serde(default)]
    label: Option<String>,
    /// Direction: "out" (default), "in", or "both"
    #[serde(default)]
    direction: Option<Direction>,
}
```

Returns: array of `TraversalNode` objects (memory + depth + path).

### graph.update_edge

```rust
#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct UpdateEdgeToolParams {
    actor_id: String,
    edge_id: String,
    #[serde(default)]
    label: Option<String>,
    /// JSON object string for edge properties. Pass null to clear.
    #[serde(default)]
    properties: Option<String>,
}
```

Returns: the updated `Edge` object.

### graph.delete_edge

```rust
#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DeleteEdgeParams {
    actor_id: String,
    edge_id: String,
}
```

Returns: `{"deleted": true}`.

### graph.list_labels

No parameters.

Returns: array of `LabelCount` objects.

### graph.get_stats

No parameters.

Returns: `GraphStats` object.

### Direction handling

`Direction` is defined in `graph.rs` with `#[serde(rename_all = "snake_case")]` and `Default` (defaults to `Out`). MCP tool params use `Option<Direction>` and unwrap with `.unwrap_or_default()`. No string parsing helper needed.

---

## Error Handling

- **Missing memory on insert_edge**: `MemoryError::NotFound("memory not found: {id}")`. Check both IDs exist and belong to the actor before INSERT.
- **Missing edge on get/update/delete**: `MemoryError::NotFound(edge_id)`. Actor scoping is enforced by joining through memories.
- **Self-edge**: `MemoryError::InvalidInput("self-edges are not allowed")`.
- **FK violation** (race condition where memory deleted between check and insert): caught by SQLite FK constraint → map to `MemoryError::QueryFailed`. Acceptable because EXCLUSIVE locking mode prevents concurrent access.
- **Traversal on nonexistent start memory**: Verify the start memory exists and belongs to the actor before running the CTE. Return `NotFound` if missing.
- **Wrong actor**: All operations scope through `memories.actor_id`. Attempting to access another actor's edges returns `NotFound` (same pattern as events/memories).

---

## Edge Cases

1. **Duplicate edges**: Allowed. Two edges with the same from/to/label can exist (different IDs). This supports multiple relationships of the same type (e.g., two "depends_on" edges with different properties).
2. **Edges to invalid memories**: Allowed. The FK references `memories(id)`, not filtered by `is_valid`. An edge to an invalidated memory still exists and is traversable. The agent can check `memory.is_valid` in results.
3. **Cascading delete**: When a memory is hard-deleted via `memory.delete_memory_record`, `ON DELETE CASCADE` removes all edges referencing it. This is the correct behavior — the memory and its relationships are gone.
4. **Consolidation**: When a memory is consolidated (update), the old memory is marked invalid but NOT deleted. Its edges remain. The new memory has no edges. The agent should re-link the new memory if needed. This is documented behavior.
5. **Empty graph**: `list_labels` returns `[]`. `graph_stats` returns `{total_edges: 0, labels: [], most_connected: []}`.
6. **Traversal cycle detection**: The recursive CTE tracks visited nodes in a JSON array. A node already in the visited set is not re-expanded. This prevents infinite loops in cyclic graphs.
7. **Traversal max visited cap**: Hard limit of 1000 visited nodes prevents runaway CTEs on dense graphs.

---

## Implementation Plan

### Task 1: Data types in graph.rs
- Define `Edge`, `Neighbor`, `TraversalNode`, `Direction`, `LabelCount`, `GraphStats`, `ConnectedMemory`, `InsertEdgeParams`, `UpdateEdgeParams`
- Add `pub mod graph;` to lib.rs

### Task 2: Db trait methods
- Uncomment and define the 8 graph methods on the `Db` trait in db.rs
- Add necessary imports (`use crate::graph::*`)

### Task 3: Db trait implementation (Connection)
- Implement all 8 methods for `Connection` in db.rs
- Add `row_to_edge` helper
- Write unit tests in db.rs for each method

### Task 4: graph.rs business logic
- Validation functions
- Public API functions that validate then delegate to Db
- Unit tests in graph.rs

### Task 5: MCP tool handlers in tools.rs
- Add 7 tool methods to `MemoryServer`
- Add param structs and `parse_direction` helper
- Add graph.rs import

### DAG

```
Task 1 ──► Task 2 ──► Task 3 ──► Task 4 ──► Task 5
```

All tasks are sequential — each depends on the previous. Task 2 needs the types from Task 1. Task 3 needs the trait signatures from Task 2. Task 4 needs the Db impl from Task 3. Task 5 needs the business logic from Task 4.

---

## Sub-Agent Instructions

### Task 1: Data types

1. Create `src/graph.rs` with the data types listed above.
2. Add `pub mod graph;` to `src/lib.rs`.
3. Run `cargo check` to verify.

### Task 2: Db trait methods

1. In `src/db.rs`, add `use crate::graph::{Edge, Neighbor, TraversalNode, Direction, LabelCount, GraphStats, ConnectedMemory, InsertEdgeParams, UpdateEdgeParams};`
2. Replace the commented-out graph methods in the `Db` trait with the real signatures from this design. All methods that operate on specific edges/memories take `actor_id: &str`.
3. Add stub implementations in `impl Db for Connection` that return `todo!()`.
4. Run `cargo check` to verify trait is object-safe and all consumers compile.

### Task 3: Db trait implementation

1. Add `row_to_edge` helper in db.rs (same pattern as `row_to_event`, `row_to_memory`).
2. Implement each Db method for Connection following the SQL in this design. All queries that access edges must join through `memories` and filter by `actor_id`.
3. For `traverse`, define 3 static SQL constants (one per direction: `SQL_TRAVERSE_OUT`, `SQL_TRAVERSE_IN`, `SQL_TRAVERSE_BOTH`). Use `match direction` to select. Do NOT build SQL dynamically. For label filtering, use 2 variants per direction (with/without label) or append the label clause. Path contains memory IDs.
4. For `update_edge`, use separate SQL statements per combination of fields being updated (label only, properties only, both). No sentinel pattern.
5. Write tests in `db.rs::tests` for:
   - `test_insert_and_get_edge`
   - `test_insert_edge_missing_memory` (NotFound)
   - `test_insert_edge_wrong_actor` (memories exist but belong to different actor)
   - `test_get_neighbors_out`
   - `test_get_neighbors_in`
   - `test_get_neighbors_both`
   - `test_get_neighbors_label_filter`
   - `test_traverse_basic` (A→B→C, depth 2)
   - `test_traverse_cycle_detection` (A→B→A)
   - `test_traverse_max_depth`
   - `test_traverse_direction`
   - `test_traverse_nonexistent_start` (NotFound)
   - `test_traverse_label_filter`
   - `test_update_edge_label`
   - `test_update_edge_properties`
   - `test_delete_edge`
   - `test_delete_edge_not_found`
   - `test_cascade_delete_memory`
   - `test_list_edge_labels`
   - `test_graph_stats`
6. Run `cargo test`.

### Task 4: graph.rs business logic

1. Add constants, validation functions, and public API functions.
2. Follow the same pattern as events.rs / memories.rs: validate → delegate to db.
3. Write tests:
   - `test_add_edge_validates_empty_label`
   - `test_add_edge_validates_self_edge`
   - `test_add_edge_validates_properties_json`
   - `test_update_edge_requires_change`
   - `test_traverse_clamps_depth`
4. Run `cargo test`.

### Task 5: MCP tool handlers

1. In `src/tools.rs`, add `use crate::graph::{self, Direction, InsertEdgeParams, UpdateEdgeParams};`
2. Add param structs: `AddEdgeParams`, `GetNeighborsParams`, `TraverseParams`, `UpdateEdgeToolParams`, `DeleteEdgeParams`. All include `actor_id`. Direction fields use `Option<Direction>` with `.unwrap_or_default()`.
3. Add 7 `#[tool(...)]` methods to `MemoryServer` following the existing pattern (call `self.run(...)`, delegate to `graph::*`).
4. Run `cargo check`, `cargo test`, `cargo clippy -- -D warnings`.

---

## Test Strategy

- **Unit tests in db.rs**: Test SQL correctness — insert, get, neighbors, traverse, update, delete, cascade, labels, stats.
- **Unit tests in graph.rs**: Test validation logic — empty fields, self-edges, JSON validation, depth clamping.
- **Unit tests in tools.rs**: Test `parse_direction` helper.
- **Build verification**: `cargo check`, `cargo test`, `cargo clippy -- -D warnings` must all pass.
