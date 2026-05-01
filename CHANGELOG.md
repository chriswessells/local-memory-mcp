# Changelog

## v0.2.0.1

### Breaking changes

- **`metadata` and `properties` wire format** (`memory.create_event`,
  `memory.create_memory_record`, `memory.create_checkpoint`,
  `graph.create_edge`, `graph.update_edge`): these fields now accept a
  JSON **object** directly instead of a JSON-encoded string.
  - Before: `"metadata": "{\"source\":\"user\"}"`
  - After:  `"metadata": {"source": "user"}`
  - Callers that pass `null` or omit these fields are unaffected.
  - Existing stores are fully compatible — v0.2 data reads correctly in v0.3.

### New validation rules

- `metadata` and `properties` objects are now limited to 50 keys and
  nesting depth 5.

---

## v0.2.0 — Breaking changes

All 29 tool names have been realigned with AWS Bedrock AgentCore Memory naming
conventions. There are no backward-compatible aliases — update your harness config
before upgrading.

### Breaking changes

#### Tool renames

| v0.1 name | v0.2 name |
|---|---|
| `memory.add_event` | `memory.create_event` |
| `memory.get_events` | `memory.list_events` |
| `memory.delete_expired` | `memory.delete_expired_events` |
| `memory.store` | `memory.create_memory_record` |
| `memory.get` | `memory.get_memory_record` |
| `memory.list` | `memory.list_memory_records` |
| `memory.recall` | `memory.retrieve_memory_records` |
| `memory.consolidate` | `memory.update_memory_record` |
| `memory.delete` | `memory.delete_memory_record` |
| `memory.checkpoint` | `memory.create_checkpoint` |
| `memory.branch` | `memory.create_branch` |
| `graph.add_edge` | `graph.create_edge` |
| `graph.stats` | `graph.get_stats` |
| `memory.switch_store` | `store.switch` |
| `memory.current_store` | `store.current` |
| `memory.list_stores` | `store.list` |
| `memory.delete_store` | `store.delete` |

#### Field renames (on affected tools only)

| v0.1 field | v0.2 field | Affected tools |
|---|---|---|
| `memory_id` | `memory_record_id` | `memory.get_memory_record`, `memory.update_memory_record`, `memory.delete_memory_record`, `graph.get_neighbors` |
| `from_memory_id` / `to_memory_id` | `from_memory_record_id` / `to_memory_record_id` | `graph.create_edge` |
| `start_memory_id` | `start_memory_record_id` | `graph.traverse` |
| `query` | `search_query` | `memory.retrieve_memory_records` only — other tools are unaffected |
| `limit` | `top_k` | `memory.retrieve_memory_records` only (all other tools keep `limit`) |

### Improved

- All 29 tool descriptions rewritten with AgentCore-style "Use this when X; use Y for Z" discriminators
- `ToolAnnotations` added to all tools (`readOnlyHint`, `destructiveHint`, `idempotentHint`, `title`)
- `#[schemars(description)]` added to ~80 parameter fields for better LLM discoverability
- Server `instructions` block added via `get_info()` override (actor_id concept, namespace convention, embedding contract, strategy vocabulary, intent→tool decision list)
- Server identity corrected from `"rmcp"` to `"local-memory-mcp"`

### Migration

Use `grep` to find all calls to update:

```bash
grep -rn 'memory\.add_event\|memory\.store\b\|memory\.recall\b\|memory\.get\b\|memory\.list\b\|memory\.consolidate\|memory\.delete\b\|memory\.checkpoint\b\|memory\.branch\b\|memory\.get_events\|memory\.delete_expired\b\|memory\.switch_store\|memory\.current_store\|memory\.list_stores\|memory\.delete_store\|graph\.add_edge\|graph\.stats\b\|"memory_id"\|"from_memory_id"\|"to_memory_id"\|"start_memory_id"' .
```

For `memory.retrieve_memory_records`, also check for old field names (these won't cause errors — the server silently ignores unknown fields — but results will be wrong):

```bash
grep -rn '"query"\|"limit"' . | grep -i retrieve
```

---

## v0.1.0 — Initial release

29 MCP tools over stdio, backed by a single SQLite binary with no cloud dependencies.

### Features

- **Short-term memory** — Immutable conversation events scoped by actor and session, with optional TTL expiry
- **Long-term memory** — Extracted insights stored with strategies and namespace organization
- **Full-text search** — FTS5 BM25-ranked keyword search over memory content
- **Vector similarity search** — sqlite-vec KNN search over caller-provided 384-dim embeddings
- **Hybrid search** — Reciprocal Rank Fusion (RRF) combining FTS5 and vector results
- **Knowledge graph** — Typed, directed edges between memories with multi-hop BFS traversal (max depth 5, max 1000 nodes)
- **Memory consolidation** — Update or invalidate memories with an immutable audit trail
- **Session checkpoints & branches** — Named snapshots and conversation forks for workflow resumption and what-if scenarios
- **Namespace registry** — Register and manage namespace paths with per-actor scoped bulk-delete
- **Multi-store isolation** — Each memory store is a separate SQLite file, independently switchable
- **Actor isolation** — All data is scoped by actor ID; actors cannot see each other's data
- **Graceful shutdown** — SIGTERM/SIGINT signal handling with WAL checkpoint and PRAGMA optimize before exit

### Infrastructure

- Cross-platform CI/CD (GitHub Actions): fmt, clippy, test on Ubuntu + macOS; cross-compiled releases for Linux x86_64, Linux aarch64, macOS arm64
- One-command installer (`install.sh`) with platform detection, SHA256 checksum verification, TLS 1.2 enforcement, and atomic install
- 153 tests (unit + integration + E2E)
- GitHub Actions upgraded to Node.js 24 runtime
