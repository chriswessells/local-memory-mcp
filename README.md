```
    ╭──────────────────────────────────────────────────────────╮
    │                                                          │
    │    l o c a l  ·  m e m o r y  ·  m c p                   │
    │                                                          │
    │    Persistent memory for AI agents.                      │
    │    SQLite · FTS5 · Vectors · Knowledge Graph             │
    │    One binary. No cloud. No Docker.                      │
    │                                                          │
    ╰──────────────────────────────────────────────────────────╯
```


Local agent memory MCP server — SQLite-backed short-term and long-term memory for AI agents, inspired by [Amazon Bedrock AgentCore Memory](https://docs.aws.amazon.com/bedrock/latest/userguide/agents-memory.html).

A single compiled Rust binary that runs as an MCP server over stdio. Embeds SQLite with FTS5 for full-text search and [sqlite-vec](https://github.com/asg017/sqlite-vec) for vector similarity search. No cloud dependencies, no Docker, no runtime dependencies.
## Why I built local memory mcp

I built local memory mcp server because I use Kiro CLI when I build applications. My vision doesn't include me reading or writing code it is a black box handled by LLMs. I don't even write the design documents. My role is to to define what I want, what good looks like, and the evidence I need to prove the code is secure, performant, reliable, resilient, maintainable, and profitable. Yes, I want the unit economics of software to make sense.

As I build software, I need my agents to have durable memory that can be access fast and efficiently. I need the durable memory to have the features and functionality that match the type of data being stored and how it should be accessed. 
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

## Have Kiro-CLI configure Kiro-CLI to use the mcp server

What I did, I asked kiro what types of memory it had available, it searched the mcp servers and kiro said it had knowledge base for working memory and local memory mcp for long term learnings.

I continued the conversation asking kiro how it could best use the memory resources available to it. After it answered, I asked how could it remember what it just shared so that every time I use kiro it would use the knowledge base for working memory containing raw data, detailed data in md files. It created a SOP. 

The third phase of the conversation I asked kiro how would it move the information from knowledge base into the memory (long term learning). it described how it would move the information. I then described how I wanted it to work, "I don't want the responsibility of having to tell you to move information to the memory, I want you to do that for me." Kiro thought, then it offered options, then it recommended a hybrid approach from the two it offered to speed up the memory consolidation into learning. Kiro created sop and configured hooks. In the end Kiro configured itself to match how I wanted it to work.

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

Supported platforms: Linux x86_64, Linux aarch64, macOS arm64 (Apple Silicon), Windows x86_64.

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

## Configuration

| Environment Variable | Default                | Description                                     |
| -------------------- | ---------------------- | ----------------------------------------------- |
| `LOCAL_MEMORY_HOME`  | `~/.local-memory-mcp/` | Base directory for store files                  |
| `LOCAL_MEMORY_SYNC`  | `FULL`                 | SQLite synchronous mode (`FULL` or `normal`)    |
| `RUST_LOG`           | `info`                 | Tracing filter (e.g., `local_memory_mcp=debug`) |

## Status

The server is functional with 29 MCP tools, 153 tests, CI/CD, and a one-command installer.


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

## License

[MIT](LICENSE)
