# LLM Harness Discoverability — Research & Recommendations

## Context

Users of `local-memory-mcp` reported that LLM harnesses (Claude Code,
Cursor, etc.) have a difficult time finding the server's tools and
knowing how to use them. This document captures the findings of an
audit of `src/tools.rs` + `src/main.rs` + the `rmcp` 1.5.0 dependency,
and a prioritized list of recommended changes. The recommendations are
also tracked in `agents/TODO.md` under "LLM Harness Discoverability".

The audit was conducted on 2026-04-28 against commit `1a1b349`.

---

## What an LLM actually sees

When a harness connects to the server it receives:

1. The MCP `initialize` response — `server_info`, `capabilities`, and
   `instructions`. Most harnesses inject `instructions` directly into
   the model's prompt context. This is the primary "how to use this
   server" channel.
2. The `tools/list` response — for each tool: `name`, `description`,
   `inputSchema` (JSON Schema), and optional `annotations`.

Anything not surfaced through one of those two channels is invisible
to the model. The README, the design docs, and field doc-comments on
internal Rust types do **not** reach the LLM.

---

## Findings

### F1 — No top-level `instructions` set on the server

`main.rs:30` calls `rmcp::serve_server(server, transport)` and
`MemoryServer` never overrides `get_info()`. rmcp's default
`ServerHandler::get_info()` returns `ServerInfo::default()`, which
sets `instructions: None` (`rmcp/src/model.rs:855-856,898-899`).

Without an `instructions` block, the LLM has no place that explains:

- what an `actor_id` is or how to choose one (every tool requires it)
- the namespace path convention (`/user/alice/preferences`)
- that the server does **not** compute embeddings — the caller must
- when to use events vs long-term memories vs the graph
- which tool to reach for given a common intent (search, save, etc.)

It has to derive all of this from 29 tool descriptions read cold.

### F2 — The server identifies itself as `"rmcp"`, not `"local-memory-mcp"`

`Implementation::from_build_env()` (`rmcp/src/model.rs:1022-1031`) reads
`env!("CARGO_CRATE_NAME")`. `env!` is expanded at the call-site's
compile time, which is rmcp's own crate. So `server_info.name` ends up
as `"rmcp"` and `server_info.version` is rmcp's version. Hosts that key
off the server name in their UI or logs will mis-label the server.

### F3 — Param fields lack descriptions

A `grep` for `schemars(description` and `///` in `src/tools.rs` shows
~13 documented fields out of ~80 across the param structs. Fields with
non-obvious semantics that have **no** description today:

| Field | Where | Issue |
|---|---|---|
| `actor_id` | nearly every tool | required everywhere, never explained |
| `session_id` | event/session tools | required, never explained |
| `strategy` | `StoreMemoryParams`, `RecallToolParams`, `ListMemoriesToolParams` | free-form string, no hint at conventional values |
| `metadata` | many | schema says `string`, code expects a JSON object string — schema misleads the LLM |
| `namespace` / `namespace_prefix` | many | convention only documented on the *create* tool field |
| `properties` (edges) | `AddEdgeParams`, `UpdateEdgeToolParams` | JSON object string, only barely flagged |
| `memory_id` / `event_id` / `edge_id` | many | no note that these are UUIDs returned by other tools |

The embedding fields have the dim constraint and a short description,
but do **not** say the server doesn't compute embeddings. As a result
LLMs almost never provide one and fall back to FTS-only search.

### F4 — Naming collisions hurt tool selection

- **`memory.create_memory_record` (verb) vs `store.list` / `store.switch`
  / `store.delete` (noun).** "Store" means two different things
  in this API: save a memory, and a SQLite database file. An LLM
  searching for "store" gets both meanings, and the descriptions don't
  strongly disambiguate. Single biggest naming problem.
- **`memory.retrieve_memory_records` is the search tool but "recall" is AgentCore
  parlance.** LLMs reaching for "search memory" don't always pick it.
  The description starts with "Search memories…" which helps, but the
  *name* is the strongest signal.
- **`memory.list_memory_records` vs `memory.retrieve_memory_records` vs `memory.get_memory_record`** — all feel like
  "find memories" without a "use this for X, not Y" discriminator.
- **Dotted names (`memory.create_event`)** are spec-legal, but some hosts
  mangle them when prefixing (e.g. `mcp__local-memory__memory.create_event`).
  Underscore-only names travel better across older or custom harnesses.

### F5 — No `ToolAnnotations` on any tool

rmcp supports `ToolAnnotations` (`rmcp/src/handler/server/router/tool.rs:296`,
`tool/tool_traits.rs:70`) — `readOnlyHint`, `destructiveHint`,
`idempotentHint`, `openWorldHint`, `title`. None are set today, so
hosts treat all 29 tools uniformly:

- read-only tools (`memory.list_memory_records`, `memory.retrieve_memory_records`, `memory.get_event`,
  `memory.list_sessions`, `graph.get_neighbors`, `graph.traverse`,
  `graph.list_labels`, `graph.get_stats`, `memory.list_checkpoints`,
  `memory.list_branches`, `memory.list_namespaces`, `store.list`,
  `store.current`) needlessly trigger permission prompts;
- destructive tools (`memory.delete_memory_record`, `store.delete`,
  `memory.delete_namespace`, `memory.delete_expired_events`,
  `graph.delete_edge`) are not flagged for the user;
- idempotent tools (`memory.create_namespace`, `store.switch`)
  are not flagged for retry-safe automation.

### F6 — Tool descriptions are accurate but lack selection guidance

Per-tool prose is generally fine, but with 29 tools the LLM's main
problem is *picking the right one*. None of the descriptions include:

- "use this when X, not Y" framing for siblings
- a short concrete example invocation for tools whose param shapes
  are non-obvious (`memory.retrieve_memory_records`, `memory.update_memory_record`, `graph.traverse`)
- any value vocabulary for free-form fields like `strategy`

---

## Recommendations (ranked by impact)

### R1 — Override `get_info()` on `MemoryServer`

Highest leverage, smallest diff. Override:

- `server_info: Implementation::new("local-memory-mcp",
   env!("CARGO_PKG_VERSION")).with_title("Local Memory")
   .with_description("…")` — fixes F2.
- `instructions: Some(<block>)` — fixes F1. The block should cover, in
  ~200–400 words:
  - the three concept layers: short-term events, long-term memories,
    knowledge graph;
  - what `actor_id` is and how to pick one (single tenant ⇒ a constant
    like `"default"`; multi-tenant ⇒ stable per-user identifier);
  - the namespace convention (`/user/{actor}/preferences`);
  - the embedding contract (caller-computed, 384-dim, omit to use FTS
    only — server does not generate embeddings);
  - the `strategy` vocabulary (free-form, but suggest AgentCore-style
    values: `summarization`, `user_preference`, `semantic`, …);
  - an intent → tool decision list (e.g. "search memories →
    `memory.retrieve_memory_records`; enumerate by namespace → `memory.list_memory_records`; record a
    new conversation turn → `memory.create_event`; save an extracted
    insight → `memory.create_memory_record`").

### R2 — Add `#[schemars(description = …)]` to every non-obvious field

Fixes F3. Priorities:

- `actor_id` and `session_id` — what they are, how to choose them.
- `strategy` — describe as a free-form label and list common values.
- `metadata` — clarify it's a JSON object string with an example like
  `{"source":"user"}`.
- `namespace` and `namespace_prefix` — repeat the path convention.
- `embedding` and `new_embedding` — append "the server does not
  compute embeddings; compute externally with a 384-dim model or omit
  to use FTS-only search".
- All `*_id` fields — note these are UUIDs returned by other tools.

### R3 — Disambiguate the `store` overload (F4)

**Resolved by `design/agentcore-parity.md`** (added 2026-04-28). The
parity doc commits to moving the four database-management tools into
a dedicated `store.*` namespace (`store.switch` / `store.current` /
`store.list` / `store.delete`) and renaming `memory.create_memory_record` →
`memory.create_memory_record`. The verb `store` no longer appears
anywhere on the surface, eliminating the noun/verb collision.

### R4 — Surface "search" / "retrieve" as a tool name (F4)

**Resolved by `design/agentcore-parity.md`** (added 2026-04-28). The
parity doc renames `memory.retrieve_memory_records` → `memory.retrieve_memory_records`,
matching AgentCore's exact verb (`RetrieveMemoryRecords`). The first
sentence of the description leads with "Search memory records…" so
both "retrieve" and "search" are visible to LLM tool-selection.

### R5 — Add `ToolAnnotations` everywhere (F5)

Tool names below use the **v0.2 names** from `agentcore-parity.md`.

| Annotation | Tools |
|---|---|
| `readOnlyHint=true` | all `*get_*`, all `*list_*`, `store.current`, `memory.retrieve_memory_records`, `graph.get_neighbors`, `graph.traverse`, `graph.list_labels`, `graph.get_stats` |
| `destructiveHint=true` | `memory.delete_memory_record`, `memory.delete_event` (when added), `memory.delete_expired_events`, `memory.delete_namespace`, `store.delete`, `graph.delete_edge` |
| `idempotentHint=true` | `memory.create_namespace`, `store.switch` |
| `title` | all tools — short human label for UI |

### R6 — Rewrite descriptions of similar tools with discriminators (F6)

Pairs that need explicit "use this for X, not Y" framing in the
description's first sentence (using v0.2 names):

- `memory.list_memory_records` ↔ `memory.retrieve_memory_records`
  ("filtered enumeration" vs "ranked retrieval")
- `memory.get_memory_record` ↔ `memory.list_memory_records` ↔
  `memory.retrieve_memory_records` (single by ID vs enumerate vs search)
- `memory.get_event` ↔ `memory.list_events` ↔ `memory.list_sessions`
  (one event, many events in one session, sessions for an actor)
- `graph.get_neighbors` ↔ `graph.traverse` (one-hop vs multi-hop)
- `memory.update_memory_record` ↔ `memory.delete_memory_record`
  (supersede with audit trail vs hard delete)

See `design/agentcore-parity.md` §"Description style guide" for the
canonical template and worked examples.

### R7 — Switch `metadata` and graph `properties` to actual JSON

Change `Option<String>` → `Option<serde_json::Value>` for `metadata`
and `properties` fields. The current shape forces the LLM to mentally
JSON-stringify, and the JSON Schema declares the field as plain
`string` — a contradictory signal. The wire schema becomes a real
object and matches what the LLM naturally produces. Validation on the
way in is unchanged.

### R8 — Replace dots with underscores in tool names

Optional but improves cross-host portability (F4). `memory.create_event`
→ `memory_add_event`, `graph.traverse` → `graph_traverse`. If we keep
dots for AgentCore parity, document the requirement explicitly in the
README and confirm the chosen target hosts handle them.

---

## Sequencing

R1 + R2 + R5 are localized to `tools.rs`/`main.rs`, do not touch
business logic, and together cover the bulk of the discoverability
gap. They should land first.

R3, R4, R7, R8 are name/shape changes — they affect the public MCP
surface and warrant a minor-version bump. Bundle them into a single
"v0.2 surface cleanup" release rather than landing piecemeal.

R6 is description-only and can land alongside R1/R2 with no surface
break.

---

## Implementation Notes (post-design-review, 2026-04-29)

These notes resolve all High findings from the design review before coding begins.

### Confirmed: `get_info()` override pattern (resolves A1, INTEROP-3)

`#[tool_router(server_handler)]` generates `#[tool_handler] impl ServerHandler for MemoryServer
{}` automatically. Adding a second `impl ServerHandler for MemoryServer` block would be a
compile error. The correct pattern, documented in `rmcp-macros-1.5.0/src/lib.rs:182–193`:

1. Change `#[tool_router(server_handler)]` → `#[tool_router]` on the inherent `impl MemoryServer` block.
2. Add a separate `#[tool_handler] impl ServerHandler for MemoryServer` block that includes `get_info()`:

```rust
#[tool_router]
impl MemoryServer {
    // tool methods unchanged
}

#[tool_handler]
impl ServerHandler for MemoryServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            server_info: Implementation::new("local-memory-mcp", env!("CARGO_PKG_VERSION"))
                .with_title("Local Memory")
                .with_description("Local agent memory server: events, long-term memories, and knowledge graph over SQLite."),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            instructions: Some(SERVER_INSTRUCTIONS.to_string()),
            ..Default::default()
        }
    }
}
```

The `#[tool_handler]` macro generates `call_tool` and `list_tools`; it does NOT generate
`get_info()` when you provide one yourself.

### Confirmed: `ToolAnnotations` attachment (resolves M3, INTEROP-4)

The `#[tool]` macro supports `annotations` directly
(`rmcp-macros-1.5.0/src/lib.rs:27,32,391`). No `list_tools()` override needed:

```rust
#[tool(
    name = "memory.retrieve_memory_records",
    description = "...",
    annotations(title = "Search memories", read_only_hint = true)
)]
async fn recall(&self, ...) { ... }

#[tool(
    name = "memory.delete_memory_record",
    description = "...",
    annotations(title = "Delete memory record", destructive_hint = true)
)]
async fn delete_memory(&self, ...) { ... }

#[tool(
    name = "store.switch",
    description = "...",
    annotations(title = "Switch store", idempotent_hint = true)
)]
async fn switch_store(&self, ...) { ... }
```

### Instructions string as named constant (resolves M1)

The instructions string must be defined as a named constant, not inline prose:

```rust
// src/tools.rs (or a dedicated src/server_info.rs)
// WARNING: instructions are sent verbatim to the LLM on every connection.
// Do NOT interpolate runtime values, user data, or filesystem paths here.
const SERVER_INSTRUCTIONS: &str = "...";
```

Add a compile-time test that asserts the constant is non-empty and contains the required
vocabulary (`actor_id`, `namespace`, `strategy`, `embedding`):

```rust
#[test]
fn server_instructions_contains_required_vocabulary() {
    for keyword in &["actor_id", "namespace", "strategy", "embedding"] {
        assert!(
            SERVER_INSTRUCTIONS.contains(keyword),
            "SERVER_INSTRUCTIONS missing keyword: {keyword}"
        );
    }
}
```

Draft instructions text (200–300 words):

```
local-memory-mcp gives you three layers of persistent, queryable memory:

EVENTS — immutable conversation turns. Use memory.create_event to record each message.
  Use memory.list_events to retrieve a session's history.

MEMORIES — long-term records extracted from events. Use memory.create_memory_record to save an insight
  or preference. Use memory.retrieve_memory_records to search by keyword or semantic similarity.
  Use memory.list_memory_records to enumerate memories by namespace. Use memory.get_memory_record to fetch one by ID.

KNOWLEDGE GRAPH — typed edges between memories. Use graph.create_edge to link two memories.
  Use graph.traverse to walk the graph from a starting memory.

actor_id: Every tool requires actor_id. In single-user deployments, pass a constant like
  "default". In multi-user deployments, use a stable per-user identifier (e.g., email hash
  or UUID). NEVER share actor_id across users — it scopes all data access.

namespace: A slash-separated path grouping related memories, e.g. "/user/alice/preferences"
  or "/project/myapp/decisions". All memories in a namespace are deleted together with
  memory.delete_namespace.

strategy: A free-form label describing how a memory was produced. Suggested values:
  "summarization", "user_preference", "semantic", "verbatim", "extraction".

embedding: The server does NOT compute embeddings. Pass a caller-computed EMBEDDING_DIM-dim
  float array to enable vector search; omit it to use FTS-only keyword search.

metadata: A JSON object string, e.g. '{"source":"user","confidence":0.9}'. The server
  stores it as-is; filter on it via memory.list_memory_records.
```

### Field description policy — single source of truth (resolves M2)

Concepts explained in the instructions block (actor_id, namespace, strategy, embedding,
metadata) must NOT be re-explained in full in `#[schemars(description)]` attributes. Field
descriptions must be short (1–2 sentences) and may cross-reference the instructions:

- `actor_id`: "Stable identifier scoping all data access for one user or agent. See server instructions."
- `namespace`: "Slash-separated path grouping related memories, e.g. '/user/alice/preferences'."
- `strategy`: "Free-form label for how this memory was produced. Suggested: 'summarization', 'user_preference', 'semantic'."
- `metadata`: "JSON object string, e.g. '{\"source\":\"user\"}'. Stored as-is."
- `embedding`: "Caller-computed float array (EMBEDDING_DIM dims). Omit for FTS-only search. Server does not generate embeddings."

All `*_id` fields: "UUID returned by [tool name that creates this resource]."

### R5 — Tier 1 annotation table (v0.1 names) (resolves INTEROP-2)

Apply these annotations using the current v0.1 tool names. The v0.2 table in §R5 above is
the Tier 2 spec; the names will update when Tier 2 lands.

| Annotation | v0.1 Tool Names |
|---|---|
| `readOnlyHint=true` | `memory.get_event`, `memory.list_events`, `memory.list_sessions`, `memory.get_memory_record`, `memory.list_memory_records`, `memory.retrieve_memory_records`, `store.current`, `store.list`, `memory.list_namespaces`, `graph.get_neighbors`, `graph.traverse`, `graph.list_labels`, `graph.get_stats`, `memory.list_checkpoints`, `memory.list_branches` |
| `destructiveHint=true` | `memory.delete_memory_record`, `memory.delete_expired_events`, `store.delete`, `memory.delete_namespace`, `graph.delete_edge` |
| `idempotentHint=true` | `memory.create_namespace`, `store.switch` |

Tools with no behavioral annotation (write, non-idempotent, non-destructive):
`memory.create_event`, `memory.create_memory_record`, `memory.update_memory_record`, `graph.create_edge`, `graph.update_edge`,
`memory.create_checkpoint`, `memory.create_branch`. Add only `title` to these.

Every tool gets a `title` (short human label). Example titles:
`memory.create_event` → "Add event", `memory.create_memory_record` → "Store memory",
`memory.retrieve_memory_records` → "Search memories", `memory.delete_memory_record` → "Delete memory record",
`store.switch` → "Switch store", `graph.traverse` → "Traverse graph".

### R6 — Tier 1 discriminator pairs (v0.1 names) (resolves INTEROP-1)

All sibling-tool references in Tier 1 R6 descriptions must use v0.1 names.
The design's worked examples (in `agentcore-parity.md`) use v0.2 names — translate back:

| Sibling pair | Discriminator guidance |
|---|---|
| `memory.list_memory_records` ↔ `memory.retrieve_memory_records` | list = filtered enumeration by namespace/strategy (no ranking); recall = ranked search by keyword or vector similarity. Use `memory.list_memory_records` to enumerate all; use `memory.retrieve_memory_records` to search. |
| `memory.get_memory_record` ↔ `memory.list_memory_records` ↔ `memory.retrieve_memory_records` | get = fetch one record by ID; list = enumerate many; recall = search/rank. |
| `memory.get_event` ↔ `memory.list_events` ↔ `memory.list_sessions` | get_event = one event by ID; get_events = all events in one session; list_sessions = enumerate sessions for an actor. |
| `graph.get_neighbors` ↔ `graph.traverse` | get_neighbors = one hop (direct edges); traverse = multi-hop depth-first with depth limit. |
| `memory.update_memory_record` ↔ `memory.delete_memory_record` | consolidate = supersede with audit trail (old memory marked invalid, new one created); delete = hard delete with no recovery. |

---

## References

- `design/agentcore-parity.md` — canonical mapping table (resolves
  R3 + R4) and description style guide.
- rmcp 1.5.0 source — `~/.cargo/registry/src/index.crates.io-…/rmcp-1.5.0/`
  - `src/model.rs:843-901` — `InitializeResult` / `ServerInfo`
  - `src/model.rs:986-1031` — `Implementation::from_build_env`
  - `src/handler/server.rs:158-334` — default `get_info`
  - `src/handler/server/router/tool.rs:296` — `ToolAnnotations`
- MCP spec — server `instructions` and tool annotations:
  https://modelcontextprotocol.io/specification
- AgentCore Memory API reference (canonical AWS docs — supersedes
  the older Bedrock Agents memory docs the README links to):
  https://docs.aws.amazon.com/bedrock-agentcore/latest/APIReference/API_Operations.html
