# local-memory-mcp — Local Agent Memory for Kiro

## Problem

AI agents are stateless. Each conversation starts fresh with no knowledge of prior interactions. You need Kiro to remember facts, preferences, context, and relationships across sessions — locally, with no cloud dependencies, no Docker, no runtime dependencies.

Amazon Bedrock AgentCore Memory solves this in the cloud. This project brings the same capabilities locally: short-term session memory, long-term extracted insights, semantic recall, and namespace isolation — all in a single compiled binary backed by SQLite.

## Solution

A compiled Rust binary that runs as an MCP server over stdio. It embeds SQLite (via `rusqlite`) with FTS5 for full-text search and `sqlite-vec` for vector similarity search. Kiro launches the binary on demand and talks to it via JSON-RPC.

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
                                          │  │   term memory)     │  │
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
- **Sufficient for agent memory**: Agent memory doesn't need native graph traversal. Relationships between memories are simple enough for an edges table + recursive CTEs. The primary access patterns are: store events, search by text, search by vector similarity, retrieve by session/actor.

---

## AgentCore Memory Feature Mapping

This project implements a local equivalent of each AgentCore Memory capability:

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

### What's different from AgentCore Memory

- **No managed LLM for extraction**: AgentCore Memory uses Bedrock models to automatically extract insights from events asynchronously. Locally, the agent (Kiro) performs extraction and provides the insight text via MCP tools. This keeps the server dependency-free.
- **Embeddings provided by caller**: The server doesn't generate embeddings. The agent provides embedding vectors when storing memories and query vectors when searching. This avoids bundling a model.
- **No automatic async extraction pipeline**: In AgentCore, long-term memory extraction happens automatically in the background after events are created. Here, the agent explicitly calls `memory.store` when it has an insight to persist. The server is a storage layer, not an intelligence layer.
- **Single-user**: No IAM, no encryption at rest (relies on OS file permissions), no multi-tenant access control.
- **Local-first**: All data stays on disk. No network calls. No cloud dependency.

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
    id TEXT PRIMARY KEY,           -- UUID
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

### Memory embeddings (vector search)

```sql
-- sqlite-vec virtual table for vector similarity search
CREATE VIRTUAL TABLE IF NOT EXISTS memory_vec USING vec0(
    memory_id TEXT PRIMARY KEY,
    embedding float[384]           -- dimension matches embedding model
);
```

### Full-text search

```sql
CREATE VIRTUAL TABLE IF NOT EXISTS memory_fts USING fts5(
    content,
    content=memories,
    content_rowid=rowid
);
```

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

```sql
CREATE TABLE IF NOT EXISTS _meta (
    key TEXT PRIMARY KEY,
    value TEXT
);
```

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
| `memory.delete` | `memory_id` | Hard-delete a memory |

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

### Store management

| Tool | Parameters | Description |
|------|-----------|-------------|
| `memory.switch_store` | `name` | Close current store, open named store (creates if new) |
| `memory.current_store` | — | Return the name of the active store |
| `memory.list_stores` | — | List all stores with names and file sizes |
| `memory.delete_store` | `name` | Delete a store file. Cannot delete the active store. |

### Utility

| Tool | Parameters | Description |
|------|-----------|-------------|
| `memory.stats` | `actor_id?` | Event count, memory count, session count, namespace count, DB size |
| `memory.export` | `actor_id?`, `format?` | Export memories as JSON |
| `memory.import` | `data`, `format?` | Import memories from JSON |

---

## Best Practices (for agent implementers)

These mirror AgentCore Memory best practices, adapted for local use:

1. **Structured memory architecture**: Use namespaces to organize memories by type (preferences, facts, summaries). Use hierarchical paths like `/project/preferences` or `/actor/{actorId}/facts`.

2. **Memory strategies**: Use `semantic` for facts and knowledge, `summary` for session summaries, `user_preference` for preferences and choices, `custom` for domain-specific insights.

3. **Efficient memory operations**: Retrieve relevant memories at the start of each interaction for context hydration. Use `memory.recall` with semantic search for related memories, `memory.get_events` for recent session context, `memory.list` with strategy filter for summaries.

4. **PII awareness**: The server stores what the agent sends. Filter PII before calling `memory.store` if the memory shouldn't contain personal information. Blob events are not processed for long-term memory — use them for transient agent state.

5. **Consolidation rhythm**: Periodically consolidate related memories to avoid duplication. Use `memory.consolidate` with `action: 'update'` to merge insights, or `action: 'invalidate'` to mark outdated memories. The audit trail preserves history via `superseded_by`.

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
| 5 | Session tools | `tools.rs` — checkpoints, branches |
| 6 | Store management tools | `tools.rs` — switch, list, delete stores |
| 7 | Namespace tools | `tools.rs` — create, list, delete namespaces |
| 8 | MCP server | `main.rs` — server init, stdio transport, shutdown |
| 9 | CI/CD | `.github/workflows/` — ci.yml, release.yml |
| 10 | Installers | `install.sh`, `install.ps1` |

---

## Future Considerations (not in MVP)

- **Local embedding model**: Bundle a small ONNX model (all-MiniLM-L6-v2) via `ort` crate so the server can generate embeddings without the agent providing vectors
- **Automatic extraction**: On-device LLM to automatically extract insights from events (like AgentCore's managed async extraction pipeline)
- **Graph relationships**: Add an edges table linking memories to each other for relationship traversal
- **Import from AgentCore**: Import/export format compatible with AgentCore Memory API
- **Encryption at rest**: SQLite Encryption Extension or sqlcipher
- **Web UI**: Local web interface for browsing and visualizing memories
- **Multi-agent**: Actor-based isolation for multi-agent systems sharing a memory store
- **Custom strategy prompts**: Store extraction/consolidation prompt templates per strategy, for use when local LLM extraction is available
