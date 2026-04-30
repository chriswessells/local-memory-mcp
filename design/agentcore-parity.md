# AgentCore Memory Parity — Tool Naming & Description Reference

## Purpose

`local-memory-mcp` is positioned as an AgentCore Memory-compatible
backend (DESIGN.md §"Design Principle: API Compatibility with
AgentCore Memory"). This document is the canonical mapping between
the MCP tools this server exposes and the operations published in
AWS's Bedrock AgentCore Memory API. It exists for three reasons:

1. **LLM discoverability.** Naming the tools after AgentCore's verbs
   makes the server immediately legible to any agent prompt written
   for AgentCore (and to any LLM that has been pre-trained on AWS
   docs). See `design/llm-discoverability.md` for the broader audit.
2. **A single source of truth.** When the description of a tool says
   "(AgentCore equivalent: `RetrieveMemoryRecords`)", that claim
   should be checkable against this doc, not against AWS's docs page
   directly.
3. **Divergence rationale.** Where we deliberately differ from
   AgentCore (graph extension, multi-store, manual extraction), the
   reasons are recorded once here instead of being re-derived in
   each tool's description.

The renames in this document are **scheduled for v0.2** (see
`agents/TODO.md` "LLM Harness Discoverability" section). Until v0.2
ships, the *current* tool names remain authoritative — see
`src/tools.rs` for the live names.

---

## Tool name mapping

Format: current name → recommended v0.2 name → AgentCore op.

### Short-term memory (events)

| Current | v0.2 name | AgentCore op | Rationale |
|---|---|---|---|
| `memory.add_event` | `memory.create_event` | `CreateEvent` | AgentCore CRUD verb. "add" is non-standard in the AgentCore API. |
| `memory.get_event` | `memory.get_event` | `GetEvent` | Already aligned. |
| `memory.get_events` | `memory.list_events` | `ListEvents` | AgentCore uses `List` for plural reads. Matches our existing `memory.list_sessions`. |
| `memory.list_sessions` | `memory.list_sessions` | `ListSessions` | Already aligned. |
| `memory.delete_expired` | `memory.delete_expired_events` | (none — local) | Disambiguate: this deletes events, not memory records or stores. AgentCore has no automatic-TTL-deletion op. |
| (none) | (consider) `memory.delete_event` | `DeleteEvent` | AgentCore exposes `DeleteEvent`; we currently only support TTL-based deletion. Out of scope for v0.2 rename, tracked separately. |

### Long-term memory (memory records)

| Current | v0.2 name | AgentCore op | Rationale |
|---|---|---|---|
| `memory.store` | `memory.create_memory_record` | `CreateMemoryRecord` (closest: `BatchCreateMemoryRecords`) | Resolves the `memory.store` (verb) ↔ `memory.list_stores` (noun) collision. AgentCore-grounded. |
| `memory.get` | `memory.get_memory_record` | `GetMemoryRecord` | Bare `get` is uninformative. |
| `memory.list` | `memory.list_memory_records` | `ListMemoryRecords` | Bare `list` is uninformative. |
| `memory.recall` | `memory.retrieve_memory_records` | `RetrieveMemoryRecords` | Highest-leverage rename. "Retrieve" is AgentCore's verb for semantic search and surfaces the search intent in the name. |
| `memory.consolidate` | `memory.update_memory_record` | (closest: `BatchUpdateMemoryRecords`) | "Consolidate" is internal jargon. The operation is an update with audit trail. |
| `memory.delete` | `memory.delete_memory_record` | `DeleteMemoryRecord` | Direct AgentCore op name. |

### Sessions, branches, checkpoints (AgentCore-mixed)

`ListSessions` exists in AgentCore. Branches appear in AgentCore's
event payloads but there is no public `CreateBranch` operation.
Checkpoints have no AgentCore counterpart.

| Current | v0.2 name | AgentCore op | Rationale |
|---|---|---|---|
| `memory.list_sessions` | `memory.list_sessions` | `ListSessions` | Already aligned. |
| `memory.checkpoint` | `memory.create_checkpoint` | (none — local) | Verb consistency with the rest of the surface. |
| `memory.branch` | `memory.create_branch` | (none — local) | Verb consistency; clears the noun/verb ambiguity of bare "branch". |
| `memory.list_checkpoints` | `memory.list_checkpoints` | (none — local) | Already aligned. |
| `memory.list_branches` | `memory.list_branches` | (none — local) | Already aligned. |

### Namespaces (local extension on AgentCore-style paths)

AgentCore uses hierarchical namespace templates with trailing slash
(e.g., `/users/{actorId}/preferences/`). We expose explicit
register/list/delete operations because we don't auto-create
namespaces from extraction strategies.

| Current | v0.2 name | AgentCore op | Rationale |
|---|---|---|---|
| `memory.create_namespace` | `memory.create_namespace` | (none — local) | Already aligned. |
| `memory.list_namespaces` | `memory.list_namespaces` | (none — local) | Already aligned. |
| `memory.delete_namespace` | `memory.delete_namespace` | (none — local) | Already aligned. |

### Store management (local-only — moved to a dedicated namespace)

Multi-store does not exist in AgentCore (which manages a single
memory resource per agent). We move these tools out of `memory.*`
into `store.*` to eliminate the `memory.store` (verb) vs
`memory.list_stores` (noun) collision at the namespace level — the
verb `store` no longer appears anywhere on the surface.

| Current | v0.2 name | AgentCore op | Rationale |
|---|---|---|---|
| `memory.switch_store` | `store.switch` | (none — local) | Removes the `_store` suffix (now redundant under the `store.*` namespace). |
| `memory.current_store` | `store.current` | (none — local) | Same. |
| `memory.list_stores` | `store.list` | (none — local) | Same. |
| `memory.delete_store` | `store.delete` | (none — local) | Same. |

### Knowledge graph (local extension)

No AgentCore parent exists for any of these operations. Renames are
for surface consistency only.

| Current | v0.2 name | AgentCore op | Rationale |
|---|---|---|---|
| `graph.add_edge` | `graph.create_edge` | (none — local) | Verb consistency. |
| `graph.get_neighbors` | `graph.get_neighbors` | (none — local) | Already aligned. |
| `graph.traverse` | `graph.traverse` | (none — local) | Already aligned. |
| `graph.update_edge` | `graph.update_edge` | (none — local) | Already aligned. |
| `graph.delete_edge` | `graph.delete_edge` | (none — local) | Already aligned. |
| `graph.list_labels` | `graph.list_labels` | (none — local) | Already aligned. |
| `graph.stats` | `graph.get_stats` | (none — local) | Verb consistency. |

### Net change

- 12 tool renames within `memory.*` and `graph.*`.
- 4 tools moved from `memory.*` to a new `store.*` namespace.
- 13 tools unchanged.

---

## Field-name mapping

Wire format stays snake_case for MCP / JSON Schema convention.
AgentCore camelCase ↔ our snake_case mapping is mechanical for most
fields; the table calls out the few where the rename has semantic
weight.

| Our field | AgentCore field | Notes |
|---|---|---|
| `actor_id` | `actorId` | Same semantics. AgentCore validates `[a-zA-Z0-9][a-zA-Z0-9-_/]*…` (1–255 chars); we accept a broader set today. |
| `session_id` | `sessionId` | Same. AgentCore validates `[a-zA-Z0-9][a-zA-Z0-9-_]*` (1–100 chars). |
| `event_id` | `eventId` | AgentCore format is `[0-9]+#[a-fA-F0-9]+`. Ours is a UUID — documented divergence. |
| **`memory_id`** | `memoryRecordId` | **Rename to `memory_record_id` in v0.2.** AgentCore's `memoryId` refers to the memory *resource* (database-like), not an individual record — our `memory_id` corresponds to AgentCore's `memoryRecordId`. The rename eliminates a real semantic collision. |
| `from_memory_id`, `to_memory_id`, `start_memory_id` | `memoryRecordId` (in their respective contexts) | Rename to `from_memory_record_id`, `to_memory_record_id`, `start_memory_record_id`. |
| `edge_id` | (none — local) | No rename. |
| `query` (on recall) | `searchQuery` | **Rename to `search_query`** on `memory.retrieve_memory_records` to match AgentCore exactly. |
| `limit` (on recall) | `topK` | **Rename to `top_k`** on `memory.retrieve_memory_records` to match AgentCore exactly. Other tools keep `limit`. |
| `embedding` | (none — local extension) | AgentCore computes embeddings server-side; we accept caller-provided 384-dim vectors. |
| `metadata` | `metadata` | Same name. AgentCore allows 0–15 key-value pairs per event; we don't enforce a limit yet. |
| `strategy` | `memoryStrategyId` | Field name kept short (`strategy`) because AgentCore's strategies are opaque resource IDs while ours is a free-form label — different semantics, deliberately different name. |
| `namespace`, `namespace_prefix` | `namespace`, `namespacePath` | Same purpose. AgentCore convention requires trailing `/`; we accept both. |
| `properties` (on edges) | (none — local) | No rename. |
| `branch_id`, `parent_branch_id`, `root_event_id` | (none — local) | Branches are exposed as fields in AgentCore's event payloads, not as separate resources with IDs. Our explicit branch resource is a local extension. |

---

## Description style guide

Every tool description in v0.2 should follow this template:

```
<Verb in present tense> <object>. Use this when <intent>; use
<sibling tool> instead for <other intent>. <Key constraints —
required params, validation rules>. Returns <response shape>.
(AgentCore equivalent: <op>; or: local-only extension.)
```

The "Use this when… use … instead for…" sentence is the single
highest-leverage addition. It's what lets an LLM pick the right tool
on the first try when several siblings could plausibly match an
intent.

### Worked examples

**`memory.retrieve_memory_records`** (replacing `memory.recall`):

> Search memory records for an actor by text query, embedding
> vector, or hybrid (Reciprocal Rank Fusion). Use this when you have
> a query and want relevance-ranked results; use
> `memory.list_memory_records` instead for filtered enumeration. At
> least one of `search_query` or `embedding` must be provided.
> Embeddings are caller-computed 384-dim float32 vectors — the
> server does not generate them. Returns ranked memory records with
> scores; scores are not comparable across modes. (AgentCore
> equivalent: `RetrieveMemoryRecords`.)

**`memory.create_memory_record`** (replacing `memory.store`):

> Create a long-term memory record for an actor. Use this when the
> agent has extracted an insight worth retaining beyond the current
> session; use `memory.create_event` for raw conversation turns
> instead. The optional 384-dim embedding is caller-computed —
> the server does not generate embeddings. Returns the created
> record with its generated ID. (AgentCore equivalent:
> `CreateMemoryRecord` / `BatchCreateMemoryRecords`.)

**`memory.list_events`** (replacing `memory.get_events`):

> List events for an actor + session, ordered chronologically
> (oldest first). Use this for windowed reads of a conversation
> timeline; use `memory.get_event` for a single event by ID, and
> `memory.list_sessions` to enumerate the sessions belonging to an
> actor. Supports branch filter, time range, and limit/offset
> pagination. (AgentCore equivalent: `ListEvents`.)

**`memory.update_memory_record`** (replacing `memory.consolidate`):

> Update or invalidate a memory record. Use this to supersede an
> outdated record with new content (action `update` — creates a
> replacement and marks the old one invalid) or to retire a record
> entirely (action `invalidate` — no replacement). Use
> `memory.delete_memory_record` instead if you want hard deletion
> with no audit trail. The immutable audit trail of superseded
> records preserves history for replay. (AgentCore equivalent:
> closest match is `BatchUpdateMemoryRecords` with our additional
> invalidation semantics.)

**`memory.create_event`** (replacing `memory.add_event`):

> Append an immutable event to a session timeline. Use this for raw
> conversation turns and binary blobs that the agent should later be
> able to extract insights from; use `memory.create_memory_record`
> directly for already-extracted insights. Event type must be
> `conversation` (requires `content`) or `blob` (requires
> base64-encoded `blob_data`). Returns the full event with its
> generated ID and `created_at` timestamp. (AgentCore equivalent:
> `CreateEvent`.)

**`store.switch`** (replacing `memory.switch_store`):

> Switch the active SQLite store, creating it if it does not exist.
> Use this to isolate memory across projects, environments, or
> tenants — each store is a separate `.db` file. The previously
> active store is checkpointed and closed before the switch. Store
> names must be 1–64 alphanumeric characters (plus `-` / `_`).
> Returns the new active store name. (Local-only extension:
> AgentCore manages a single memory resource per agent.)

**`graph.create_edge`** (replacing `graph.add_edge`):

> Create a directed, labeled edge between two memory records. Use
> this to record typed relationships ("supersedes", "references",
> "contradicts") between extracted insights for graph traversal.
> Both records must belong to the same actor; self-edges are
> rejected. Returns the full edge object with its generated ID.
> (Local-only extension: AgentCore Memory does not expose a graph
> layer.)

---

## Documented divergences from AgentCore

These are by-design and called out in the affected tools'
descriptions:

1. **Manual extraction.** Agents call `memory.create_memory_record`
   explicitly. AgentCore extracts memory records automatically from
   events via background strategies. Reason: this server is
   embedded, single-process, and doesn't run an extraction LLM.
2. **Caller-provided embeddings.** Agents pass 384-dim vectors;
   AgentCore embeds server-side. Reason: avoid bundling an embedding
   model in a small Rust binary.
3. **Free-form `strategy` label.** Agents pick a string at write
   time; AgentCore strategies are opaque resource-managed IDs.
   Reason: the agent already chose what to extract, so a free-form
   label is more informative than an opaque ID. Recommended values:
   `semantic`, `summary`, `user_preference`, `custom_<name>`.
4. **Multi-store namespace (`store.*`).** AgentCore manages one
   memory resource per agent. We expose multiple SQLite-file stores
   for project/tenant isolation.
5. **Knowledge graph (`graph.*`).** No AgentCore counterpart.
6. **Explicit branch / checkpoint resources.** Branches appear as
   fields inside AgentCore event payloads but are not separately
   creatable. We expose `memory.create_branch` /
   `memory.create_checkpoint` as first-class operations because the
   embedded use case (workflow resumption, what-if scenarios) needs
   addressable branches.
7. **Update-with-audit-trail vs. batch update.** AgentCore's
   `BatchUpdateMemoryRecords` updates in place; our
   `memory.update_memory_record` creates a replacement record and
   marks the old one invalid, preserving an immutable audit trail.
   Reason: replay-ability and debuggability for local agent
   development.
8. **`actor_id` required on get/update/delete record ops.**
   AgentCore scopes by `memoryId` (the resource); we scope by
   `actor_id` because each store can hold many actors. Reason:
   actor isolation within a single store.

These divergences are listed in the (AgentCore equivalent: …) clause
of each affected tool's description so the LLM sees them inline.

---

## Implementation Notes (post-design-review, 2026-04-29)

Resolves all 7 High findings from the design review.

### Breaking-changes and migration guidance (resolves Rel-F1, Interop-F2, Arch-A7)

This is a **hard rename** — no backward-compatible aliases. Every harness `.mcp.json`
or system prompt that references v0.1 tool names will break on upgrade. Required
deliverables alongside the code changes:

1. **`CHANGELOG.md`** — new file at repo root, content:
   ```
   ## v0.2.0 — Breaking changes

   All 29 tool names have been realigned with AWS Bedrock AgentCore Memory naming
   conventions. There are no backward-compatible aliases — update your harness config
   before upgrading.

   ### Tool renames
   | v0.1 name | v0.2 name |
   |---|---|
   | memory.add_event | memory.create_event |
   | memory.get_events | memory.list_events |
   | memory.delete_expired | memory.delete_expired_events |
   | memory.store | memory.create_memory_record |
   | memory.get | memory.get_memory_record |
   | memory.list | memory.list_memory_records |
   | memory.recall | memory.retrieve_memory_records |
   | memory.consolidate | memory.update_memory_record |
   | memory.delete | memory.delete_memory_record |
   | memory.checkpoint | memory.create_checkpoint |
   | memory.branch | memory.create_branch |
   | graph.add_edge | graph.create_edge |
   | graph.stats | graph.get_stats |
   | memory.switch_store | store.switch |
   | memory.current_store | store.current |
   | memory.list_stores | store.list |
   | memory.delete_store | store.delete |

   ### Field renames (on affected tools only)
   | v0.1 field | v0.2 field | Affected tools |
   |---|---|---|
   | memory_id | memory_record_id | get_memory_record, update_memory_record, delete_memory_record |
   | from_memory_id / to_memory_id | from_memory_record_id / to_memory_record_id | graph.create_edge |
   | start_memory_id | start_memory_record_id | graph.traverse |
   | query | search_query | memory.retrieve_memory_records only |
   | limit | top_k | memory.retrieve_memory_records only (all other tools keep `limit`) |
   ```

2. **README.md** — add "Upgrading from v0.1" section before the tool table with
   the same rename list and a note: "Use `grep` to find calls to update."

### SERVER_INSTRUCTIONS must be updated (resolves Arch-A1, Interop-F1, Rel-F4, Maint-M4)

`src/tools.rs::SERVER_INSTRUCTIONS` references these v0.1 names in the intent guide:
`memory.add_event`, `memory.store`, `memory.recall`, `memory.list`, `memory.get`,
`memory.list_sessions`, `memory.delete_namespace`, `graph.add_edge`.

Update the intent guide section to v0.2 names:
```
- Record a conversation turn     → memory.create_event
- Save an extracted insight      → memory.create_memory_record
- Search memories by meaning     → memory.retrieve_memory_records
- Enumerate by namespace/strategy→ memory.list_memory_records
- Fetch one memory by ID         → memory.get_memory_record
- Link two memories in graph     → graph.create_edge
- Walk the knowledge graph       → graph.traverse
```

Also fix the `store.*` carve-out in the `actor_id` section. After Tier 2, the `store.*`
tools (`store.switch`, `store.current`, `store.list`, `store.delete`) do not accept
`actor_id`. Add a sentence: "The `store.*` tools operate globally — they do not
require `actor_id`."

Also fix the Tier 1 backlog Low item: replace "email hash" example with "UUID or opaque
per-user identifier" in the `actor_id` section.

### Complete source cleanup file list (resolves Maint-M1)

Update tool-name references in these files. Policy: update files that describe the
current public API; treat historical review findings in `agents/` as immutable (the
old names in review docs are correct for the time they were written).

**Must update (public API docs and source):**
- `src/tools.rs` — all `name = "..."` strings, field names, descriptions, SERVER_INSTRUCTIONS
- `tests/integration.rs` — see precise grep list below
- `tests/e2e.rs` — see precise grep list below
- `README.md` — tool tables (lines ~91–151) and prose
- `design/DESIGN.md` — feature mapping section
- `design/mcp-server.md` — tool list and descriptions
- `design/memory-tools.md` — MCP tool descriptions
- `design/event-tools.md` — MCP tool descriptions
- `design/search.md` — recall/search tool references
- `design/session-tools.md` — checkpoint/branch tool references
- `design/namespace-tools.md` — namespace tool references
- `design/knowledge-graph.md` — graph tool references
- `design/integration-tests.md` — test fixture tool names
- `design/llm-discoverability.md` — R5/R6 v0.1 name tables
- `Cargo.toml` — version → "0.2.0"

**Leave unchanged (historical records — old names are correct in context):**
- `agents/ADR.md`, `agents/TODO.md`, `agents/TIME_LOG.md`, `agents/LESSONS_LEARNED.md`

### Precise grep checklist for tests (resolves Arch-A2, Rel-F2, Maint-M3)

**In `tests/integration.rs` — field renames:**
- Replace `"memory_id"` with `"memory_record_id"`: lines 109, 135, 164, 177, 272, 485, 536
- Replace `"from_memory_id"` with `"from_memory_record_id"`: lines 259, 523, 627
- Replace `"to_memory_id"` with `"to_memory_record_id"`: lines 259, 524, 628
- Replace `"start_memory_id"` with `"start_memory_record_id"`: line 285
- Replace `"query"` with `"search_query"` on recall calls: line 211
- Replace all v0.1 tool name strings: `memory.store`→`memory.create_memory_record`,
  `memory.recall`→`memory.retrieve_memory_records`, etc.

**In `tests/e2e.rs` — tool names and field names:**
- Replace `"memory.add_event"` with `"memory.create_event"`: lines 123, 136, 220
- Replace `"memory.store"` with `"memory.create_memory_record"`: lines 148, 154, 188, 266
- Replace `"memory.recall"` with `"memory.retrieve_memory_records"`: lines 165, 171
- Replace `"graph.add_edge"` with `"graph.create_edge"`: lines 182, 203
- Replace `"query"` with `"search_query"` on recall calls: line 173
- Replace `"from_memory_id"` / `"to_memory_id"` with `*_record_id` equivalents: lines 205

**CI gate:** After the rename, verify no v0.1 names remain in source or tests:
```bash
grep -rn 'memory\.add_event\|memory\.store\b\|memory\.recall\b\|memory\.get\b\|memory\.list\b\|memory\.consolidate\|memory\.delete\b\|memory\.checkpoint\b\|memory\.branch\b\|memory\.get_events\|memory\.delete_expired\b\|memory\.switch_store\|memory\.current_store\|memory\.list_stores\|memory\.delete_store\|graph\.add_edge\|graph\.stats\b\|"memory_id"\|"from_memory_id"\|"to_memory_id"\|"start_memory_id"' src/ tests/
```
This command must produce no output before merge.

### Rust method naming (resolves Maint-M2)

To keep `grep` across source and tests coherent, rename Rust handler functions to match
the last segment of their v0.2 MCP tool name:

| Current Rust name | v0.2 MCP name | New Rust name |
|---|---|---|
| `fn add_event` | `memory.create_event` | `fn create_event` |
| `fn get_event` | `memory.get_event` | (unchanged) |
| `fn get_events` | `memory.list_events` | `fn list_events` |
| `fn delete_expired` | `memory.delete_expired_events` | `fn delete_expired_events` |
| `fn store_memory` | `memory.create_memory_record` | `fn create_memory_record` |
| `fn get_memory` | `memory.get_memory_record` | `fn get_memory_record` |
| `fn list_memories` | `memory.list_memory_records` | `fn list_memory_records` |
| `fn recall` | `memory.retrieve_memory_records` | `fn retrieve_memory_records` |
| `fn consolidate_memory` | `memory.update_memory_record` | `fn update_memory_record` |
| `fn delete_memory` | `memory.delete_memory_record` | `fn delete_memory_record` |
| `fn checkpoint` | `memory.create_checkpoint` | `fn create_checkpoint` |
| `fn branch` | `memory.create_branch` | `fn create_branch` |
| `fn add_edge` | `graph.create_edge` | `fn create_edge` |
| `fn graph_stats` | `graph.get_stats` | `fn get_stats` |
| `fn switch_store` | `store.switch` | `fn switch_store` (keep — readable) |
| `fn current_store` | `store.current` | `fn current_store` (keep) |
| `fn list_stores` | `store.list` | `fn list_stores` (keep) |
| `fn delete_store` | `store.delete` | `fn delete_store` (keep) |

Unchanged tools: `get_event`, `list_sessions`, `list_checkpoints`, `list_branches`,
`create_namespace`, `list_namespaces`, `delete_namespace`, `get_neighbors`, `traverse`,
`update_edge`, `delete_edge`, `list_labels`, `list_sessions`.

### ToolAnnotations carry over automatically (resolves Interop-F7)

Tier 1 `ToolAnnotations` are Rust function attributes (`annotations(title = "...",
read_only_hint = true)`). The rename changes only the `name = "..."` string in the
`#[tool(...)]` macro — the annotations attribute on the same function is untouched.
No re-application needed.

### Implementation plan

**Task order (sequential within each group):**

**Group A — `src/tools.rs` (all changes in one pass):**
1. Rename Rust method names per table above
2. Update `name = "..."` strings in all `#[tool(...)]` macros (13 renames + 4 namespace moves)
3. Rename param struct fields: `memory_id`→`memory_record_id` on `GetMemoryParams`,
   `ConsolidateParams`, `DeleteMemoryParams`; `from_memory_id`/`to_memory_id`→
   `from_memory_record_id`/`to_memory_record_id` on `AddEdgeParams`; `start_memory_id`→
   `start_memory_record_id` on `TraverseParams`; `query`→`search_query` and `limit`→`top_k`
   on `RecallToolParams`/`RetrieveMemoryRecordsParams` (rename the struct too)
4. Update `SERVER_INSTRUCTIONS` with v0.2 names, store.* actor_id carve-out,
   and "UUID or opaque identifier" fix
5. Update all `#[schemars(description)]` attributes that reference v0.1 tool names
   (especially `*_id` field descriptions)
6. Rewrite all 29 tool descriptions following the style guide in this document;
   include "Use this when X; use Y for Z", Returns clause, AgentCore equivalent tag
7. Add `#[schemars(description)]` to `top_k` field noting it replaces `limit` on this tool
8. Add `store.* tools do not require actor_id` note to `store.switch`/`store.current`/
   `store.list`/`store.delete` descriptions
9. Run `cargo check` — fix all compile errors (serde field renames cascade to call sites
   within tools.rs only; all other callers are through the MCP JSON layer)

**Group B — tests (after Group A passes `cargo check`):**
10. Update `tests/integration.rs` — all field names and tool names per grep checklist
11. Update `tests/e2e.rs` — all tool names and field names per grep checklist
12. Run `cargo test` — all 151 must pass

**Group C — docs and version (can run in parallel with Group B):**
13. Update `README.md` — tool tables + add "Upgrading from v0.1" section
14. Update design docs: `design/DESIGN.md`, `design/mcp-server.md`, `design/memory-tools.md`,
    `design/event-tools.md`, `design/search.md`, `design/session-tools.md`,
    `design/namespace-tools.md`, `design/knowledge-graph.md`, `design/integration-tests.md`,
    `design/llm-discoverability.md`
15. Bump `Cargo.toml` version → `"0.2.0"`
16. Create `CHANGELOG.md` with v0.2.0 breaking-changes table

**Group D — final gate:**
17. Run the CI grep command (see above) — must produce no output
18. Run `cargo clippy -- -D warnings` — must pass clean

---

## References

### AgentCore Memory documentation

- AgentCore Memory overview:
  https://docs.aws.amazon.com/bedrock-agentcore/latest/devguide/what-is-bedrock-agentcore.html
- Short-term vs long-term:
  https://docs.aws.amazon.com/bedrock-agentcore/latest/devguide/memory-types.html
- Namespaces, templates, actors:
  https://docs.aws.amazon.com/bedrock-agentcore/latest/devguide/memory-organization.html
- Memory strategies:
  https://docs.aws.amazon.com/bedrock-agentcore/latest/devguide/memory-strategies.html

### AgentCore API reference (per-operation)

- `CreateEvent`:
  https://docs.aws.amazon.com/bedrock-agentcore/latest/APIReference/API_CreateEvent.html
- `GetEvent`:
  https://docs.aws.amazon.com/bedrock-agentcore/latest/APIReference/API_GetEvent.html
- `ListEvents`:
  https://docs.aws.amazon.com/bedrock-agentcore/latest/APIReference/API_ListEvents.html
- `ListSessions`:
  https://docs.aws.amazon.com/bedrock-agentcore/latest/APIReference/API_ListSessions.html
- `GetMemoryRecord`:
  https://docs.aws.amazon.com/bedrock-agentcore/latest/APIReference/API_GetMemoryRecord.html
- `ListMemoryRecords`:
  https://docs.aws.amazon.com/bedrock-agentcore/latest/APIReference/API_ListMemoryRecords.html
- `RetrieveMemoryRecords`:
  https://docs.aws.amazon.com/bedrock-agentcore/latest/APIReference/API_RetrieveMemoryRecords.html
- Batch ops & `Delete*`:
  https://docs.aws.amazon.com/bedrock-agentcore/latest/APIReference/API_Operations.html

### Internal cross-references

- `design/DESIGN.md` — original AgentCore parity statement and
  feature mapping.
- `design/llm-discoverability.md` — broader discoverability audit;
  R3 + R4 are now satisfied by this document's rename table.
- `design/mcp-server.md` H10 / H11 — already-recorded gaps about
  parameter naming divergences and description verbosity.
- `agents/TODO.md` "LLM Harness Discoverability" — implementation
  schedule for the v0.2 surface change.
