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
