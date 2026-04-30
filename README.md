```
    ╭──────────────────────────────────────────────────────────╮
    │                                                          │
    │    l o c a l  ·  m e m o r y  ·  m c p                 │
    │                                                          │
    │    Persistent memory for AI agents.                      │
    │    SQLite · FTS5 · Vectors · Knowledge Graph             │
    │    One binary. No cloud. No Docker.                      │
    │                                                          │
    ╰──────────────────────────────────────────────────────────╯
```

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

The server is functional with 29 MCP tools, 153 tests, CI/CD, and a one-command installer.

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
| Integration & E2E tests (153 tests) | ✅ Done |

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

## How This Was Built

This project was developed entirely using a structured, agent-driven process over approximately **20 hours** of wall-clock time, spread across four days. The result is a production-ready Rust MCP server with 29 tools, 153 tests, a cross-platform CI/CD pipeline, and a one-command installer — work that would typically represent several weeks of traditional development effort.

### The development process

Every component followed the same four-phase workflow, with no exceptions:

**1. Design first.** Before writing a single line of code, the agent produced a detailed design document covering data flow, error handling strategy, schema changes, the full API surface, an implementation plan, and a dependency graph of parallel vs. sequential tasks. All design artifacts live in `design/` as version-controlled, reviewable documents.

**2. Multi-perspective design review.** Five specialized review personas — security, architecture, maintainability, reliability, and interoperability — each read the design and produced findings rated Critical, High, Medium, or Low. Every Critical and High finding was resolved before any code was written. Medium and Low items were logged to `agents/TODO.md` as backlog. If the fixes were substantial, the full review panel re-ran against the revised design. The gate was simple: reviewers had to approve the design that would actually be built, not a prior version of it.

**3. Code against the approved design.** The agent implemented the approved design following its own dependency graph, writing tests alongside the code. The build had to stay clean — `cargo check`, `cargo test`, and `cargo clippy -- -D warnings` — throughout.

**4. Code review.** The same five personas reviewed the implementation. All Critical and High findings were fixed before merging to `main`. Every component's architectural decisions were recorded in `agents/ADR.md`, and the retrospective notes in `agents/LESSONS_LEARNED.md` informed the design of the next component.

The tracking files — `agents/TODO.md`, `agents/TIME_LOG.md`, `agents/ADR.md` — meant that any session, with any agent or human, could pick up exactly where the previous one left off without re-deriving context.

### Why this works better than other approaches

**Compared to vibe coding** — Vibe coding is fast to start: describe what you want, accept what the agent produces, iterate until it roughly works. For small scripts and throwaway tools, that's fine. For a project with 12 interdependent components, concurrent read/write paths, multiple search modes, and a security boundary between actors, architecture has to be intentional. Vibe coding produces architecture by accident. Critical issues — actor isolation gaps, SQL injection vectors, data loss on concurrent writes — surface after they're baked in, when they're expensive to fix. This process catches them in the design phase, before they exist in code.

**Compared to developer-directed coding** — In developer-directed coding, the human writes the spec: detailed requirements, interface definitions, maybe pseudocode. The agent fills in the implementation. This is more disciplined than vibe coding, but the human is still the primary reasoning engine — responsible for identifying every failure mode, every edge case, every security implication. The agent is a fast typist. In this process, the agent does the design reasoning too. The developer steers direction, validates quality gates, and makes judgment calls — but the five review personas do the heavy lifting of finding what was missed. The human doesn't need to be an expert in every dimension simultaneously; the personas provide that specialization.

**Compared to tab-complete coding** — Inline completions (Copilot, Cursor) work at the line or function level. The model sees local context and predicts what comes next — excellent for boilerplate, common patterns, and filling in known shapes. But a completion model doesn't know whether the function it's suggesting fits the broader architecture, whether it introduces a new SQL injection surface, or whether it's consistent with the actor-isolation invariant established three files away. It has no design document to check against and no cross-component view. This process works at the component level, with explicit design artifacts that make cross-cutting concerns visible before any implementation begins.

The common thread is that structure creates leverage. A five-minute design review that catches a Critical security finding prevents hours of remediation later. A tracked architectural decision prevents the next session from re-litigating a settled question. Tests written against a reviewed design have a clear target to hit. None of this requires a human to do the review or write the design — but it does require that someone asks for it.

## Configuration

| Environment Variable | Default | Description |
|---------------------|---------|-------------|
| `LOCAL_MEMORY_HOME` | `~/.local-memory-mcp/` | Base directory for store files |
| `LOCAL_MEMORY_SYNC` | `FULL` | SQLite synchronous mode (`FULL` or `normal`) |
| `RUST_LOG` | `info` | Tracing filter (e.g., `local_memory_mcp=debug`) |

## License

[MIT](LICENSE)
