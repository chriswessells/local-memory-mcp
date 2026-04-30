# Changelog

## v0.2.0 — Breaking changes

All 29 tool names have been realigned with AWS Bedrock AgentCore Memory naming
conventions. There are no backward-compatible aliases — update your harness config
before upgrading.

### Tool renames

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

### Field renames (on affected tools only)

| v0.1 field | v0.2 field | Affected tools |
|---|---|---|
| `memory_id` | `memory_record_id` | `memory.get_memory_record`, `memory.update_memory_record`, `memory.delete_memory_record`, `graph.get_neighbors` |
| `from_memory_id` / `to_memory_id` | `from_memory_record_id` / `to_memory_record_id` | `graph.create_edge` |
| `start_memory_id` | `start_memory_record_id` | `graph.traverse` |
| `query` | `search_query` | `memory.retrieve_memory_records` only — other tools are unaffected |
| `limit` | `top_k` | `memory.retrieve_memory_records` only (all other tools keep `limit`) |

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

29 MCP tools covering events, long-term memory records, knowledge graph, sessions,
namespaces, and multi-store management over a local SQLite database.
