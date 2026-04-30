# local-memory-mcp

Local agent memory MCP server — SQLite-backed short-term and long-term memory for AI agents, inspired by [Amazon Bedrock AgentCore Memory](https://docs.aws.amazon.com/bedrock/latest/userguide/agents-memory.html).

A single compiled Rust binary that runs as an MCP server over stdio. Embeds SQLite with FTS5 for full-text search and [sqlite-vec](https://github.com/asg017/sqlite-vec) for vector similarity search. No cloud dependencies, no Docker, no runtime dependencies.

## Install

```bash
curl --proto '=https' --tlsv1.2 -fsSL https://raw.githubusercontent.com/chriswessells/local-memory-mcp/main/install.sh | bash
```

Or download and inspect first:

```bash
curl --proto '=https' --tlsv1.2 -fsSL https://raw.githubusercontent.com/chriswessells/local-memory-mcp/main/install.sh -o install.sh
less install.sh
bash install.sh
```

Custom install directory:

```bash
INSTALL_DIR=/usr/local/bin bash install.sh
```

Supported platforms: Linux x86_64, Linux aarch64, macOS arm64 (Apple Silicon).

### Build from source

```bash
git clone https://github.com/chriswessells/local-memory-mcp.git
cd local-memory-mcp
cargo build --release
# Binary at target/release/local-memory-mcp
```

Requires Rust toolchain and a C compiler (for bundled SQLite).

## MCP Server Configuration

Add to your MCP client config (Kiro, Claude Desktop, etc.):

```json
{
  "mcpServers": {
    "local-memory": {
      "command": "/path/to/local-memory-mcp",
      "args": []
    }
  }
}
```

The installer prints the exact config with the correct absolute path after installation.

## Status

The server is functional with 29 MCP tools, 149 tests, CI/CD, and a one-command installer.

| Component | Status |
|-----------|--------|
| Core DB layer (schema, migrations, store management) | ✅ Done |
| Event tools (short-term memory CRUD) | ✅ Done |
| Memory tools (long-term memory CRUD) | ✅ Done |
| Search (FTS5 + vector + hybrid RRF) | ✅ Done |
| Knowledge graph (edges, traversal, stats) | ✅ Done |
| Session tools (checkpoints, branches) | ✅ Done |
| Namespace tools | ✅ Done |
| Store management tools | ✅ Done |
| MCP server (stdio transport, 29 tools) | ✅ Done |
| CI/CD (GitHub Actions, release workflow) | ✅ Done |
| Installers (install.sh) | ✅ Done |
| Integration & E2E tests (149 tests) | ✅ Done |

## Features

- **Short-term memory** — Immutable conversation events scoped by actor and session, with optional TTL expiry
- **Long-term memory** — Extracted insights stored with strategies and namespace organization
- **Full-text search** — FTS5 BM25-ranked keyword search over memory content
- **Vector similarity search** — sqlite-vec KNN search over caller-provided 384-dim embeddings
- **Hybrid search** — Reciprocal Rank Fusion (RRF) combining FTS5 and vector results
- **Knowledge graph** — Typed, directed edges between memories with multi-hop BFS traversal
- **Memory consolidation** — Update or invalidate memories with an immutable audit trail
- **Session checkpoints & branches** — Named snapshots and conversation forks for workflow resumption and what-if scenarios
- **Namespace registry** — Register and manage namespace paths with per-actor scoped bulk-delete
- **Multi-store isolation** — Each memory store is a separate SQLite file, independently switchable
- **Namespace hierarchy** — Organize memories in paths like `/user/{actorId}/preferences`
- **Actor isolation** — All data is scoped by actor ID; actors cannot see each other's data

## Upgrading from v0.1

v0.2 renames 17 tools and 5 fields with no backward-compatible aliases. Use `grep` to find calls to update:

```bash
grep -rn 'memory\.add_event\|memory\.store\b\|memory\.recall\b\|memory\.get\b\|memory\.list\b\|memory\.consolidate\|memory\.delete\b\|memory\.checkpoint\b\|memory\.branch\b\|memory\.get_events\|memory\.delete_expired\b\|memory\.switch_store\|memory\.current_store\|memory\.list_stores\|memory\.delete_store\|graph\.add_edge\|graph\.stats\b\|"memory_id"\|"from_memory_id"\|"to_memory_id"\|"start_memory_id"' .
```

See [CHANGELOG.md](CHANGELOG.md) for the full rename table.

## MCP Tools (29)

### Short-term memory (events)

| Tool | Description |
|------|-------------|
| `memory.create_event` | Append an immutable conversation or blob event to a session timeline |
| `memory.get_event` | Retrieve a single event by ID |
| `memory.list_events` | List events for an actor+session with branch filter, time range, and pagination |
| `memory.list_sessions` | List distinct sessions with event counts and date ranges |
| `memory.delete_expired_events` | Remove events past their TTL |

### Long-term memory

| Tool | Description |
|------|-------------|
| `memory.create_memory_record` | Create a long-term memory record with optional embedding |
| `memory.get_memory_record` | Retrieve a single memory record by ID |
| `memory.retrieve_memory_records` | Search by text (FTS5), vector similarity, or hybrid RRF |
| `memory.update_memory_record` | Update or invalidate a memory record (immutable audit trail) |
| `memory.list_memory_records` | List records with namespace, strategy, and validity filters |
| `memory.delete_memory_record` | Hard-delete a memory record and its embedding |

### Knowledge graph

| Tool | Description |
|------|-------------|
| `graph.create_edge` | Create a directed, labeled relationship between memory records |
| `graph.get_neighbors` | Get directly connected memory records (one hop) |
| `graph.traverse` | Multi-hop BFS traversal with depth and direction control |
| `graph.update_edge` | Update an edge's label or properties |
| `graph.delete_edge` | Delete a relationship |
| `graph.list_labels` | List distinct edge labels with counts |
| `graph.get_stats` | Edge count, label distribution, most-connected memory records |

### Namespaces

| Tool | Description |
|------|-------------|
| `memory.create_namespace` | Register a namespace with optional description (idempotent) |
| `memory.list_namespaces` | List registered namespaces with optional prefix filter and pagination |
| `memory.delete_namespace` | Delete all actor-scoped memories in a namespace and remove the registry entry |

### Session tools (checkpoints & branches)

| Tool | Description |
|------|-------------|
| `memory.create_checkpoint` | Create a named snapshot at a specific event for workflow resumption |
| `memory.create_branch` | Fork a conversation from a specific event for alternative paths |
| `memory.list_checkpoints` | List all checkpoints for a session, ordered by creation time |
| `memory.list_branches` | List all branches for a session, ordered by creation time |

### Store management

| Tool | Description |
|------|-------------|
| `store.switch` | Close current store, open another (creates if new) |
| `store.current` | Return the active store name |
| `store.list` | List all stores with file sizes |
| `store.delete` | Delete a store (cannot delete active) |

## Design Principle: AgentCore Memory Compatibility

An agent with a system prompt should be able to use either AgentCore Memory or local-memory-mcp and not know the difference.

Same conceptual model, same tool semantics, same data lifecycle. The only transparent differences:

- **Extraction is explicit** — The agent calls `memory.create_memory_record` instead of extraction happening automatically
- **Embeddings are caller-provided** — The agent provides 384-dim vectors; the server stores and indexes them
- **Store management is additive** — `store.*` tools are a local-only extension
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

## Configuration

| Environment Variable | Default | Description |
|---------------------|---------|-------------|
| `LOCAL_MEMORY_HOME` | `~/.local-memory-mcp/` | Base directory for store files |
| `LOCAL_MEMORY_SYNC` | `FULL` | SQLite synchronous mode (`FULL` or `normal`) |
| `RUST_LOG` | `info` | Tracing filter (e.g., `local_memory_mcp=debug`) |

## License

[MIT](LICENSE)
