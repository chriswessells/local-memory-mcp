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

2. **Same tool semantics**: The MCP tools mirror AgentCore Memory's API operations. `memory.add_event` behaves like `CreateEvent`. `memory.recall` behaves like `RetrieveMemoryRecords`. `memory.store` behaves like the result of the extraction pipeline. Parameter names, return shapes, and error semantics should be close enough that a prompt written for one works for the other without modification.

3. **Same data lifecycle**: Events are immutable. Memories are extracted from events. Memories can be consolidated (updated or invalidated). Superseded memories leave an audit trail. Namespaces organize memories hierarchically. Sessions group events chronologically.

4. **Transparent differences**: The only differences an agent might notice are:
   - **Extraction is explicit**: The agent calls `memory.store` instead of extraction happening automatically in the background. A prompt can handle this with: "After each conversation, extract key insights and store them as memories."
   - **Embeddings are caller-provided**: The agent provides vectors instead of the server generating them. A prompt can handle this with: "When storing or recalling memories, generate an embedding for the content."
   - **Store management is local-only**: `memory.switch_store`, `memory.list_stores`, etc. are additional tools that don't exist in AgentCore. An agent prompt can simply ignore them if targeting both backends.
   - **Knowledge graph is additive**: `graph.*` tools provide relationship traversal between memories. AgentCore Memory doesn't have this natively. An agent prompt can use graph tools when available and fall back to namespace-based organization when not.

5. **Prompt portability**: A single system prompt like the following should work against either backend:

   ```
   You have access to a memory system. Use it to:
   - Store conversation events with memory.add_event (actor_id, session_id, role, content)
   - Extract and store insights with memory.store (actor_id, content, strategy, namespace)
   - Recall relevant memories with memory.recall (actor_id, query, namespace)
   - List past sessions with memory.list_sessions (actor_id)
   - Consolidate outdated memories with memory.consolidate (memory_id, action)

   Strategies: 'semantic' for facts, 'summary' for session summaries, 'user_preference' for preferences.
   Organize memories in namespaces like /user/{actorId}/preferences, /user/{actorId}/facts.
   ```

   And optionally extended with graph capabilities:

   ```
   When you identify relationships between memories, link them:
   - graph.add_edge (from_memory_id, to_memory_id, label, properties?)
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
| **Memory strategies** (semantic, summary, user_preference, custom) | Agent-driven extraction via MCP tools. The agent calls `memory.store` with the insight and strategy label; the server stores and indexes it. |
| **Namespaces** (hierarchical organization) | `namespaces` table — hierarchical paths like `/org/user/preferences`. Supports prefix matching on retrieval. |
| **Dynamic namespace templates** ({actorId}, {sessionId}) | The agent constructs namespace paths before calling tools. The server stores the resolved path. |
| **Actor/session scoping** | All events scoped by `actor_id` + `session_id`. Memories scoped by `actor_id` + optional `namespace`. |
| **Session listing** (ListSessions) | `memory.list_sessions` tool — list distinct sessions for an actor with event counts and date ranges |
| **Event retrieval** (GetEvent) | `memory.get_event` tool — retrieve a single event by ID |
| **Event metadata** (key-value filtering) | `metadata` JSON column on events. `memory.get_events` supports filtering by metadata keys/values via JSON path queries. |
| **Semantic search** (RetrieveMemoryRecords) | `sqlite-vec` HNSW index on memory embeddings. Agent provides query vector. `memory.recall` with `embedding` param. |
| **Keyword search** | FTS5 index on memory content. `memory.recall` with `query` param. |
| **Get single memory** (GetMemoryRecord) | `memory.get` tool — retrieve a single memory by ID |
| **List memories** (ListMemoryRecords) | `memory.list` tool — list memories with filters (actor, namespace, strategy, validity) |
| **Branching** | `branches` table — fork conversation from any event (`root_event_id`), creating alternative paths. Supports message editing, what-if scenarios, and alternative approaches. |
| **Checkpointing** | `checkpoints` table — named snapshots of conversation state within a session. Used for workflow resumption and conversation bookmarks. |
| **Blob storage** | Blob events (`event_type = 'blob'`) with `blob_data` column. Used for agent state, not processed for long-term memory extraction. |
| **TTL / expiry** | `expires_at` column on events. Cleanup via `memory.delete_expired` tool. |
| **Consolidation** (extraction + consolidation) | `memory.consolidate` tool — update or invalidate memories. Immutable audit trail via `is_valid` flag (superseded memories marked invalid, not deleted). |
| **PII awareness** | Documented as agent responsibility. The server stores what the agent sends. The agent should filter PII before calling `memory.store`. Noted in best practices. |
| **Observability** | Tracing spans on all operations. Logged to stderr. `memory.stats` tool for counts and sizes. |

### Beyond AgentCore Memory: Knowledge Graph

AgentCore Memory does not include a graph database. This project adds one as a first-class feature:

| Feature | Implementation |
|---------|---------------|
| **Typed edges between memories** | `knowledge_edges` table — labeled, directed relationships with properties |
| **Neighbor discovery** | `graph.get_neighbors` — find directly connected memories by direction and label |
| **Multi-hop traversal** | `graph.traverse` — BFS traversal via recursive CTEs, configurable depth and label filters |
| **Edge management** | `graph.add_edge`, `graph.update_edge`, `graph.delete_edge` |
| **Graph statistics** | `graph.stats` — edge count, label distribution |

This enables the agent to build a knowledge graph on top of its memories — linking concepts, tracking dependencies, mapping relationships between projects, people, tools, and ideas. The graph and memory systems share the same SQLite file and ACID transactions.

### What's different from AgentCore Memory

- **No managed LLM for extraction**: AgentCore Memory uses Bedrock models to automatically extract insights from events asynchronously. Locally, the agent (Kiro) performs extraction and provides the insight text via MCP tools. This keeps the server dependency-free.
- **Embeddings provided by caller**: The server doesn't generate embeddings. The agent provides embedding vectors when storing memories and query vectors when searching. This avoids bundling a model.
- **No automatic async extraction pipeline**: In AgentCore, long-term memory extraction happens automatically in the background after events are created. Here, the agent explicitly calls `memory.store` when it has an insight to persist. The server is a storage layer, not an intelligence layer.
- **Knowledge graph is additive**: `graph.*` tools are a local-only extension. AgentCore Memory doesn't have native graph traversal. The graph tools are optional — an agent can use the memory system without them.
- **Single-user**: No IAM, no encryption at rest (relies on OS file permissions), no multi-tenant access control.
- **Local-first**: All data stays on disk. No network calls. No cloud dependency.
- **Store management is additive**: `memory.switch_store`, `memory.list_stores`, etc. are local-only tools that don't exist in AgentCore. They extend the API without breaking compatibility — an agent prompt targeting both backends simply doesn't use them.

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
| `memory.add_event` | `actor_id`, `session_id`, `event_type`, `role?`, `content?`, `blob_data?`, `metadata?`, `branch_id?` | Store an immutable event |
| `memory.get_event` | `event_id` | Retrieve a single event by ID |
| `memory.get_events` | `actor_id`, `session_id`, `branch_id?`, `limit?`, `before?`, `after?`, `metadata_filter?` | Retrieve events in chronological order, optionally filtered by metadata key-value pairs |
| `memory.list_sessions` | `actor_id`, `limit?`, `offset?` | List distinct sessions for an actor with event counts and date ranges |
| `memory.delete_expired` | — | Remove events past their `expires_at` |

### Long-term memory

| Tool | Parameters | Description |
|------|-----------|-------------|
| `memory.store` | `actor_id`, `content`, `strategy`, `namespace?`, `metadata?`, `source_session_id?`, `embedding?` | Store an extracted insight with optional embedding vector |
| `memory.get` | `memory_id` | Retrieve a single memory by ID |
| `memory.recall` | `actor_id`, `query?`, `embedding?`, `namespace?`, `namespace_prefix?`, `strategy?`, `limit?` | Search memories by text (FTS5) and/or vector similarity. Supports namespace prefix matching. |
| `memory.consolidate` | `memory_id`, `new_content?`, `new_embedding?`, `action` (update/invalidate) | Update or invalidate a memory. On update, the old memory is marked invalid with `superseded_by` pointing to the new one. |
| `memory.list` | `actor_id`, `namespace?`, `namespace_prefix?`, `strategy?`, `valid_only?`, `limit?`, `offset?` | List memories with filters. Supports namespace prefix matching. |
| `memory.delete` | `memory_id` | Hard-delete a memory and its edges |

### Knowledge graph (local-only, not in AgentCore)

| Tool | Parameters | Description |
|------|-----------|-------------|
| `graph.add_edge` | `from_memory_id`, `to_memory_id`, `label`, `properties?` | Create a directed, labeled relationship between two memories |
| `graph.get_neighbors` | `memory_id`, `direction?` (out/in/both), `label?`, `limit?` | Get directly connected memories |
| `graph.traverse` | `start_memory_id`, `max_depth?` (default 2, max 5), `label?`, `direction?` | Multi-hop BFS traversal via recursive CTEs |
| `graph.update_edge` | `edge_id`, `label?`, `properties?` | Update an edge's label or properties |
| `graph.delete_edge` | `edge_id` | Delete a relationship |
| `graph.list_labels` | — | List all distinct edge labels with counts |
| `graph.stats` | — | Edge count, label distribution, most-connected memories |

### Namespaces

| Tool | Parameters | Description |
|------|-----------|-------------|
| `memory.create_namespace` | `name`, `description?` | Create a namespace (hierarchical path like `/org/user/preferences`) |
| `memory.list_namespaces` | `prefix?` | List all namespaces, optionally filtered by path prefix |
| `memory.delete_namespace` | `name` | Delete a namespace and its memories |

### Session management

| Tool | Parameters | Description |
|------|-----------|-------------|
| `memory.checkpoint` | `session_id`, `actor_id`, `name`, `event_id`, `metadata?` | Create a named checkpoint at a specific event |
| `memory.branch` | `session_id`, `root_event_id`, `name?`, `parent_branch_id?` | Fork conversation from any event, creating an alternative path |
| `memory.list_checkpoints` | `session_id` | List checkpoints for a session |
| `memory.list_branches` | `session_id` | List branches for a session |

### Store management (local-only, not in AgentCore)

| Tool | Parameters | Description |
|------|-----------|-------------|
| `memory.switch_store` | `name` | Close current store, open named store (creates if new) |
| `memory.current_store` | — | Return the name of the active store |
| `memory.list_stores` | — | List all stores with names and file sizes |
| `memory.delete_store` | `name` | Delete a store file. Cannot delete the active store. |

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

3. **Efficient memory operations**: Retrieve relevant memories at the start of each interaction for context hydration. Use `memory.recall` with semantic search for related memories, `memory.get_events` for recent session context, `memory.list` with strategy filter for summaries.

4. **PII awareness**: The server stores what the agent sends. Filter PII before calling `memory.store` if the memory shouldn't contain personal information. Blob events are not processed for long-term memory — use them for transient agent state.

5. **Consolidation rhythm**: Periodically consolidate related memories to avoid duplication. Use `memory.consolidate` with `action: 'update'` to merge insights, or `action: 'invalidate'` to mark outdated memories. The audit trail preserves history via `superseded_by`.

6. **Knowledge graph**: When you identify relationships between memories, link them with `graph.add_edge`. Use descriptive labels like `uses`, `depends_on`, `related_to`, `authored_by`, `part_of`. Use `graph.traverse` to discover connected knowledge — e.g., "what does this project depend on, and what do those dependencies depend on?"

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
2. **Switch** — `memory.switch_store` closes current connection, opens named store.
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

## Future Considerations (not in MVP)

- **Local embedding model**: Bundle a small ONNX model (all-MiniLM-L6-v2) via `ort` crate so the server can generate embeddings without the agent providing vectors
- **Automatic extraction**: On-device LLM to automatically extract insights from events (like AgentCore's managed async extraction pipeline)
- **Import from AgentCore**: Import/export format compatible with AgentCore Memory API
- **Encryption at rest**: SQLite Encryption Extension or sqlcipher
- **Web UI**: Local web interface for browsing and visualizing memories and the knowledge graph
- **Multi-agent**: Actor-based isolation for multi-agent systems sharing a memory store
- **Custom strategy prompts**: Store extraction/consolidation prompt templates per strategy, for use when local LLM extraction is available
- **Graph algorithms**: PageRank, shortest path, community detection on the knowledge graph
