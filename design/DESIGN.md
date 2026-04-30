# local-memory-mcp — Local Agent Memory for Kiro

## Problem

AI agents are stateless. Each conversation starts fresh with no knowledge of prior interactions. You need Kiro to remember facts, preferences, context, and relationships across sessions — locally, with no cloud dependencies, no Docker, no runtime dependencies.

Amazon Bedrock AgentCore Memory solves this in the cloud. This project brings the same capabilities locally: short-term session memory, long-term extracted insights, semantic recall, and namespace isolation — all in a single compiled binary backed by SQLite. It goes further by adding a knowledge graph layer that connects memories with typed, traversable relationships.

## Solution

A compiled Rust binary that runs as an MCP server over stdio. It embeds SQLite (via `rusqlite`) with FTS5 for full-text search and `sqlite-vec` for vector similarity search. Kiro launches the binary on demand and talks to it via JSON-RPC.

---

## Design Principle: API Compatibility with AgentCore Memory

**An agent with a system prompt should be able to use either AgentCore Memory or local-memory-mcp and not know the difference.**

This is the north-star design constraint. It means:

1. **Same conceptual model**: Short-term events, long-term memories, actors, sessions, namespaces, strategies, branching, checkpointing — all work the same way. An agent prompt that says "store user preferences in the `/user/{actorId}/preferences` namespace using the `user_preference` strategy" should work identically against either backend.

2. **Same tool semantics**: The MCP tools mirror AgentCore Memory's API operations. `memory.create_event` behaves like `CreateEvent`. `memory.retrieve_memory_records` behaves like `RetrieveMemoryRecords`. `memory.create_memory_record` behaves like the result of the extraction pipeline. Parameter names, return shapes, and error semantics should be close enough that a prompt written for one works for the other without modification.

3. **Same data lifecycle**: Events are immutable. Memories are extracted from events. Memories can be consolidated (updated or invalidated). Superseded memories leave an audit trail. Namespaces organize memories hierarchically. Sessions group events chronologically.

4. **Transparent differences**: The only differences an agent might notice are:
   - **Extraction is explicit**: The agent calls `memory.create_memory_record` instead of extraction happening automatically in the background. A prompt can handle this with: "After each conversation, extract key insights and store them as memories."
   - **Embeddings are caller-provided**: The agent provides vectors instead of the server generating them. A prompt can handle this with: "When storing or recalling memories, generate an embedding for the content."
   - **Store management is local-only**: `store.switch`, `store.list`, etc. are additional tools that don't exist in AgentCore. An agent prompt can simply ignore them if targeting both backends.
   - **Knowledge graph is additive**: `graph.*` tools provide relationship traversal between memories. AgentCore Memory doesn't have this natively. An agent prompt can use graph tools when available and fall back to namespace-based organization when not.

5. **Prompt portability**: A single system prompt like the following should work against either backend:

   ```
   You have access to a memory system. Use it to:
   - Store conversation events with memory.create_event (actor_id, session_id, role, content)
   - Extract and store insights with memory.create_memory_record (actor_id, content, strategy, namespace)
   - Recall relevant memories with memory.retrieve_memory_records (actor_id, query, namespace)
   - List past sessions with memory.list_sessions (actor_id)
   - Consolidate outdated memories with memory.update_memory_record (memory_id, action)

   Strategies: 'semantic' for facts, 'summary' for session summaries, 'user_preference' for preferences.
   Organize memories in namespaces like /user/{actorId}/preferences, /user/{actorId}/facts.
   ```

   And optionally extended with graph capabilities:

   ```
   When you identify relationships between memories, link them:
   - graph.create_edge (from_memory_id, to_memory_id, label, properties?)
   - graph.get_neighbors (memory_id, direction?, label?)
   - graph.traverse (start_memory_id, max_depth?, label?, direction?)
   ```

This principle guides every tool name, parameter name, return format, and behavioral decision in the design. When in doubt, match AgentCore Memory's behavior.

---

## Design Principle: API Contracts for Parallel Development

**Components communicate through trait-based API contracts, not shared implementation details.**

This is the internal development constraint. It means:

1. **Trait boundaries between components**: The core DB layer exposes a `Db` trait that provides typed methods for each data operation (insert event, query memories, add edge, etc.). Downstream components (events.rs, memories.rs, graph.rs, search.rs) call trait methods — they never write raw SQL or access `rusqlite::Connection` directly.

2. **Parallel agent safety**: Multiple coding agents can implement different components simultaneously without conflicting. Agent A implementing events.rs and Agent B implementing memories.rs both code against the `Db` trait's method signatures. Neither needs to know the other's SQL, error handling, or query patterns. The trait is the contract.

3. **Single SQL owner**: All SQL lives in one place — the `Db` trait implementation in `db.rs`. This eliminates SQL scattered across modules, prevents duplicate/conflicting queries, and makes schema changes a single-file concern.

4. **Testability**: Components can be tested with a real SQLite `Db` implementation (in-memory or tempfile) without mocking. The trait also enables future mock implementations if needed.

5. **How it works in practice**:

   ```
   ┌─────────────┐     ┌──────────────┐     ┌─────────────┐
   │  tools.rs    │────►│  events.rs   │────►│  Db trait    │
   │  (MCP layer) │     │  memories.rs │     │  (db.rs)     │
   │              │     │  graph.rs    │     │              │
   │              │     │  search.rs   │     │  impl Db for │
   │              │     │              │     │  Connection   │
   └─────────────┘     └──────────────┘     └─────────────┘
        Component 8      Components 2-7        Component 1
   ```

   - Component 1 defines the `Db` trait with all method signatures and implements it for `rusqlite::Connection`
   - Components 2-7 accept `&dyn Db` (or `&impl Db`) and call trait methods
   - Component 8 wires everything together

6. **Contract-first development**: The `Db` trait is designed and reviewed as part of Component 1. Once approved, it becomes the stable interface that all other components depend on. Changes to the trait require updating all consumers — this is intentional friction that prevents silent breakage.

This principle guides every internal API boundary. When in doubt, add a method to the `Db` trait rather than exposing the connection.

---

## Architecture

```
┌─────────────┐     stdio (JSON-RPC)     ┌──────────────────────────┐
│   Kiro CLI   │ ◄──────────────────────► │  local-memory-mcp binary │
└─────────────┘                           │  (Rust, single binary)   │
                                          │                          │
                                          │  ┌────────────────────┐  │
                                          │  │  rmcp (MCP SDK)    │  │
                                          │  └────────┬───────────┘  │
                                          │           │              │
                                          │  ┌────────▼───────────┐  │
                                          │  │  Memory Engine     │  │
                                          │  │  (short + long     │  │
                                          │  │   term + graph)    │  │
                                          │  └────────┬───────────┘  │
                                          │           │              │
                                          │  ┌────────▼───────────┐  │
                                          │  │  SQLite + FTS5     │  │
                                          │  │  + sqlite-vec      │  │
                                          │  └────────┬───────────┘  │
                                          └───────────┼──────────────┘
                                                      │
                                                      ▼
                                          ~/.local-memory-mcp/
                                              memory.db (single file)
```

### Key decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Language | Rust | Single compiled binary, no runtime deps, fast, safe |
| Database | SQLite (rusqlite + bundled) | Embedded, single-file, battle-tested, public domain |
| Full-text search | FTS5 (built into SQLite) | BM25 ranking, prefix queries, no extra deps |
| Vector search | sqlite-vec | Embeddable vector similarity for semantic recall |
| Graph | SQLite edges table + recursive CTEs | Typed relationships between memories, multi-hop traversal without external deps |
| MCP SDK | `rmcp` crate | Official Rust MCP SDK, macro-driven tool definitions |
| Transport | stdio | Kiro's native MCP transport. Binary launched on demand. |
| Storage | Single SQLite file per memory store | Simple backup (copy file), ACID transactions across all data |

### Why SQLite over SurrealDB?

The original kiro-graph project used SurrealDB embedded with RocksDB. We pivoted for these reasons:

- **License**: SQLite is public domain. SurrealDB is BSL 1.1 with restrictions on offering as a Database Service.
- **Binary size**: SQLite adds ~2MB. SurrealDB + RocksDB adds ~30-50MB.
- **Build time**: SQLite compiles in seconds. SurrealDB + RocksDB takes minutes.
- **Simplicity**: One file per database. No directory of RocksDB SST files.
- **Maturity**: SQLite is the most deployed database in the world.
- **Sufficient for agent memory**: Graph traversal via recursive CTEs handles the relationship patterns needed for a knowledge graph (multi-hop neighbor discovery, path finding). The primary access patterns are: store events, search by text, search by vector similarity, retrieve by session/actor, traverse relationships.

---

## AgentCore Memory Feature Mapping

This project implements a local equivalent of each AgentCore Memory capability, plus a knowledge graph layer:

| AgentCore Memory Feature | Local Implementation |
|--------------------------|---------------------|
| **Short-term memory** (session events) | `events` table — immutable, ordered by timestamp, scoped by actor + session |
| **Long-term memory** (extracted insights) | `memories` table — persistent facts, preferences, summaries with embeddings |
| **Memory strategies** (semantic, summary, user_preference, custom) | Agent-driven extraction via MCP tools. The agent calls `memory.create_memory_record` with the insight and strategy label; the server stores and indexes it. |
| **Namespaces** (hierarchical organization) | `namespaces` table — hierarchical paths like `/org/user/preferences`. Supports prefix matching on retrieval. |
| **Dynamic namespace templates** ({actorId}, {sessionId}) | The agent constructs namespace paths before calling tools. The server stores the resolved path. |
| **Actor/session scoping** | All events scoped by `actor_id` + `session_id`. Memories scoped by `actor_id` + optional `namespace`. |
| **Session listing** (ListSessions) | `memory.list_sessions` tool — list distinct sessions for an actor with event counts and date ranges |
| **Event retrieval** (GetEvent) | `memory.get_event` tool — retrieve a single event by ID |
| **Event metadata** (key-value filtering) | `metadata` JSON column on events. `memory.list_events` supports filtering by metadata keys/values via JSON path queries. |
| **Semantic search** (RetrieveMemoryRecords) | `sqlite-vec` HNSW index on memory embeddings. Agent provides query vector. `memory.retrieve_memory_records` with `embedding` param. |
| **Keyword search** | FTS5 index on memory content. `memory.retrieve_memory_records` with `query` param. |
| **Get single memory** (GetMemoryRecord) | `memory.get_memory_record` tool — retrieve a single memory by ID |
| **List memories** (ListMemoryRecords) | `memory.list_memory_records` tool — list memories with filters (actor, namespace, strategy, validity) |
| **Branching** | `branches` table — fork conversation from any event (`root_event_id`), creating alternative paths. Supports message editing, what-if scenarios, and alternative approaches. |
| **Checkpointing** | `checkpoints` table — named snapshots of conversation state within a session. Used for workflow resumption and conversation bookmarks. |
| **Blob storage** | Blob events (`event_type = 'blob'`) with `blob_data` column. Used for agent state, not processed for long-term memory extraction. |
| **TTL / expiry** | `expires_at` column on events. Cleanup via `memory.delete_expired_events` tool. |
| **Consolidation** (extraction + consolidation) | `memory.update_memory_record` tool — update or invalidate memories. Immutable audit trail via `is_valid` flag (superseded memories marked invalid, not deleted). |
| **PII awareness** | Documented as agent responsibility. The server stores what the agent sends. The agent should filter PII before calling `memory.create_memory_record`. Noted in best practices. |
| **Observability** | Tracing spans on all operations. Logged to stderr. `memory.stats` tool for counts and sizes. |

### Beyond AgentCore Memory: Knowledge Graph

AgentCore Memory does not include a graph database. This project adds one as a first-class feature:

| Feature | Implementation |
|---------|---------------|
| **Typed edges between memories** | `knowledge_edges` table — labeled, directed relationships with properties |
| **Neighbor discovery** | `graph.get_neighbors` — find directly connected memories by direction and label |
| **Multi-hop traversal** | `graph.traverse` — BFS traversal via recursive CTEs, configurable depth and label filters |
| **Edge management** | `graph.create_edge`, `graph.update_edge`, `graph.delete_edge` |
| **Graph statistics** | `graph.get_stats` — edge count, label distribution |

This enables the agent to build a knowledge graph on top of its memories — linking concepts, tracking dependencies, mapping relationships between projects, people, tools, and ideas. The graph and memory systems share the same SQLite file and ACID transactions.

### What's different from AgentCore Memory

- **No managed LLM for extraction**: AgentCore Memory uses Bedrock models to automatically extract insights from events asynchronously. Locally, the agent (Kiro) performs extraction and provides the insight text via MCP tools. This keeps the server dependency-free.
- **Embeddings provided by caller**: The server doesn't generate embeddings. The agent provides embedding vectors when storing memories and query vectors when searching. This avoids bundling a model.
- **No automatic async extraction pipeline**: In AgentCore, long-term memory extraction happens automatically in the background after events are created. Here, the agent explicitly calls `memory.create_memory_record` when it has an insight to persist. The server is a storage layer, not an intelligence layer.
- **Knowledge graph is additive**: `graph.*` tools are a local-only extension. AgentCore Memory doesn't have native graph traversal. The graph tools are optional — an agent can use the memory system without them.
- **Single-user**: No IAM, no encryption at rest (relies on OS file permissions), no multi-tenant access control.
- **Local-first**: All data stays on disk. No network calls. No cloud dependency.
- **Store management is additive**: `store.switch`, `store.list`, etc. are local-only tools that don't exist in AgentCore. They extend the API without breaking compatibility — an agent prompt targeting both backends simply doesn't use them.

---

## Data Model

### Events (short-term memory)

```sql
CREATE TABLE IF NOT EXISTS events (
    id TEXT PRIMARY KEY,           -- UUID
    actor_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    event_type TEXT NOT NULL,      -- 'conversation' or 'blob'
    role TEXT,                     -- 'user', 'assistant', 'tool', 'system'
    content TEXT,                  -- message content (conversation events)
    blob_data BLOB,               -- binary content (blob events)
    metadata TEXT,                 -- JSON object for arbitrary key-value metadata
    branch_id TEXT,                -- NULL = main branch
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    expires_at TEXT                -- NULL = no expiry
);
CREATE INDEX IF NOT EXISTS idx_events_session ON events(actor_id, session_id, created_at);
CREATE INDEX IF NOT EXISTS idx_events_branch ON events(session_id, branch_id, created_at);
CREATE INDEX IF NOT EXISTS idx_events_actor ON events(actor_id, created_at);
```

### Memories (long-term memory)

```sql
CREATE TABLE IF NOT EXISTS memories (
    memory_rowid INTEGER PRIMARY KEY,  -- stable rowid for FTS5 content-sync
    id TEXT UNIQUE NOT NULL,       -- UUID (logical primary key for API/FK references)
    actor_id TEXT NOT NULL,
    namespace TEXT DEFAULT 'default',
    strategy TEXT NOT NULL,        -- 'semantic', 'summary', 'user_preference', 'custom'
    content TEXT NOT NULL,         -- the extracted insight
    metadata TEXT,                 -- JSON object
    source_session_id TEXT,        -- which session produced this memory
    is_valid INTEGER DEFAULT 1,    -- 0 = superseded (immutable audit trail)
    superseded_by TEXT,            -- ID of the memory that replaced this one
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_memories_actor ON memories(actor_id, namespace, is_valid);
CREATE INDEX IF NOT EXISTS idx_memories_strategy ON memories(strategy, is_valid);
```

### Knowledge edges (graph)

```sql
CREATE TABLE IF NOT EXISTS knowledge_edges (
    id TEXT PRIMARY KEY,           -- UUID
    from_memory_id TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
    to_memory_id TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
    label TEXT NOT NULL,           -- 'uses', 'depends_on', 'related_to', 'authored_by', etc.
    properties TEXT,               -- JSON object for edge metadata
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_edges_from ON knowledge_edges(from_memory_id, label);
CREATE INDEX IF NOT EXISTS idx_edges_to ON knowledge_edges(to_memory_id, label);
CREATE INDEX IF NOT EXISTS idx_edges_label ON knowledge_edges(label);
```

### Memory embeddings (vector search)

```sql
-- sqlite-vec virtual table for vector similarity search
-- Joins to memories via: memory_vec.memory_id = memories.id (TEXT UUID)
CREATE VIRTUAL TABLE IF NOT EXISTS memory_vec USING vec0(
    memory_id TEXT PRIMARY KEY,
    embedding float[384]           -- dimension matches embedding model
);
```

### Full-text search

```sql
-- FTS5 content-sync table
-- Joins to memories via: memory_fts.rowid = memories.memory_rowid (INTEGER)
CREATE VIRTUAL TABLE IF NOT EXISTS memory_fts USING fts5(
    content,
    content=memories,
    content_rowid=memory_rowid
);
```

**Key mapping for search.rs**: `memory_vec` uses `memories.id` (TEXT UUID) as its key. `memory_fts` uses `memories.memory_rowid` (INTEGER) as its rowid. Combined search queries must join through the `memories` table: `memory_fts.rowid = memories.memory_rowid` and `memory_vec.memory_id = memories.id`.

### Namespaces

```sql
CREATE TABLE IF NOT EXISTS namespaces (
    name TEXT PRIMARY KEY,         -- hierarchical path, e.g. '/org/user/preferences'
    description TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
```

### Checkpoints

```sql
CREATE TABLE IF NOT EXISTS checkpoints (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    actor_id TEXT NOT NULL,
    name TEXT NOT NULL,
    event_id TEXT NOT NULL,        -- the event this checkpoint points to
    metadata TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_checkpoint_name ON checkpoints(session_id, name);
```

### Branches

```sql
CREATE TABLE IF NOT EXISTS branches (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    name TEXT,                     -- optional human-readable branch name
    parent_branch_id TEXT,         -- NULL = forked from main
    root_event_id TEXT NOT NULL,   -- the event from which this branch forks
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_branches_session ON branches(session_id);
```

### Schema version

Schema version is tracked via `PRAGMA user_version` (built-in SQLite integer in the database header). No separate table needed. See `design/core-db-layer.md` for migration strategy.

---

## MCP Tools

### Short-term memory (events)

| Tool | Parameters | Description |
|------|-----------|-------------|
| `memory.create_event` | `actor_id`, `session_id`, `event_type`, `role?`, `content?`, `blob_data?`, `metadata?`, `branch_id?` | Store an immutable event |
| `memory.get_event` | `event_id` | Retrieve a single event by ID |
| `memory.list_events` | `actor_id`, `session_id`, `branch_id?`, `limit?`, `before?`, `after?`, `metadata_filter?` | Retrieve events in chronological order, optionally filtered by metadata key-value pairs |
| `memory.list_sessions` | `actor_id`, `limit?`, `offset?` | List distinct sessions for an actor with event counts and date ranges |
| `memory.delete_expired_events` | — | Remove events past their `expires_at` |

### Long-term memory

| Tool | Parameters | Description |
|------|-----------|-------------|
| `memory.create_memory_record` | `actor_id`, `content`, `strategy`, `namespace?`, `metadata?`, `source_session_id?`, `embedding?` | Store an extracted insight with optional embedding vector |
| `memory.get_memory_record` | `memory_id` | Retrieve a single memory by ID |
| `memory.retrieve_memory_records` | `actor_id`, `query?`, `embedding?`, `namespace?`, `namespace_prefix?`, `strategy?`, `limit?` | Search memories by text (FTS5) and/or vector similarity. Supports namespace prefix matching. |
| `memory.update_memory_record` | `memory_id`, `new_content?`, `new_embedding?`, `action` (update/invalidate) | Update or invalidate a memory. On update, the old memory is marked invalid with `superseded_by` pointing to the new one. |
| `memory.list_memory_records` | `actor_id`, `namespace?`, `namespace_prefix?`, `strategy?`, `valid_only?`, `limit?`, `offset?` | List memories with filters. Supports namespace prefix matching. |
| `memory.delete_memory_record` | `memory_id` | Hard-delete a memory and its edges |

### Knowledge graph (local-only, not in AgentCore)

| Tool | Parameters | Description |
|------|-----------|-------------|
| `graph.create_edge` | `from_memory_id`, `to_memory_id`, `label`, `properties?` | Create a directed, labeled relationship between two memories |
| `graph.get_neighbors` | `memory_id`, `direction?` (out/in/both), `label?`, `limit?` | Get directly connected memories |
| `graph.traverse` | `start_memory_id`, `max_depth?` (default 2, max 5), `label?`, `direction?` | Multi-hop BFS traversal via recursive CTEs |
| `graph.update_edge` | `edge_id`, `label?`, `properties?` | Update an edge's label or properties |
| `graph.delete_edge` | `edge_id` | Delete a relationship |
| `graph.list_labels` | — | List all distinct edge labels with counts |
| `graph.get_stats` | — | Edge count, label distribution, most-connected memories |

### Namespaces

| Tool | Parameters | Description |
|------|-----------|-------------|
| `memory.create_namespace` | `name`, `description?` | Create a namespace (hierarchical path like `/org/user/preferences`) |
| `memory.list_namespaces` | `prefix?` | List all namespaces, optionally filtered by path prefix |
| `memory.delete_namespace` | `name` | Delete a namespace and its memories |

### Session management

| Tool | Parameters | Description |
|------|-----------|-------------|
| `memory.create_checkpoint` | `session_id`, `actor_id`, `name`, `event_id`, `metadata?` | Create a named checkpoint at a specific event |
| `memory.create_branch` | `session_id`, `actor_id`, `root_event_id`, `name?`, `parent_branch_id?` | Fork conversation from any event, creating an alternative path |
| `memory.list_checkpoints` | `actor_id`, `session_id`, `limit?`, `offset?` | List checkpoints for a session |
| `memory.list_branches` | `actor_id`, `session_id`, `limit?`, `offset?` | List branches for a session |

### Store management (local-only, not in AgentCore)

| Tool | Parameters | Description |
|------|-----------|-------------|
| `store.switch` | `name` | Close current store, open named store (creates if new) |
| `store.current` | — | Return the name of the active store |
| `store.list` | — | List all stores with names and file sizes |
| `store.delete` | `name` | Delete a store file. Cannot delete the active store. |

### Utility

| Tool | Parameters | Description |
|------|-----------|-------------|
| `memory.stats` | `actor_id?` | Event count, memory count, edge count, session count, namespace count, DB size |
| `memory.export` | `actor_id?`, `format?` | Export memories and edges as JSON |
| `memory.import` | `data`, `format?` | Import memories and edges from JSON |

---

## Best Practices (for agent implementers)

These mirror AgentCore Memory best practices, adapted for local use:

1. **Structured memory architecture**: Use namespaces to organize memories by type (preferences, facts, summaries). Use hierarchical paths like `/project/preferences` or `/actor/{actorId}/facts`.

2. **Memory strategies**: Use `semantic` for facts and knowledge, `summary` for session summaries, `user_preference` for preferences and choices, `custom` for domain-specific insights.

3. **Efficient memory operations**: Retrieve relevant memories at the start of each interaction for context hydration. Use `memory.retrieve_memory_records` with semantic search for related memories, `memory.list_events` for recent session context, `memory.list_memory_records` with strategy filter for summaries.

4. **PII awareness**: The server stores what the agent sends. Filter PII before calling `memory.create_memory_record` if the memory shouldn't contain personal information. Blob events are not processed for long-term memory — use them for transient agent state.

5. **Consolidation rhythm**: Periodically consolidate related memories to avoid duplication. Use `memory.update_memory_record` with `action: 'update'` to merge insights, or `action: 'invalidate'` to mark outdated memories. The audit trail preserves history via `superseded_by`.

6. **Knowledge graph**: When you identify relationships between memories, link them with `graph.create_edge`. Use descriptive labels like `uses`, `depends_on`, `related_to`, `authored_by`, `part_of`. Use `graph.traverse` to discover connected knowledge — e.g., "what does this project depend on, and what do those dependencies depend on?"

---

## Multi-Store Design

Each memory store is a separate SQLite file:

```
~/.local-memory-mcp/
├── default.db        ← default memory store
├── work.db           ← additional store
├── research.db       ← additional store
└── ...
```

### Lifecycle

1. **Startup** — opens `default.db`. Creates it with schema if new.
2. **Switch** — `store.switch` closes current connection, opens named store.
3. **All tools** operate against the active store.
4. **One store open at a time** — no concurrent connections.

---

## Project Structure

```
local-memory-mcp/
├── Start_session.md
├── design/
│   ├── DESIGN.md              # This file
│   └── core-db-layer.md       # Detailed design for db.rs + store.rs
├── agents/
│   ├── WORKFLOW.md
│   ├── sec_review.md
│   ├── arch_review.md
│   ├── maint_review.md
│   ├── rel_review.md
│   ├── interop_review.md
│   ├── TODO.md
│   ├── ADR.md
│   ├── LESSONS_LEARNED.md
│   └── TIME_LOG.md
├── Cargo.toml
├── src/
│   ├── main.rs
│   ├── lib.rs
│   ├── db.rs                  # SQLite connection, schema migration
│   ├── store.rs               # StoreManager (multi-store lifecycle)
│   ├── error.rs               # Typed error enum
│   ├── events.rs              # Short-term memory operations
│   ├── memories.rs            # Long-term memory operations
│   ├── graph.rs               # Knowledge graph operations
│   ├── search.rs              # FTS5 + vector search
│   └── tools.rs               # MCP tool definitions
└── tests/
    └── integration_test.rs
```

### Key crate dependencies

```toml
[dependencies]
rmcp = { version = "1.5", features = ["transport-io", "server"] }
rusqlite = { version = "0.35", features = ["bundled", "vtab"] }
sqlite-vec = "0.1"              # vector search extension
tokio = { version = "1", features = ["macros", "rt-multi-thread", "sync"] }
thiserror = "2"
dirs = "6"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
uuid = { version = "1", features = ["v4"] }
```

---

## Kiro Integration

```bash
kiro-cli mcp add \
  --name local-memory \
  --scope global \
  --command /path/to/local-memory-mcp
```

---

## Data Location

```
~/.local-memory-mcp/
├── default.db       ← default store, opened on startup
├── <name>.db        ← additional stores
└── ...
```

Override with `LOCAL_MEMORY_HOME` env var.

---

## Performance Characteristics

- **Startup**: SQLite opens in <10ms
- **Store switch**: Close + open is <20ms
- **Event writes**: In-process, no network hop. ~1μs per insert.
- **FTS5 search**: Sub-millisecond at typical scale (thousands of memories)
- **Vector search**: sqlite-vec HNSW, sub-millisecond for <100K vectors
- **Graph traversal**: Recursive CTEs, sub-millisecond for typical depths (2-3 hops, thousands of edges)
- **Memory**: Single SQLite connection. Minimal footprint.
- **Binary size**: ~5-10MB (SQLite + sqlite-vec compiled in)

---

## Components

| # | Component | Scope |
|---|-----------|-------|
| 1 | Core DB layer | `db.rs`, `store.rs` — SQLite init, schema, store switching |
| 2 | Event tools | `events.rs`, `tools.rs` — add, get, list sessions, expire events |
| 3 | Memory tools | `memories.rs`, `tools.rs` — store, get, recall, consolidate, list, delete |
| 4 | Search | `search.rs` — FTS5 + vector search integration |
| 5 | Knowledge graph | `graph.rs`, `tools.rs` — add/update/delete edges, neighbors, traverse, stats |
| 6 | Session tools | `tools.rs` — checkpoints, branches |
| 7 | Store management tools | `tools.rs` — switch, list, delete stores |
| 8 | Namespace tools | `tools.rs` — create, list, delete namespaces |
| 9 | MCP server | `main.rs` — server init, stdio transport, shutdown |
| 10 | CI/CD | `.github/workflows/` — ci.yml, release.yml |
| 11 | Installers | `install.sh`, `install.ps1` |

---

## Agentic Coding Workflow — Research

This project is also an experiment in how to drive secure, stable, resilient, performant, and scalable software through autonomous agentic coding in a single pass. This section captures the research behind the workflow, the rationale for keeping it vendor-neutral, and the primitives we plan to adopt. The actionable backlog lives in `agents/TODO.md` under "Agentic Coding Workflow — vendor-agnostic improvements."

### Why vendor-neutral

The project is developed across multiple agent harnesses (Claude Code, Kiro CLI, and others). The first instinct is to use each harness's native primitives — `.claude/agents/`, `.claude/commands/`, `.kiro/steering/`, `.cursor/rules/`. That creates two problems:

1. **Lock-in.** Every persona, slash command, or hook has to be re-authored per harness. Switching tools means rewriting workflow infrastructure.
2. **Hidden control plane.** Dotfolders are not what reviewers look at on a PR, not what consumers see when they audit the repo, and not what `git log` makes legible. A persona prompt that gates security review is *load-bearing source code* and should live in the repo, in plain sight, under `CODEOWNERS` protection — not in a vendor's hidden side-channel.

The reframe: **prompts are first-class software artifacts.** They are versioned, reviewable, diffable, schema-bound, and authored once in the repo. Each vendor's dotfolder becomes a thin adapter that points at canonical artifacts. The substance never leaves the repo.

### Architectural shape

```
                                    .claude/   ← thin adapter (generated)
                                   /
AGENTS.md  →  agents/  ←  scripts/  →  .kiro/      ← thin adapter (generated)
(entry)      (prompts,   (orchestrate) \
              schemas,                   .cursor/   ← thin adapter (generated)
              workflows)
```

- **`AGENTS.md`** at repo root is the universal entry point for any agent.
- **`agents/`** holds the canonical artifacts: personas with frontmatter + JSON Schemas for outputs + workflow prompts + ADRs + lessons learned + time log.
- **`scripts/`** holds shell/Python orchestration. Any agent or human invokes the same scripts. The scripts pick the LLM CLI via env var.
- **Vendor dotfolders** are ≤30-line stubs generated by `scripts/render-adapters.sh`. They symlink, copy, or `exec` canonical artifacts. Adopt or drop a vendor in minutes.

### Vendor-neutral primitives

These are the smallest viable open standards the project will adopt. Each replaces what would otherwise be a vendor-specific feature.

| Primitive | Role | Replaces (vendor-specific) | Source |
|-----------|------|----------------------------|--------|
| **AGENTS.md** | Universal entry point read by Codex, Gemini CLI, Devin, Windsurf, Jules, Cursor, VS Code, Zed, Aider, Warp, Roo Code, Factory, **Kiro CLI**. Linux Foundation stewarded (Agentic AI Foundation). | `CLAUDE.md` only, `.kiro/steering/`-only, `.cursor/rules/`-only entry docs | [agents.md](https://agents.md/) |
| **Personas as versioned, schema-bound markdown** | Each `agents/personas/*_review.md` carries YAML frontmatter (`name`, `version`, `description`, `output_schema`, `model_hint`) and produces JSON conforming to `agents/schemas/finding.schema.json`. | `.claude/agents/`-format only personas | [JSON Schema](https://json-schema.org/), [Persona project](https://github.com/JasperHG90/persona) |
| **Shell scripts + `justfile`** | `scripts/review.sh`, `scripts/design-review.sh`, `scripts/code-review.sh`, `scripts/aggregate-findings.py`. Model is selected via `LLM_MODEL` env var. Both humans and any agent harness call the same scripts. | `.claude/commands/`-format slash commands | [casey/just](https://github.com/casey/just), [Simon Willison's `llm`](https://llm.datasette.io/) |
| **MCP** for tool surfaces (`.mcp.json` at repo root) | Vendor-neutral tool protocol; donated to Linux Foundation December 2025; 10,000+ public servers as of March 2026. This project is itself an MCP server — eat the dog food. | Vendor-private tool registries | [MCP spec](https://modelcontextprotocol.io/specification/2025-11-25), [MCP on GitHub](https://github.com/modelcontextprotocol/modelcontextprotocol) |
| **Lefthook** | Cross-vendor git hooks (single Go binary, no runtime deps, parallel execution). Same hook config runs identically regardless of editor or harness. | `.claude/settings.json` `hooks`, `.kiro/hooks/`-only, husky, vendor-specific harness hooks | [evilmartians/lefthook](https://github.com/evilmartians/lefthook) |
| **promptfoo** | Vendor-agnostic regression tests for the personas themselves. Tests can compare behavior across Claude, GPT, Gemini, Llama. | Vendor-specific evaluation tooling | [promptfoo.dev](https://www.promptfoo.dev/docs/intro/), [GitHub](https://github.com/promptfoo/promptfoo) |
| **inspect-ai (UK AISI)** | Adversarial evals (CodeIPI for prompt-injection resilience of personas) and full-loop agent evals against `local-memory-mcp` itself in a Docker sandbox. | Vendor-private safety tooling | [inspect.aisi.org.uk](https://inspect.aisi.org.uk/), [GitHub](https://github.com/UKGovernmentBEIS/inspect_ai), [inspect_evals](https://github.com/UKGovernmentBEIS/inspect_evals) |
| **OpenTelemetry GenAI conventions + OpenLLMetry** | One trace covering agent harness LLM calls *and* MCP server tool calls. OTLP export to any backend. | Vendor-private telemetry consoles | [OTel LLM blog](https://opentelemetry.io/blog/2024/llm-observability/), [OpenLLMetry](https://github.com/traceloop/openllmetry) |
| **Conventional commits + git-cliff** | Drives `CHANGELOG.md` generation and gives the agent unambiguous semver guidance. | GitHub `--generate-notes` only | `git-cliff` |

### Proposed repository layout

```
local-memory-mcp/
├── AGENTS.md                          # universal entry point (NEW)
├── agents/
│   ├── personas/                      # frontmatter + body
│   │   ├── sec_review.md
│   │   ├── arch_review.md
│   │   ├── maint_review.md
│   │   ├── rel_review.md
│   │   └── interop_review.md
│   ├── workflows/                     # design_review.md, code_review.md, close_component.md
│   ├── schemas/finding.schema.json
│   ├── ADR.md, LESSONS_LEARNED.md, PERSONA_IMPROVEMENTS.md, TIME_LOG.md, WORKFLOW.md
├── scripts/
│   ├── review.sh
│   ├── design-review.sh
│   ├── code-review.sh
│   ├── close-component.sh
│   ├── aggregate-findings.py
│   └── render-adapters.sh             # generates vendor dotfolders
├── components.toml                    # typed component-state (replaces parts of TODO.md)
├── lefthook.yml
├── justfile
├── deny.toml
├── promptfoo.yaml
├── inspect/codeipi.eval.py
├── .mcp.json                          # MCP servers used by this repo
├── .github/CODEOWNERS                 # protects agents/personas/, scripts/, schemas/
└── .github/workflows/agent-review.yml # runs scripts/code-review.sh on PRs
```

Vendor adapters (`.claude/`, `.kiro/`, `.cursor/`) stay in the repo but are tiny and generated.

### Vendor adapter pattern

Each harness retains its UX wins, but only as adapters over canonical artifacts:

- **`.claude/`** — `settings.json` allowlists + hooks that exec `lefthook` or `scripts/*`. `agents/` and `commands/` are auto-generated wrappers around `agents/personas/` and `scripts/`.
- **`.kiro/`** — `steering/AGENTS.md` symlinks to repo root. `mcp.json` symlinks to `.mcp.json`. `hooks/*.yml` exec `scripts/*` at lifecycle events (`agentSpawn`, `userPromptSubmit`, `preToolUse`).
- **`.cursor/rules/*.mdc`** — generated from `agents/personas/*.md` by `scripts/render-adapters.sh`.
- **Codex / Gemini CLI / Aider / Windsurf / Devin** — read `AGENTS.md` natively; zero adapter needed.

### Safety argument

Five concrete safety wins from moving prompts out of dotfolders and into the repo:

1. **CODEOWNERS protects personas.** A contributor (or compromised dependency) cannot quietly weaken `sec_review.md` because every change goes through PR review like any source file. Today, an attacker editing `.claude/agents/sec-reviewer.md` to "always return zero findings" would be invisible to other reviewers because dotfolders aren't typically covered by CODEOWNERS.
2. **`git log` legibility.** `git log --follow agents/personas/sec_review.md` shows the entire prompt-evolution history. Today the same content fragments across `.claude/`, `.cursor/`, and `.kiro/`, often manually copy-pasted, with drift you can't even detect.
3. **Auditable prompt-injection resistance.** When personas are repo-visible, you can run `inspect-ai`'s CodeIPI eval against the actual prompts. When they're hidden in `.claude/`, you can't audit how robust they are.
4. **Trust verification at install time.** A consumer of `local-memory-mcp` can read `agents/` and verify what gates the build. They cannot audit `.claude/`. This matters because this project is a security-relevant tool (memory store often holding sensitive context).
5. **No silent vendor drift.** When a harness changes its subagent format upstream, your generated `.claude/agents/*.md` or `.kiro/steering/*.md` files might silently take on new semantics. With canonical artifacts + a generator, you control the translation layer and surface vendor changes as PRs to the generator.

The same argument applies to **agent telemetry**: vendor-private telemetry (Anthropic console, Kiro telemetry) means a regression in agent behavior may be visible only to that vendor. OpenTelemetry/OpenLLMetry puts the trace data in *your* observability stack, where any team can audit it.

### What stays from the current setup

The structured workflow this project already has is closer to the right shape than most agentic projects. These are kept and formalized, not replaced:

- **Five-persona review gating** (`agents/sec_review.md` etc. → `agents/personas/sec_review.md` with frontmatter)
- **ADR culture** (`agents/ADR.md`) — institutional memory that prevents re-litigating decisions
- **Living tracking files** (TODO, LESSONS_LEARNED, TIME_LOG, PERSONA_IMPROVEMENTS) — updated *during* work, not after
- **Component-first contract development** (the `Db` trait as the parallel-safe API contract; ADR-007)
- **Severity-driven backlog** (Critical/High block; Medium/Low feed `TODO.md`)
- **Lessons-learned → persona-improvements meta-loop**
- **CI/CD hygiene basics** (SHA-pinned actions, per-job least-privilege `permissions:`, `--locked` builds, draft-then-publish atomicity, SHA256SUMS, 4-target build matrix)

### Recommended adoption sequence

The shortest path through the backlog. Each step is independently shippable.

1. **`AGENTS.md` at repo root + symlink `CLAUDE.md` → `AGENTS.md`** (~15 minutes). Picked up automatically by Kiro CLI, Codex, Cursor, Aider, Windsurf, Devin, Gemini CLI.
2. **Frontmatter on personas + `agents/schemas/finding.schema.json`** (~1 hour). Formalizes what is already there.
3. **`scripts/review.sh`, `scripts/design-review.sh`, `scripts/code-review.sh`, `scripts/aggregate-findings.py`, `justfile`** (~3 hours). Both Claude Code and Kiro now invoke the same logic.
4. **`lefthook.yml`, `cargo-deny`, `dependabot.yml`, `CODEOWNERS` covering `agents/personas/`** (~half a day). Closes supply-chain and agent-tampering attack surfaces.
5. **`.github/workflows/agent-review.yml` running `scripts/code-review.sh` with configurable `LLM_MODEL`** (~1 hour). Gate enforcement without vendor lock.

After that, vendor adapters in `.claude/`, `.kiro/`, `.cursor/` are stub files generated by `scripts/render-adapters.sh`. Each ≤30 lines.

### References

- [agents.md — universal entry-point spec](https://agents.md/)
- [Codex AGENTS.md guide (OpenAI Developers)](https://developers.openai.com/codex/guides/agents-md)
- [Augment Code — How to build AGENTS.md (2026)](https://www.augmentcode.com/guides/how-to-build-agents-md)
- [Kiro CLI steering docs](https://kiro.dev/docs/cli/steering/)
- [Kiro CLI hooks docs](https://kiro.dev/docs/cli/hooks/)
- [Kiro CLI custom agents reference](https://kiro.dev/docs/cli/custom-agents/configuration-reference/)
- [Model Context Protocol specification (2025-11-25)](https://modelcontextprotocol.io/specification/2025-11-25)
- [Model Context Protocol on GitHub](https://github.com/modelcontextprotocol/modelcontextprotocol)
- [.mcp.json configuration guide](https://www.claudemdeditor.com/mcp-json-guide)
- [The Complete Guide to MCP in 2026](https://www.essamamdani.com/blog/complete-guide-model-context-protocol-mcp-2026)
- [Lefthook — cross-vendor git hooks](https://github.com/evilmartians/lefthook)
- [Lefthook docs](https://lefthook.dev/)
- [promptfoo — vendor-agnostic prompt eval](https://github.com/promptfoo/promptfoo)
- [promptfoo: evaluating coding agents](https://www.promptfoo.dev/docs/guides/evaluate-coding-agents/)
- [inspect-ai — UK AISI evaluation framework](https://github.com/UKGovernmentBEIS/inspect_ai)
- [inspect_evals — community evals incl. CodeIPI](https://github.com/UKGovernmentBEIS/inspect_evals)
- [Inspect AI homepage](https://inspect.aisi.org.uk/)
- [OpenTelemetry — LLM observability](https://opentelemetry.io/blog/2024/llm-observability/)
- [OpenLLMetry (Traceloop)](https://github.com/traceloop/openllmetry)
- [openlit — OpenTelemetry-native LLM observability](https://github.com/openlit/openlit)
- [Persona — prompts as portable software artifacts](https://github.com/JasperHG90/persona)
- [casey/just — task runner](https://github.com/casey/just)
- [Simon Willison's `llm` — provider-agnostic LLM CLI](https://llm.datasette.io/)
- [CLAUDE.md, AGENTS.md, and Every AI Config File Explained](https://dev.to/deployhq/claudemd-agentsmd-and-every-ai-config-file-explained-4pde)

---

## Future Considerations (not in MVP)

- **Local embedding model**: Bundle a small ONNX model (all-MiniLM-L6-v2) via `ort` crate so the server can generate embeddings without the agent providing vectors
- **Automatic extraction**: On-device LLM to automatically extract insights from events (like AgentCore's managed async extraction pipeline)
- **Import from AgentCore**: Import/export format compatible with AgentCore Memory API
- **Encryption at rest**: SQLite Encryption Extension or sqlcipher
- **Web UI**: Local web interface for browsing and visualizing memories and the knowledge graph
- **Multi-agent**: Actor-based isolation for multi-agent systems sharing a memory store
- **Custom strategy prompts**: Store extraction/consolidation prompt templates per strategy, for use when local LLM extraction is available
- **Graph algorithms**: PageRank, shortest path, community detection on the knowledge graph
