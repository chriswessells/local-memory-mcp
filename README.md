# local-memory-mcp

Local agent memory MCP server — SQLite-backed short-term and long-term memory for AI agents, inspired by [Amazon Bedrock AgentCore Memory](https://docs.aws.amazon.com/bedrock/latest/userguide/agents-memory.html).

A single compiled Rust binary that runs as an MCP server over stdio. Embeds SQLite with FTS5 for full-text search and [sqlite-vec](https://github.com/asg017/sqlite-vec) for vector similarity search. No cloud dependencies, no Docker, no runtime dependencies.

## Status

**Early development** — Components 1–4 of 11 are complete and merged. Not yet usable as an MCP server.

| Component | Status |
|-----------|--------|
| Core DB layer (schema, migrations, store management) | ✅ Done |
| Event tools (short-term memory CRUD) | ✅ Done |
| Memory tools (long-term memory CRUD) | ✅ Done |
| Search (FTS5 + vector + hybrid RRF) | ✅ Done |
| Knowledge graph | 🔲 Planned |
| Session tools (checkpoints, branches) | 🔲 Planned |
| Store management tools | 🔲 Planned |
| Namespace tools | 🔲 Planned |
| MCP server (stdio transport) | 🔲 Planned |
| CI/CD | 🔲 Planned |
| Installers | 🔲 Planned |

## Features

- **Short-term memory** — Immutable conversation events scoped by actor and session, with optional TTL expiry
- **Long-term memory** — Extracted insights stored with strategies (semantic, summary, user_preference, custom) and namespace organization
- **Full-text search** — FTS5 BM25-ranked keyword search over memory content
- **Vector similarity search** — sqlite-vec KNN search over caller-provided embeddings
- **Hybrid search** — Reciprocal Rank Fusion (RRF) combining FTS5 and vector results
- **Memory consolidation** — Update or invalidate memories with an immutable audit trail (superseded_by chain)
- **Multi-store isolation** — Each memory store is a separate SQLite file, independently switchable
- **Namespace hierarchy** — Organize memories in paths like `/user/{actorId}/preferences`
- **Knowledge graph** (planned) — Typed, directed edges between memories with multi-hop traversal
- **Session management** (planned) — Checkpoints and branches for conversation forking

## Design Principle: AgentCore Memory Compatibility

An agent with a system prompt should be able to use either AgentCore Memory or local-memory-mcp and not know the difference.

Same conceptual model, same tool semantics, same data lifecycle. The only transparent differences:

- **Extraction is explicit** — The agent calls `memory.store` instead of extraction happening automatically
- **Embeddings are caller-provided** — The agent provides vectors; the server stores and indexes them
- **Store management is additive** — `memory.switch_store`, `memory.list_stores` are local-only extensions
- **Knowledge graph is additive** — `graph.*` tools are a local-only extension

## Architecture

```
┌─────────────┐     stdio (JSON-RPC)     ┌──────────────────────────┐
│   Kiro CLI   │ ◄──────────────────────► │  local-memory-mcp binary │
└─────────────┘                           │                          │
                                          │  rmcp (MCP SDK)          │
                                          │  Memory Engine           │
                                          │  SQLite + FTS5           │
                                          │  + sqlite-vec            │
                                          └──────────┬───────────────┘
                                                     │
                                                     ▼
                                          ~/.local-memory-mcp/
                                              default.db
                                              work.db
                                              ...
```

| Choice | Rationale |
|--------|-----------|
| Rust | Single compiled binary, no runtime deps |
| SQLite (rusqlite, bundled) | Embedded, single-file, ACID, public domain |
| FTS5 | BM25 ranking, prefix queries, built into SQLite |
| sqlite-vec | Embeddable vector similarity search |
| rmcp | Official Rust MCP SDK |
| stdio transport | Kiro's native MCP transport |

Each memory store is a separate `.db` file under `~/.local-memory-mcp/`. One store open at a time. Full isolation, portable, independently deletable.

## MCP Tools

### Short-term memory (events)

| Tool | Description |
|------|-------------|
| `memory.add_event` | Store an immutable conversation or blob event |
| `memory.get_event` | Retrieve a single event by ID |
| `memory.get_events` | Retrieve events for an actor+session with filters |
| `memory.list_sessions` | List distinct sessions with event counts and date ranges |
| `memory.delete_expired` | Remove events past their TTL |

### Long-term memory

| Tool | Description |
|------|-------------|
| `memory.store` | Store an extracted insight with optional embedding |
| `memory.get` | Retrieve a single memory by ID |
| `memory.recall` | Search by text (FTS5), vector similarity, or hybrid |
| `memory.consolidate` | Update or invalidate a memory (immutable audit trail) |
| `memory.list` | List memories with namespace, strategy, and validity filters |
| `memory.delete` | Hard-delete a memory and its edges |

### Knowledge graph (planned)

| Tool | Description |
|------|-------------|
| `graph.add_edge` | Create a directed, labeled relationship between memories |
| `graph.get_neighbors` | Get directly connected memories |
| `graph.traverse` | Multi-hop BFS traversal |
| `graph.update_edge` | Update an edge's label or properties |
| `graph.delete_edge` | Delete a relationship |
| `graph.list_labels` | List distinct edge labels with counts |
| `graph.stats` | Edge count, label distribution, most-connected memories |

### Namespaces (planned)

| Tool | Description |
|------|-------------|
| `memory.create_namespace` | Create a hierarchical namespace |
| `memory.list_namespaces` | List namespaces with optional prefix filter |
| `memory.delete_namespace` | Delete a namespace and its memories |

### Session management (planned)

| Tool | Description |
|------|-------------|
| `memory.checkpoint` | Create a named checkpoint at a specific event |
| `memory.branch` | Fork conversation from any event |
| `memory.list_checkpoints` | List checkpoints for a session |
| `memory.list_branches` | List branches for a session |

### Store management (planned)

| Tool | Description |
|------|-------------|
| `memory.switch_store` | Close current store, open another |
| `memory.current_store` | Return the active store name |
| `memory.list_stores` | List all stores with file sizes |
| `memory.delete_store` | Delete a store (cannot delete active) |

### Utility (planned)

| Tool | Description |
|------|-------------|
| `memory.stats` | Event/memory/edge/session counts, DB size |
| `memory.export` | Export memories and edges as JSON |
| `memory.import` | Import memories and edges from JSON |

## Configuration

| Environment Variable | Default | Description |
|---------------------|---------|-------------|
| `LOCAL_MEMORY_HOME` | `~/.local-memory-mcp/` | Base directory for store files |
| `LOCAL_MEMORY_SYNC` | `FULL` | SQLite synchronous mode (`FULL` or `normal`) |
| `RUST_LOG` | — | Tracing filter (e.g., `local_memory_mcp=debug`) |

## Data Model

### Events (short-term)

Immutable conversation records scoped by actor + session. Support conversation and blob types, optional metadata (JSON), branch association, and TTL expiry.

### Memories (long-term)

Extracted insights with strategy labels, namespace organization, and optional embeddings. Consolidation creates an immutable audit trail — superseded memories are marked invalid with `superseded_by` pointers, never deleted.

### Knowledge edges (planned)

Directed, labeled relationships between memories with JSON properties. Enables graph traversal via recursive CTEs.

## Performance

- **Startup**: SQLite opens in <10ms
- **Store switch**: <20ms (close + open)
- **Event writes**: ~1μs per insert (in-process, no network)
- **FTS5 search**: Sub-millisecond at typical scale
- **Vector search**: Sub-millisecond for <100K vectors
- **Binary size**: ~5–10MB

## License

MIT
