# Design: `metadata` and `properties` as `serde_json::Value` (R7)

## Goal

Change `metadata` (events, memories, checkpoints) and graph `properties` from
`Option<String>` to `Option<serde_json::Value>` throughout the MCP tool layer
and domain structs. Callers currently must double-serialize JSON
(`"metadata": "{\"key\":\"val\"}"`); after this change they pass a real object
(`"metadata": {"key": "val"}`). The JSON Schema exposed by schemars will
reflect the actual shape, improving LLM discoverability.

SQLite still stores these columns as `TEXT`. The conversion happens at the
DB boundary — all layers above it use `serde_json::Value`.

---

## Backward compatibility with existing stores

v0.2 stored metadata by receiving a JSON string from the MCP client (e.g.,
`"metadata": "{\"source\":\"user\"}"`) and storing its content directly in the
SQLite TEXT column (i.e., the column contains `{"source":"user"}`). This is
valid JSON. When v0.3 calls `serde_json::from_str` on that TEXT value it
correctly recovers a `Value::Object`. **No migration is required.** Existing
databases are fully compatible with v0.3.

---

## Affected surfaces

### Tool param structs (`src/tools.rs`)

| Struct | Field | Old type | New type |
|--------|-------|----------|----------|
| `CreateEventToolParams` | `metadata` | `Option<String>` | `Option<serde_json::Value>` |
| `CreateMemoryRecordParams` | `metadata` | `Option<String>` | `Option<serde_json::Value>` |
| `CreateCheckpointToolParams` | `metadata` | `Option<String>` | `Option<serde_json::Value>` |
| `CreateEdgeToolParams` | `properties` | `Option<String>` | `Option<serde_json::Value>` |
| `UpdateEdgeToolParams` | `properties` | `Option<String>` | `Option<serde_json::Value>` |

### Domain output structs

| Struct | File | Field | Old | New |
|--------|------|-------|-----|-----|
| `Event` | `events.rs` | `metadata` | `Option<String>` | `Option<serde_json::Value>` |
| `Memory` | `memories.rs` | `metadata` | `Option<String>` | `Option<serde_json::Value>` |
| `Checkpoint` | `sessions.rs` | `metadata` | `Option<String>` | `Option<serde_json::Value>` |
| `Edge` | `graph.rs` | `properties` | `Option<String>` | `Option<serde_json::Value>` |

### Domain input param structs

| Struct | File | Field | Old type | New type |
|--------|------|-------|----------|----------|
| `InsertEventParams` | `events.rs` | `metadata` | `Option<&'a str>` | `Option<serde_json::Value>` |
| `InsertMemoryParams` | `memories.rs` | `metadata` | `Option<&'a str>` | `Option<serde_json::Value>` |
| `InsertCheckpointParams` | `sessions.rs` | `metadata` | `Option<&'a str>` | `Option<serde_json::Value>` |
| `InsertEdgeParams` | `graph.rs` | `properties` | `Option<&'a str>` | `Option<serde_json::Value>` |
| `UpdateEdgeParams` | `graph.rs` | `properties` | `Option<&'a str>` | `Option<serde_json::Value>` |

The lifetime `'a` on these fields is dropped. All other `'a` fields in each
struct are unaffected — lifetimes remain valid.

---

## Constants

Add to `src/events.rs` (and re-export / use in other modules as needed):

```rust
pub const MAX_METADATA_KEYS: usize = 50;
pub const MAX_METADATA_DEPTH: usize = 5;
```

Add to `src/graph.rs`:

```rust
pub const MAX_PROPERTIES_KEYS: usize = 50;
pub const MAX_PROPERTIES_DEPTH: usize = 5;
```

---

## DB boundary: serialization contract

SQLite columns stay `TEXT`. All conversions happen in `db.rs`.

### On write (INSERT / UPDATE)

```rust
fn serialize_json_opt(v: &Option<serde_json::Value>) -> rusqlite::Result<Option<String>> {
    match v {
        None => Ok(None),
        // serde_json::Value is always serializable; unwrap is safe here.
        Some(val) => Ok(Some(serde_json::to_string(val)
            .expect("serde_json::Value is always serializable"))),
    }
}
```

Every existing bind site that currently does `":metadata": params.metadata`
becomes `":metadata": serialize_json_opt(&params.metadata)?`.

### On read (row mappers)

```rust
fn parse_json_opt(raw: Option<String>) -> rusqlite::Result<Option<serde_json::Value>> {
    match raw {
        None => Ok(None),
        Some(text) => serde_json::from_str(&text)
            .map(Some)
            .map_err(|e| rusqlite::Error::FromSqlConversionFailure(
                0, // column index unknown at this level; see call-site comments
                rusqlite::types::Type::Text,
                Box::new(e),
            )),
    }
}
```

Row mappers change from `metadata: row.get(n)?` to:
```rust
// The first `?` propagates rusqlite column-read errors.
// The second `?` propagates JSON parse errors via parse_json_opt.
metadata: parse_json_opt(row.get(n)?)?,
```

### Row-level fallback for corrupted rows

In list queries (`query_map` iterations for `list_events`, `list_memories`,
`list_memory_records`, `get_neighbors`, `traverse`), a single row with
unparseable metadata must not abort the entire result set. Apply a fallback
in the row mapper:

```rust
fn parse_json_opt_lenient(raw: Option<String>) -> Option<serde_json::Value> {
    raw.and_then(|text| {
        serde_json::from_str(&text)
            .map_err(|e| {
                tracing::warn!(error = %e, "metadata/properties JSON parse failed; returning null");
            })
            .ok()
    })
}
```

Use `parse_json_opt` (strict, returns `Err`) in INSERT RETURNING and single-row
GET queries where a failure is unexpected and should surface.
Use `parse_json_opt_lenient` (soft, returns `None` on error) in all
`query_map`-based list/search queries. This prevents one corrupted row from
poisoning an entire result set.

### Error handling on read (strict path)

A deserialization failure in a single-row GET maps to
`rusqlite::Error::FromSqlConversionFailure`, which existing infrastructure
converts to `MemoryError::QueryFailed`. No new error variant is needed.
The `QueryFailed` MCP response must not include the raw column text — verify
in `error.rs` that the inner rusqlite error is not exposed verbatim.

---

## Validation

```rust
fn validate_json_object_value(v: &serde_json::Value, field: &str) -> Result<(), MemoryError> {
    if !v.is_object() {
        return Err(MemoryError::InvalidInput(format!(
            "{field} must be a JSON object"
        )));
    }
    Ok(())
}

fn json_value_depth(v: &serde_json::Value) -> usize {
    match v {
        serde_json::Value::Object(m) => {
            1 + m.values().map(json_value_depth).max().unwrap_or(0)
        }
        serde_json::Value::Array(a) => {
            1 + a.iter().map(json_value_depth).max().unwrap_or(0)
        }
        _ => 0,
    }
}
```

Full validation sequence for `metadata` (same pattern for `properties`):

```rust
if let Some(ref v) = params.metadata {
    validate_json_object_value(v, "metadata")?;

    let obj = v.as_object().expect("validated as object above");
    if obj.len() > MAX_METADATA_KEYS {
        return Err(MemoryError::InvalidInput(format!(
            "metadata exceeds maximum of {MAX_METADATA_KEYS} keys"
        )));
    }
    if json_value_depth(v) > MAX_METADATA_DEPTH {
        return Err(MemoryError::InvalidInput(format!(
            "metadata exceeds maximum nesting depth of {MAX_METADATA_DEPTH}"
        )));
    }

    // serde_json::Value is always serializable; unwrap is safe.
    let serialized = serde_json::to_string(v)
        .expect("serde_json::Value is always serializable");
    if serialized.len() > MAX_METADATA_SIZE {
        return Err(MemoryError::InvalidInput(format!(
            "metadata exceeds maximum length of {MAX_METADATA_SIZE} bytes"
        )));
    }
}
```

Notes:
- `serde_json::to_string(v)` cannot fail for a `serde_json::Value` — use `expect`.
- Size limit applies to compact-serialized form.
- Validation serializes once; the DB bind site serializes again via
  `serialize_json_opt`. This double-serialize is acceptable (metadata is small,
  path is non-critical).

---

## Tool layer bridge (`tools.rs`)

Tool handlers currently do `params.metadata.as_deref()` to pass
`Option<&str>` to domain params. After the change, domain params accept
`Option<serde_json::Value>`, so the bridge becomes:

```rust
// before
metadata: params.metadata.as_deref(),

// after
metadata: params.metadata.clone(),
```

Similarly for `properties`. The `.clone()` is correct — `serde_json::Value`
clones the JSON tree, which is fine for the small objects these fields hold.

---

## Wire format: breaking change

| Version | Wire representation |
|---------|---------------------|
| v0.2 | `"metadata": "{\"source\":\"user\"}"` (string) |
| v0.3 | `"metadata": {"source": "user"}` (object) |

This is a **breaking wire change** for any caller that passes `metadata` or
`properties`. Callers that pass `null` or omit these fields are unaffected.
Document in `CHANGELOG.md` under a new `v0.3.0` section.

---

## No schema migration

SQLite columns are already `TEXT`. Serialized JSON strings are stored and
retrieved identically. No `PRAGMA user_version` bump, no migration needed.

---

## schemars impact

`Option<serde_json::Value>` without guidance generates a loose schema.
Use a custom helper that explicitly allows both `null` and `object`:

```rust
fn json_object_schema(_gen: &mut schemars::gen::SchemaGenerator) -> schemars::schema::Schema {
    use schemars::schema::{InstanceType, SchemaObject, SingleOrVec, SubschemaValidation};
    // schema_with functions receive the generator for recursive schema building;
    // not needed for a leaf type like {type: object}.
    SchemaObject {
        subschemas: Some(Box::new(SubschemaValidation {
            any_of: Some(vec![
                SchemaObject {
                    instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::Null))),
                    ..Default::default()
                }
                .into(),
                SchemaObject {
                    instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::Object))),
                    ..Default::default()
                }
                .into(),
            ]),
            ..Default::default()
        })),
        ..Default::default()
    }
    .into()
}
```

Applied via `#[schemars(schema_with = "json_object_schema")]` on each field.
This generates `anyOf: [null, {type: object}]` — correct for an optional
JSON object. Add a unit test verifying the generated schema includes `null`
(see Sub-agent D instructions).

---

## Implementation plan

### Task 1: DB helpers (independent)
Add `serialize_json_opt`, `parse_json_opt`, and `parse_json_opt_lenient`
private functions near the top of `db.rs` (after `escape_like`).
**Acceptance**: Functions compile.

### Task 2: Domain output structs + input param structs (independent)
Change fields in `events.rs`, `memories.rs`, `sessions.rs`, `graph.rs`.
Add `MAX_METADATA_KEYS`, `MAX_METADATA_DEPTH` constants.
Update validation: replace string-parse checks with `validate_json_object_value`
+ key count + depth + serialize-then-check-len.
Add `json_value_depth` helper (place in the first module that uses it; the
others can import or duplicate — it's small).
Update broken unit tests in `sessions.rs` (see Sub-agent B instructions).
**Acceptance**: `cargo check` passes, unit tests in each module pass.

### Task 3: DB row mappers + bind sites (requires Tasks 1 + 2)
Update all `row_to_*` functions in `db.rs` using the strict/lenient split.
Update all bind sites.
Add roundtrip unit tests.
**Acceptance**: `cargo test` passes.

### Task 4: Tool param structs + bridge (requires Task 2)
Change `Option<String>` fields in `tools.rs`.
Add `json_object_schema` helper and `schema_with` annotations.
Update bridge call sites.
Update `SERVER_INSTRUCTIONS` examples.
Update schemars descriptions to remove "JSON object string" phrasing.
Add schema shape test.
**Acceptance**: `cargo check`, `cargo test`, `cargo clippy` all clean.

### Task 5: CHANGELOG + README + TODO (requires Tasks 3 + 4)
Add `v0.3.0` section to `CHANGELOG.md`.
Update `README.md` parameter type descriptions for `metadata`/`properties`.
Mark R7 complete in `agents/TODO.md`.
Log time in `agents/TIME_LOG.md`.

---

## DAG

```
Task 1 (DB helpers)      ─┐
                           ├─► Task 3 (DB row mappers + bind sites) ─┐
Task 2 (domain structs)  ─┤                                           ├─► Task 5
                           └─► Task 4 (tool structs + bridge)        ─┘
```

Tasks 1 and 2 can run in parallel.
Task 3 requires Tasks 1 and 2.
Task 4 requires Task 2 only.
Task 5 requires Tasks 3 and 4.

---

## Sub-agent instructions

**Pre-conditions**: Zero open Critical/High findings. Build target:
`cargo check && cargo test && cargo clippy --all-targets -- -D warnings`.

---

### Sub-agent A: DB helpers (Task 1)

File: `src/db.rs`

Add the following three private functions directly after the `escape_like`
function:

```rust
fn serialize_json_opt(v: &Option<serde_json::Value>) -> rusqlite::Result<Option<String>> {
    match v {
        None => Ok(None),
        // serde_json::Value is always serializable; unwrap is safe here.
        Some(val) => Ok(Some(
            serde_json::to_string(val).expect("serde_json::Value is always serializable"),
        )),
    }
}

fn parse_json_opt(raw: Option<String>) -> rusqlite::Result<Option<serde_json::Value>> {
    match raw {
        None => Ok(None),
        Some(text) => serde_json::from_str(&text).map(Some).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                0, // column index not tracked at this helper level
                rusqlite::types::Type::Text,
                Box::new(e),
            )
        }),
    }
}

fn parse_json_opt_lenient(raw: Option<String>) -> Option<serde_json::Value> {
    raw.and_then(|text| {
        serde_json::from_str(&text)
            .map_err(|e| {
                tracing::warn!(error = %e, "metadata/properties JSON parse failed; returning null");
            })
            .ok()
    })
}
```

Run `cargo check`. Done.

---

### Sub-agent B: Domain structs and params (Task 2)

**Constants to add:**

In `src/events.rs`, add alongside existing constants:
```rust
pub const MAX_METADATA_KEYS: usize = 50;
pub const MAX_METADATA_DEPTH: usize = 5;
```

In `src/graph.rs`, add alongside existing constants:
```rust
pub const MAX_PROPERTIES_KEYS: usize = 50;
pub const MAX_PROPERTIES_DEPTH: usize = 5;
```

**Depth helper** — add to `src/events.rs` (used there; `memories.rs` and
`sessions.rs` import it via `use crate::events::json_value_depth`):
```rust
pub fn json_value_depth(v: &serde_json::Value) -> usize {
    match v {
        serde_json::Value::Object(m) => {
            1 + m.values().map(json_value_depth).max().unwrap_or(0)
        }
        serde_json::Value::Array(a) => {
            1 + a.iter().map(json_value_depth).max().unwrap_or(0)
        }
        _ => 0,
    }
}
```

`graph.rs` defines its own copy (same code, different module) to avoid a
cross-module dependency just for this small helper.

**Validation helper** — add to each module (or share via `events.rs`):
```rust
fn validate_json_object_value(v: &serde_json::Value, field: &str) -> Result<(), MemoryError> {
    if !v.is_object() {
        return Err(MemoryError::InvalidInput(format!(
            "{field} must be a JSON object"
        )));
    }
    Ok(())
}
```

**`src/events.rs`**

1. `Event.metadata`: `Option<String>` → `Option<serde_json::Value>`
2. `InsertEventParams.metadata`: `Option<&'a str>` → `Option<serde_json::Value>`
3. Replace the metadata validation block in `validate_insert_params`:
   ```rust
   if let Some(ref v) = params.metadata {
       validate_json_object_value(v, "metadata")?;
       let obj = v.as_object().expect("validated as object above");
       if obj.len() > MAX_METADATA_KEYS {
           return Err(MemoryError::InvalidInput(format!(
               "metadata exceeds maximum of {MAX_METADATA_KEYS} keys"
           )));
       }
       if json_value_depth(v) > MAX_METADATA_DEPTH {
           return Err(MemoryError::InvalidInput(format!(
               "metadata exceeds maximum nesting depth of {MAX_METADATA_DEPTH}"
           )));
       }
       let serialized = serde_json::to_string(v)
           .expect("serde_json::Value is always serializable");
       if serialized.len() > MAX_METADATA_SIZE {
           return Err(MemoryError::InvalidInput(format!(
               "metadata exceeds maximum length of {MAX_METADATA_SIZE} bytes"
           )));
       }
   }
   ```

**`src/memories.rs`**

1. `Memory.metadata`: `Option<String>` → `Option<serde_json::Value>`
2. `InsertMemoryParams.metadata`: `Option<&'a str>` → `Option<serde_json::Value>`
3. Replace the metadata validation block in `validate_insert_memory_params`
   with the same pattern as events (using `MAX_METADATA_KEYS`, `MAX_METADATA_DEPTH`,
   `MAX_METADATA_SIZE`, `json_value_depth` from `crate::events`).

**`src/sessions.rs`**

1. `Checkpoint.metadata`: `Option<String>` → `Option<serde_json::Value>`
2. `InsertCheckpointParams.metadata`: `Option<&'a str>` → `Option<serde_json::Value>`
3. Replace the `validate_metadata_json` call and `validate_max_len` call in
   `validate_checkpoint_params` with the same pattern.
4. Remove the now-dead `validate_metadata_json` helper.
5. **Update unit tests** — two tests currently pass `Option<&str>` literals that
   will be compile errors after the type change:
   - `test_validate_metadata_not_object`: change `metadata: Some("[]")` to
     `metadata: Some(serde_json::json!([]))` — an array is still not an object.
   - `test_validate_metadata_invalid_json`: change `metadata: Some("not json")` to
     `metadata: Some(serde_json::json!("not an object"))` — a string Value is
     still not an object.
   The error assertion (`"metadata must be a JSON object"`) remains valid.

**`src/graph.rs`**

1. `Edge.properties`: `Option<String>` → `Option<serde_json::Value>`
2. `InsertEdgeParams.properties`: `Option<&'a str>` → `Option<serde_json::Value>`
3. `UpdateEdgeParams.properties`: `Option<&'a str>` → `Option<serde_json::Value>`
4. Replace `validate_max_len(props, ...) + validate_json_object(props, ...)` in
   `validate_insert_edge_params` and `validate_update_edge_params` with the
   same pattern (using `MAX_PROPERTIES_KEYS`, `MAX_PROPERTIES_DEPTH`,
   `MAX_PROPERTIES_SIZE`).
5. Remove `validate_json_object` if it is now dead code.

Run `cargo check`. Verify `cargo test` passes for each module's unit tests.

---

### Sub-agent C: DB row mappers and bind sites (Task 3)

**Prerequisite**: Sub-agents A and B complete and compiling.

File: `src/db.rs`

**Which mapper uses strict vs lenient:**

| Function | Query type | Use |
|----------|-----------|-----|
| `row_to_event` (single GET) | Single row | `parse_json_opt` (strict) |
| `row_to_event` (in list query) | `query_map` | `parse_json_opt_lenient` |
| `row_to_memory` (in INSERT RETURNING / single GET) | Single row | `parse_json_opt` (strict) |
| `row_to_memory` (in list / search queries) | `query_map` | `parse_json_opt_lenient` |
| `row_to_edge` | Mixed | Use `parse_json_opt_lenient` (edges appear in lists) |
| `row_to_neighbor` | `query_map` | `parse_json_opt_lenient` for both edge.properties and memory.metadata |
| `row_to_traversal_node` | `query_map` | `parse_json_opt_lenient` for memory.metadata |
| Checkpoint row (in `create_checkpoint` RETURNING ~line 1856) | Single row | `parse_json_opt` (strict) |
| Checkpoint row (in `list_checkpoints` ~line 2012) | `query_map` | `parse_json_opt_lenient` |

Because the existing `row_to_event` and `row_to_memory` helpers are shared
between single and list call sites, split them into two variants or inline
the logic at the call site. The simplest approach: rename the existing helpers
to `row_to_event_strict` and `row_to_event_lenient` (and equivalently for
memory, edge), and use the appropriate one at each call site.

Alternatively: use `parse_json_opt` everywhere but catch the error in the
`query_map` closure to substitute `None`. Either approach is acceptable; pick
the one that touches fewer lines.

**Row mappers — update each metadata/properties read:**

`row_to_event` (line ~425), col 7:
```rust
// strict: metadata: parse_json_opt(row.get(7)?)?,
// lenient: metadata: parse_json_opt_lenient(row.get(7)?),
```

`row_to_memory` (line ~441), col 5:
```rust
metadata: parse_json_opt[_lenient](row.get(5)?)[?],
```

`row_to_edge` (line ~472), col 4:
```rust
properties: parse_json_opt_lenient(row.get(4)?),
```

`row_to_neighbor` (line ~483):
- edge.properties: col 4 → `parse_json_opt_lenient(row.get(4)?)`
- memory.metadata: col 11 → `parse_json_opt_lenient(row.get(11)?)`

`row_to_traversal_node` (line ~508):
- memory.metadata: col 8 → `parse_json_opt_lenient(row.get(8)?)`

Checkpoint in `create_checkpoint` (~line 1856 RETURNING), col 5:
```rust
metadata: parse_json_opt(row.get(5)?)?,
```

Checkpoint in `list_checkpoints` (~line 2012), col 5:
```rust
metadata: parse_json_opt_lenient(row.get(5)?),
```

**INSERT bind sites** — update every `":metadata": params.metadata` and
`":properties": params.properties`:
```rust
":metadata": serialize_json_opt(&params.metadata)?,
":properties": serialize_json_opt(&params.properties)?,
```

Search for `":metadata"` and `":properties"` in `db.rs` to find all ~9 sites.

**Roundtrip tests** — add to the `#[cfg(test)]` section in `db.rs`:

```rust
#[test]
fn test_metadata_roundtrip() {
    let (_dir, conn) = open_db(); // use existing open_db helper in db.rs tests
    let meta = serde_json::json!({"source": "user", "confidence": 0.9});
    let params = crate::memories::InsertMemoryParams {
        actor_id: "a1",
        content: "test content",
        strategy: "semantic",
        namespace: None,
        metadata: Some(meta.clone()),
        source_session_id: None,
        embedding: None,
    };
    let memory = conn.insert_memory(&params).unwrap();
    assert_eq!(memory.metadata, Some(meta));
}

#[test]
fn test_properties_roundtrip() {
    let (_dir, conn) = open_db();
    let m1 = conn.insert_memory(&crate::memories::InsertMemoryParams {
        actor_id: "a1", content: "A", strategy: "semantic",
        namespace: None, metadata: None, source_session_id: None, embedding: None,
    }).unwrap();
    let m2 = conn.insert_memory(&crate::memories::InsertMemoryParams {
        actor_id: "a1", content: "B", strategy: "semantic",
        namespace: None, metadata: None, source_session_id: None, embedding: None,
    }).unwrap();
    let props = serde_json::json!({"weight": 0.8});
    let edge = conn.insert_edge(&crate::graph::InsertEdgeParams {
        actor_id: "a1",
        from_memory_id: &m1.id,
        to_memory_id: &m2.id,
        label: "relates_to",
        properties: Some(props.clone()),
    }).unwrap();
    assert_eq!(edge.properties, Some(props));
}
```

Run `cargo test`. All tests must pass.

---

### Sub-agent D: Tool param structs and bridge (Task 4)

**Prerequisite**: Sub-agent B complete.

File: `src/tools.rs`

**Step 1**: Add the `json_object_schema` helper near the top of the file
(after imports, before the first `impl` or struct):

```rust
fn json_object_schema(_gen: &mut schemars::gen::SchemaGenerator) -> schemars::schema::Schema {
    use schemars::schema::{InstanceType, SchemaObject, SingleOrVec, SubschemaValidation};
    // schema_with functions receive the generator for recursive schema building;
    // not needed for this leaf type. Generates anyOf: [null, {type: object}].
    SchemaObject {
        subschemas: Some(Box::new(SubschemaValidation {
            any_of: Some(vec![
                SchemaObject {
                    instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::Null))),
                    ..Default::default()
                }
                .into(),
                SchemaObject {
                    instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::Object))),
                    ..Default::default()
                }
                .into(),
            ]),
            ..Default::default()
        })),
        ..Default::default()
    }
    .into()
}
```

**Step 2**: Update each tool param struct field. Replace existing
`metadata: Option<String>` annotations (lines 167, 250, 561) with:

```rust
#[schemars(
    schema_with = "json_object_schema",
    description = "Optional JSON object of arbitrary key-value pairs, \
                   e.g. {\"source\": \"user\", \"confidence\": 0.9}. \
                   Maximum 50 keys, depth 5."
)]
metadata: Option<serde_json::Value>,
```

Replace existing `properties: Option<String>` annotations (lines 416, 474) with:

```rust
#[schemars(
    schema_with = "json_object_schema",
    description = "Optional JSON object of edge properties, \
                   e.g. {\"weight\": 0.8, \"since\": \"2025-01\"}. \
                   Maximum 50 keys, depth 5."
)]
properties: Option<serde_json::Value>,
```

**Step 3**: Update bridge sites in tool handlers:
```rust
// OLD
metadata: params.metadata.as_deref(),
// NEW
metadata: params.metadata.clone(),
```
Sites: lines 704, 828, 1306 (metadata) and lines 1142, 1221 (properties).

**Step 4**: Search `SERVER_INSTRUCTIONS` (the large string constant in
`tools.rs`) for occurrences of `metadata` and `properties` in example
snippets. Update any examples that show `"metadata": "{...}"` (string) to
`"metadata": {...}` (object).

**Step 5**: Add a schema shape test to the `#[cfg(test)]` section in
`tools.rs` or a new test module:

```rust
#[test]
fn test_metadata_schema_is_nullable_object() {
    let schema = schemars::schema_for!(CreateEventToolParams);
    let props = schema.schema.object.as_ref().unwrap();
    let meta_schema = &props.properties["metadata"];
    // Should be anyOf containing null and object
    let schema_json = serde_json::to_string(meta_schema).unwrap();
    assert!(schema_json.contains("null"), "metadata schema must allow null");
    assert!(schema_json.contains("object"), "metadata schema must allow object");
}
```

Run `cargo check && cargo test && cargo clippy --all-targets -- -D warnings`.

---

### Sub-agent E: CHANGELOG, README, and tracking (Task 5)

**Prerequisite**: All build checks pass.

**`CHANGELOG.md`**: Add a new section at the top:

```markdown
## [v0.3.0] — Unreleased

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
```

**`README.md`**: Find any table rows or parameter descriptions that list
`metadata` or `properties` as type `string` (or "JSON string") and update
them to type `object`.

**`agents/TODO.md`**: Mark R7 complete:
```
- [x] R7: Change `metadata` and graph `properties` from `Option<String>` to `Option<serde_json::Value>` so the JSON Schema reflects the actual object shape
```

**`agents/TIME_LOG.md`**: Add an entry for this task.
