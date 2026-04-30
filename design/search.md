# Component 4: Search (FTS5 + Vector) — Detailed Design

## Scope

Full-text and vector similarity search over long-term memories. This component adds:

1. **Data types** — `SearchResult`, `RecallParams`, `SearchFtsParams`, `SearchVectorParams` in `search.rs`
2. **Db trait methods** — 2 methods added to the `Db` trait in `db.rs` (`search_fts`, `search_vector`)
3. **Business logic** — FTS5 query sanitization, hybrid search with Reciprocal Rank Fusion (RRF), validation + delegation in `search.rs`

This component does NOT include:
- MCP tool definitions (Component 8)
- Memory CRUD (Component 3 — already done)
- Namespace CRUD (Component 7)

**Relationship to `memory.retrieve_memory_records`**: The MCP tool `memory.retrieve_memory_records` (Component 8) will call `search::recall()` from this component. The recall function orchestrates FTS5-only, vector-only, or hybrid search depending on which parameters the caller provides.

---

## Design Review Resolutions (Round 1)

### High

- **H1 — Vector over-fetch starvation with fixed multiplier** (4 reviewers): Replaced magic `limit * 4` with named constants `VECTOR_OVERFETCH_FACTOR = 4` and `MAX_K_OVERFETCH = 1000`. Documented the known limitation for multi-actor stores.
- **H5 — Silent hybrid fallback to vector-only**: Added `tracing::warn!` when hybrid search falls back to vector-only due to empty sanitized query.
- **H6 — Db trait method signatures have high arity**: Introduced `SearchFtsParams` and `SearchVectorParams` structs, consistent with existing `ListMemoriesParams`/`GetEventsParams` pattern. Makes trait extensible without signature changes.
- **H7 — No token count cap in sanitize_fts_query**: Added `MAX_FTS_TOKENS = 64` cap via `.take()` in the sanitization pipeline. Added `MAX_QUERY_LEN = 4096` for query string length validation. Prevents DoS via expensive FTS5 queries.
- **H10 — Cap k_overfetch independently**: `MAX_K_OVERFETCH = 1000` caps the KNN candidate count regardless of `limit * factor`.

### Not search-specific (logged to backlog)

- unsafe transmute fragility (existing, already documented in ADR/code)
- Store switch during in-flight search (store.rs concern)
- No query timeout / cancellation (server-wide concern)
- FTS5 content-sync crash recovery (already in backlog as `memory.rebuild_index`)
- Schema migration path documentation (v1 migration already creates tables)

### Medium/Low (logged to TODO backlog)

See TODO.md backlog section.

---

## Design Review Resolutions (Round 2)

### High

- **R2-H1 — `debug_assert_eq!` for embedding dimension stripped in release builds** (2 reviewers): Changed to a runtime check (`if params.embedding.len() != EMBEDDING_DIM`) in `search_vector` impl. Defense-in-depth alongside `validate_recall_params`.

### Medium (addressed in design)

- **R2-M1 — Hybrid `fetch_limit` unbounded**: Capped at `(limit * 2).min(MAX_PAGE_LIMIT)`. Added comment documenting the compounding over-fetch with `VECTOR_OVERFETCH_FACTOR`.
- **R2-M2 — Score incomparability undocumented**: Added explicit warning to `SearchResult.score` docstring.

### Medium/Low (logged to TODO backlog)

- `escape_like` reuse: sub-agent instructions already reference it; added explicit note
- `SanitizedFtsQuery` newtype for FTS injection safety at trait boundary
- `validate_recall_params` body not shown — implementer follows validation table
- Vector search filter tests (namespace, valid_only) for parity with FTS tests
- RRF eager clone optimization (`or_insert_with` instead of `or_insert`)
- Hyphen stripping comment (aligned with FTS5 unicode61 tokenizer)
- `search_vector` doc comment: clarify returns raw L2 distance
- Structured tracing fields for hybrid fallback warn log

---

## FTS5 Injection Prevention

This is the designated owner of FTS5 query safety (per core-db-layer.md C4).

FTS5 `MATCH` syntax supports operators (`AND`, `OR`, `NOT`, `NEAR`, `*`, `"..."`, column filters, `^`). Passing raw user input to `MATCH` allows:
- Query syntax errors (unbalanced quotes, invalid operators) → SQLite error
- Wildcard abuse (`*` prefix) → expensive scans
- Column filter injection (`content:`) → not dangerous here (single column) but still unexpected

**Strategy**: Strip all FTS5 special characters and treat user input as a bag of words. Each word becomes a quoted term. Words are joined with implicit AND (FTS5 default).

```
User input:  "Rust OR DROP TABLE"
Sanitized:   "rust" "or" "drop" "table"
FTS5 sees:   match all four words (implicit AND)
```

**Sanitization rules**:
1. Split input on whitespace
2. For each token, remove all characters that are not alphanumeric or `_` (strip `"`, `*`, `(`, `)`, `:`, `^`, `{`, `}`, `+`, `-`)
3. Discard empty tokens after stripping
4. Take at most `MAX_FTS_TOKENS` (64) tokens — discard the rest to bound FTS5 query complexity
5. Wrap each surviving token in double quotes: `"token"`
6. Join with spaces (FTS5 implicit AND)
7. If no tokens survive, return empty results (don't query FTS5)

```rust
/// Maximum number of tokens in a sanitized FTS5 query.
/// Bounds the number of posting list intersections FTS5 must perform.
const MAX_FTS_TOKENS: usize = 64;

/// Maximum length of a search query string (bytes).
/// Search queries should be short — not document-sized.
pub const MAX_QUERY_LEN: usize = 4096;
```

This prevents all FTS5 operator injection while preserving useful search behavior. The double-quoting ensures each token is treated as a literal phrase (single word).

---

## Data Types

```rust
// src/search.rs

use serde::{Deserialize, Serialize};
use crate::memories::Memory;

/// A memory with a relevance score from search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    #[serde(flatten)]
    pub memory: Memory,
    /// Relevance score. Higher is better. Scale depends on search strategy:
    /// - FTS-only: negated BM25 rank (higher = more relevant, unbounded)
    /// - Vector-only: `1.0 / (1.0 + L2_distance)` (higher = more similar, range 0.0–1.0)
    /// - Hybrid: RRF score (higher = more relevant, range ~0.0–0.033)
    ///
    /// Scores from different strategies are NOT comparable.
    /// Do not apply a single threshold across FTS-only, vector-only, and hybrid results.
    pub score: f64,
}

/// Parameters for the recall function.
#[derive(Debug, Clone)]
pub struct RecallParams<'a> {
    pub actor_id: &'a str,
    pub query: Option<&'a str>,           // FTS5 text search
    pub embedding: Option<&'a [f32]>,     // vector similarity search
    pub namespace: Option<&'a str>,       // exact namespace match
    pub namespace_prefix: Option<&'a str>,// namespace prefix match
    pub strategy: Option<&'a str>,        // filter by strategy
    pub limit: u32,
}
```

---

## Db Trait Methods

Added to `pub trait Db` in `db.rs` (replacing the commented stubs):

```rust
// -- Search (Component 4) --

/// Full-text search over memory content via FTS5.
/// `fts_query` must be pre-sanitized by the caller (search.rs).
/// Returns memories ordered by BM25 relevance, filtered by the params,
/// always restricted to valid memories only.
fn search_fts(&self, params: &SearchFtsParams<'_>) -> Result<Vec<(Memory, f64)>, MemoryError>;

/// Vector similarity search over memory embeddings via sqlite-vec.
/// Returns memories ordered by L2 distance (ascending), filtered by the params,
/// always restricted to valid memories only.
fn search_vector(&self, params: &SearchVectorParams<'_>) -> Result<Vec<(Memory, f64)>, MemoryError>;
```

### Param structs (in search.rs, imported by db.rs)

```rust
/// Parameters for FTS5 search at the Db trait level.
#[derive(Debug, Clone)]
pub struct SearchFtsParams<'a> {
    pub actor_id: &'a str,
    pub fts_query: &'a str,              // pre-sanitized FTS5 query
    pub namespace: Option<&'a str>,
    pub namespace_prefix: Option<&'a str>,
    pub strategy: Option<&'a str>,
    pub limit: u32,
}

/// Parameters for vector similarity search at the Db trait level.
#[derive(Debug, Clone)]
pub struct SearchVectorParams<'a> {
    pub actor_id: &'a str,
    pub embedding: &'a [f32],
    pub namespace: Option<&'a str>,
    pub namespace_prefix: Option<&'a str>,
    pub strategy: Option<&'a str>,
    pub limit: u32,
}
```

This matches the existing pattern (`InsertEventParams`, `GetEventsParams`, `InsertMemoryParams`, `ListMemoriesParams`) and makes the trait extensible — new filters are struct fields, not signature changes.

### Why `Vec<(Memory, f64)>` instead of `Vec<SearchResult>`

The Db trait returns raw tuples to avoid coupling the trait to the `SearchResult` serialization type. The business logic layer in `search.rs` wraps them into `SearchResult`.

### Import in db.rs

```rust
use crate::search::{SearchFtsParams, SearchVectorParams};
```

---

## SQL Implementation

### search_fts

```sql
SELECT m.id, m.actor_id, m.namespace, m.strategy, m.content, m.metadata,
       m.source_session_id, m.is_valid, m.superseded_by, m.created_at, m.updated_at,
       -rank AS score
FROM memory_fts
JOIN memories m ON memory_fts.rowid = m.memory_rowid
WHERE memory_fts MATCH :fts_query
  AND m.actor_id = :actor_id
  AND m.is_valid = 1
  [AND m.namespace = :namespace]
  [AND m.namespace LIKE :namespace_prefix ESCAPE '\']
  [AND m.strategy = :strategy]
ORDER BY rank
LIMIT :limit
```

**Notes**:
- `rank` is FTS5's built-in BM25 ranking column (negative values, lower = more relevant)
- We negate it (`-rank`) so higher score = more relevant, consistent with vector search
- `ORDER BY rank` (ascending) puts most relevant first since rank is negative
- The `MATCH` clause drives the query — FTS5 returns candidate rowids, then we join and filter
- Dynamic WHERE clauses use positional parameters (same pattern as `list_memories`)

### search_vector

sqlite-vec provides KNN search via a special query syntax on `vec0` virtual tables:

```sql
SELECT m.id, m.actor_id, m.namespace, m.strategy, m.content, m.metadata,
       m.source_session_id, m.is_valid, m.superseded_by, m.created_at, m.updated_at,
       v.distance
FROM memory_vec v
JOIN memories m ON v.memory_id = m.id
WHERE v.embedding MATCH :query_embedding
  AND k = :k
  AND m.actor_id = :actor_id
  AND m.is_valid = 1
  [AND m.namespace = :namespace]
  [AND m.namespace LIKE :namespace_prefix ESCAPE '\']
  [AND m.strategy = :strategy]
ORDER BY v.distance
LIMIT :limit
```

**Important constraint**: sqlite-vec's `vec0` KNN query (`embedding MATCH` + `k = N`) returns the top-K nearest neighbors *before* any additional WHERE filters are applied. This means post-filters (actor_id, namespace, strategy, is_valid) reduce the result set below K.

**Mitigation**: Over-fetch by requesting more candidates from the vector index, then apply filters and take the first `limit` results.

```rust
/// Over-fetch multiplier for vector KNN queries. Since sqlite-vec applies
/// post-filters after KNN, we request more candidates to compensate for
/// rows filtered out by actor_id, namespace, strategy, and is_valid.
const VECTOR_OVERFETCH_FACTOR: u32 = 4;

/// Hard cap on KNN candidates regardless of limit * factor.
/// Bounds worst-case memory and CPU usage for large limit values.
const MAX_K_OVERFETCH: u32 = 1000;
```

The over-fetch `k` is computed as: `k = min(limit * VECTOR_OVERFETCH_FACTOR, MAX_K_OVERFETCH)`.

**Known limitation**: In stores shared by many actors, the global vector index may contain mostly other actors' embeddings. The fixed over-fetch multiplier may not retrieve enough candidates for the queried actor, resulting in fewer results than `limit`. This is a correctness tradeoff — the alternative (unbounded over-fetch or iterative retry) risks unbounded resource usage. For best results, use single-actor stores or keep the actor population small per store.

**Distance metric**: sqlite-vec `vec0` defaults to L2 (Euclidean) distance. For normalized embeddings (which most sentence transformers produce), L2 distance and cosine distance are monotonically related: `L2² = 2 - 2·cos(θ)`. We convert to a similarity score: `score = 1.0 / (1.0 + distance)` so higher = more similar, matching FTS5 convention.

**Revised SQL** (accounting for over-fetch):

```sql
-- Step 1: KNN from vec0 (no post-filters possible inside vec0 query)
SELECT v.memory_id, v.distance
FROM memory_vec v
WHERE v.embedding MATCH :query_embedding
  AND k = :k_overfetch

-- Step 2: Join + filter (in application code or as a CTE)
```

Implementation approach — two-step query:
1. Query `memory_vec` for top `k_overfetch` nearest neighbors (returns `memory_id` + `distance`), where `k_overfetch = min(limit * VECTOR_OVERFETCH_FACTOR, MAX_K_OVERFETCH)`
2. Join results against `memories` table with all filters, take first `limit`

```sql
WITH knn AS (
    SELECT memory_id, distance
    FROM memory_vec
    WHERE embedding MATCH :query_embedding AND k = :k_overfetch
)
SELECT m.id, m.actor_id, m.namespace, m.strategy, m.content, m.metadata,
       m.source_session_id, m.is_valid, m.superseded_by, m.created_at, m.updated_at,
       knn.distance
FROM knn
JOIN memories m ON knn.memory_id = m.id
WHERE m.actor_id = :actor_id
  AND m.is_valid = 1
  [AND m.namespace = :namespace]
  [AND m.namespace LIKE :namespace_prefix ESCAPE '\']
  [AND m.strategy = :strategy]
ORDER BY knn.distance ASC
LIMIT :limit
```

### Embedding serialization

sqlite-vec expects embeddings as little-endian `f32` byte blobs. The query embedding must be serialized the same way as stored embeddings:

```rust
let query_bytes: Vec<u8> = embedding.iter().flat_map(|f| f.to_le_bytes()).collect();
```

---

## Hybrid Search: Reciprocal Rank Fusion (RRF)

When both `query` and `embedding` are provided, `recall()` runs both searches and merges results using RRF.

**Why RRF**: FTS5 BM25 scores and vector distances are on incomparable scales. RRF is a simple, parameter-free rank fusion method that only uses ordinal positions, not raw scores. It's widely used in information retrieval (Cormack et al., 2009).

**Algorithm**:
1. Run `search_fts` with `limit * 2` (over-fetch for better fusion)
2. Run `search_vector` with `limit * 2`
3. For each memory appearing in either result set, compute:
   ```
   rrf_score = Σ 1 / (k + rank_i)
   ```
   where `k = 60` (standard constant) and `rank_i` is the 1-based position in each result list. If a memory appears in only one list, it gets a score from that list only.
4. Sort by `rrf_score` descending
5. Take top `limit` results

**Constant `k = 60`**: This is the standard RRF constant from the original paper. It dampens the contribution of high-ranked results, preventing a single list from dominating.

```rust
const RRF_K: f64 = 60.0;

fn reciprocal_rank_fusion(
    fts_results: &[(Memory, f64)],
    vec_results: &[(Memory, f64)],
    limit: usize,
) -> Vec<SearchResult> {
    // Build map: memory_id → rrf_score
    let mut scores: HashMap<String, (f64, Memory)> = HashMap::new();

    for (rank, (memory, _)) in fts_results.iter().enumerate() {
        let rrf = 1.0 / (RRF_K + (rank + 1) as f64);
        scores.entry(memory.id.clone())
            .and_modify(|(s, _)| *s += rrf)
            .or_insert((rrf, memory.clone()));
    }
    for (rank, (memory, _)) in vec_results.iter().enumerate() {
        let rrf = 1.0 / (RRF_K + (rank + 1) as f64);
        scores.entry(memory.id.clone())
            .and_modify(|(s, _)| *s += rrf)
            .or_insert((rrf, memory.clone()));
    }

    let mut results: Vec<SearchResult> = scores.into_values()
        .map(|(score, memory)| SearchResult { memory, score })
        .collect();
    results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    results.truncate(limit);
    results
}
```

---

## Business Logic Layer (search.rs)

```rust
/// Sanitize user input for FTS5 MATCH. Returns None if no valid tokens remain.
fn sanitize_fts_query(input: &str) -> Option<String> {
    let tokens: Vec<String> = input
        .split_whitespace()
        .map(|t| t.chars().filter(|c| c.is_alphanumeric() || *c == '_').collect::<String>())
        .filter(|t| !t.is_empty())
        .take(MAX_FTS_TOKENS)
        .map(|t| format!("\"{t}\""))
        .collect();
    if tokens.is_empty() { None } else { Some(tokens.join(" ")) }
}

/// Search memories by text, vector, or both.
pub fn recall(db: &dyn Db, params: &RecallParams<'_>) -> Result<Vec<SearchResult>, MemoryError> {
    validate_recall_params(params)?;

    let limit = params.limit.clamp(1, MAX_PAGE_LIMIT);

    match (params.query, params.embedding) {
        (None, None) => {
            Err(MemoryError::InvalidInput(
                "at least one of query or embedding must be provided".into(),
            ))
        }
        (Some(query), None) => {
            // FTS-only
            let sanitized = sanitize_fts_query(query)
                .ok_or_else(|| MemoryError::InvalidInput(
                    "query contains no searchable terms".into(),
                ))?;
            let results = db.search_fts(&SearchFtsParams {
                actor_id: params.actor_id,
                fts_query: &sanitized,
                namespace: params.namespace,
                namespace_prefix: params.namespace_prefix,
                strategy: params.strategy,
                limit,
            })?;
            Ok(results.into_iter().map(|(m, s)| SearchResult { memory: m, score: s }).collect())
        }
        (None, Some(embedding)) => {
            // Vector-only
            let results = db.search_vector(&SearchVectorParams {
                actor_id: params.actor_id,
                embedding,
                namespace: params.namespace,
                namespace_prefix: params.namespace_prefix,
                strategy: params.strategy,
                limit,
            })?;
            Ok(results.into_iter()
                .map(|(m, dist)| SearchResult { memory: m, score: 1.0 / (1.0 + dist) })
                .collect())
        }
        (Some(query), Some(embedding)) => {
            // Hybrid: RRF
            // Over-fetch for better fusion quality. Capped to bound resource usage.
            // Note: search_vector internally over-fetches by VECTOR_OVERFETCH_FACTOR,
            // so effective KNN k = min(fetch_limit * VECTOR_OVERFETCH_FACTOR, MAX_K_OVERFETCH).
            let fetch_limit = (limit * 2).min(MAX_PAGE_LIMIT);
            let sanitized = sanitize_fts_query(query);

            let fts_results = if let Some(ref q) = sanitized {
                db.search_fts(&SearchFtsParams {
                    actor_id: params.actor_id,
                    fts_query: q,
                    namespace: params.namespace,
                    namespace_prefix: params.namespace_prefix,
                    strategy: params.strategy,
                    limit: fetch_limit,
                })?
            } else {
                vec![]
            };

            let vec_results = db.search_vector(&SearchVectorParams {
                actor_id: params.actor_id,
                embedding,
                namespace: params.namespace,
                namespace_prefix: params.namespace_prefix,
                strategy: params.strategy,
                limit: fetch_limit,
            })?;

            if fts_results.is_empty() {
                // FTS had no valid tokens — fall back to vector-only
                tracing::warn!("hybrid search: FTS query sanitized to empty, falling back to vector-only");
                let results = vec_results.into_iter()
                    .take(limit as usize)
                    .map(|(m, dist)| SearchResult { memory: m, score: 1.0 / (1.0 + dist) })
                    .collect();
                return Ok(results);
            }

            Ok(reciprocal_rank_fusion(&fts_results, &vec_results, limit as usize))
        }
    }
}
```

---

## Input Validation

| Field | Rule |
|-------|------|
| `actor_id` | Non-empty, max `MAX_ACTOR_ID_LEN` |
| `query` | If present, non-empty, max `MAX_QUERY_LEN` (4 KB) |
| `embedding` | If present, length must equal `EMBEDDING_DIM` (384) |
| `namespace` | If present, non-empty, max `MAX_NAMESPACE_LEN` |
| `namespace_prefix` | If present, non-empty, max `MAX_NAMESPACE_LEN` |
| `namespace` + `namespace_prefix` | Mutually exclusive |
| `strategy` | If present, non-empty, max `MAX_STRATEGY_LEN` |
| `query` + `embedding` | At least one must be provided |
| `limit` | Clamped to `1..=MAX_PAGE_LIMIT` |

---

## Edge Cases

1. **Empty FTS query after sanitization**: If the user provides `query = "***"`, all characters are stripped. Return `InvalidInput("query contains no searchable terms")` for FTS-only. For hybrid, fall back to vector-only.

2. **No embeddings stored**: Vector search returns empty results. Not an error.

3. **Over-fetch returns fewer than limit**: Normal — there aren't enough matching memories.

4. **Duplicate memories in hybrid**: RRF naturally handles this — a memory appearing in both lists gets a higher combined score.

5. **Very long query strings**: The `query` field is validated against `MAX_QUERY_LEN` (4 KB) to prevent abuse. After sanitization, tokens are capped at `MAX_FTS_TOKENS` (64) to bound FTS5 query complexity.

6. **Unicode in FTS5**: FTS5's default tokenizer (`unicode61`) handles Unicode correctly. Our sanitization preserves alphanumeric characters from all scripts via `char::is_alphanumeric()`.

---

## Implementation Plan

| # | Task | Acceptance Criteria |
|---|------|-------------------|
| 1 | Add `SearchResult`, `RecallParams`, `SearchFtsParams`, `SearchVectorParams` to `search.rs` with constants; wire `pub mod search` in `lib.rs` | Structs compile, serde derives work, `cargo check` passes |
| 2 | Add 2 Db trait method signatures to `db.rs` | Trait compiles, `_assert_object_safe` passes |
| 3 | Implement `search_fts` for Connection | Test: returns memories matching FTS query, ordered by relevance, filtered by actor/namespace/strategy |
| 4 | Implement `search_vector` for Connection | Test: returns memories ordered by distance, filtered by actor/namespace/strategy; KNN roundtrip test passes |
| 5 | Implement `sanitize_fts_query` | Test: strips operators, quotes tokens, caps at MAX_FTS_TOKENS, handles empty input |
| 6 | Implement `recall` with FTS-only, vector-only, and hybrid paths | Tests: each path works, hybrid uses RRF, validation rejects bad input, RRF known-score test passes |
| 7 | Run `cargo check && cargo test && cargo clippy -- -D warnings` | All pass |

---

## DAG

```
[1: Data types + wire module]     [2: Db trait signatures]
              │                              │
              └──────────┬───────────────────┘
                         ▼
           ┌─────────────┼─────────────┐
           ▼             ▼             ▼
   [3: search_fts] [4: search_vector] [5: sanitize_fts_query]
           │             │             │
           └─────────────┼─────────────┘
                         ▼
                 [6: recall + RRF]
                         │
                         ▼
                 [7: Final verification]
```

Tasks 3, 4, 5 can run in parallel after tasks 1 and 2.

---

## Sub-Agent Instructions

### Pre-conditions
- `cargo check` and `cargo test` pass on current main
- Read: `src/db.rs`, `src/memories.rs`, `src/error.rs`, `src/lib.rs`, `design/search.md`

### Step 1: Create `src/search.rs` with data types

Create `src/search.rs` with:
- `use std::collections::HashMap;`
- `use serde::{Deserialize, Serialize};`
- `use crate::db::{Db, EMBEDDING_DIM};`
- `use crate::error::MemoryError;`
- `use crate::memories::Memory;`
- `use crate::events::MAX_ACTOR_ID_LEN;`
- `use crate::memories::{MAX_NAMESPACE_LEN, MAX_STRATEGY_LEN};`
- `use crate::events::MAX_PAGE_LIMIT;`
- Constants: `const RRF_K: f64 = 60.0;`, `const MAX_FTS_TOKENS: usize = 64;`, `const VECTOR_OVERFETCH_FACTOR: u32 = 4;`, `const MAX_K_OVERFETCH: u32 = 1000;`, `pub const MAX_QUERY_LEN: usize = 4096;`
- Struct `SearchResult` with `#[serde(flatten)] pub memory: Memory` and `pub score: f64`
- Struct `RecallParams<'a>` with fields: `actor_id`, `query`, `embedding`, `namespace`, `namespace_prefix`, `strategy`, `limit`
- Struct `SearchFtsParams<'a>` with fields: `actor_id`, `fts_query`, `namespace`, `namespace_prefix`, `strategy`, `limit`
- Struct `SearchVectorParams<'a>` with fields: `actor_id`, `embedding`, `namespace`, `namespace_prefix`, `strategy`, `limit`

Also add `pub mod search;` to `src/lib.rs` immediately (enables incremental compilation).

### Step 2: Add Db trait methods

In `src/db.rs`:
- Add `use crate::search::{SearchFtsParams, SearchVectorParams};`
- Replace the commented `// -- Search (Component 4) --` stubs with the 2 method signatures:
  - `search_fts(&self, params: &SearchFtsParams<'_>) -> Result<Vec<(Memory, f64)>, MemoryError>`
  - `search_vector(&self, params: &SearchVectorParams<'_>) -> Result<Vec<(Memory, f64)>, MemoryError>`
- Verify `_assert_object_safe` still compiles

### Step 3: Implement `search_fts` for Connection

- Accept `&SearchFtsParams<'_>`
- Build dynamic SQL with the FTS5 join pattern:
  ```sql
  SELECT m.id, m.actor_id, m.namespace, m.strategy, m.content, m.metadata,
         m.source_session_id, m.is_valid, m.superseded_by, m.created_at, m.updated_at,
         -rank AS score
  FROM memory_fts
  JOIN memories m ON memory_fts.rowid = m.memory_rowid
  WHERE memory_fts MATCH ?1
    AND m.actor_id = ?2
    AND m.is_valid = 1
  ```
- Add optional filters for namespace, namespace_prefix, strategy using positional params
- **namespace_prefix**: Use `format!("{}%", escape_like(prefix))` before binding, matching the pattern in `list_memories`
- `ORDER BY rank LIMIT ?N`
- Use `row_to_memory` for the first 11 columns, then `row.get::<_, f64>(11)` for score
- Return `Vec<(Memory, f64)>`

### Step 4: Implement `search_vector` for Connection

- Accept `&SearchVectorParams<'_>`
- Validate embedding dimension at runtime (defense-in-depth): `if params.embedding.len() != EMBEDDING_DIM as usize { return Err(MemoryError::InvalidInput(...)) }`
- Serialize query embedding as little-endian f32 bytes: `params.embedding.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>()`
- Compute `k_overfetch = (params.limit * VECTOR_OVERFETCH_FACTOR).min(MAX_K_OVERFETCH)`
- Use CTE approach with over-fetch:
  ```sql
  WITH knn AS (
      SELECT memory_id, distance
      FROM memory_vec
      WHERE embedding MATCH ?1 AND k = ?2
  )
  SELECT m.id, m.actor_id, m.namespace, m.strategy, m.content, m.metadata,
         m.source_session_id, m.is_valid, m.superseded_by, m.created_at, m.updated_at,
         knn.distance
  FROM knn
  JOIN memories m ON knn.memory_id = m.id
  WHERE m.actor_id = ?3
    AND m.is_valid = 1
  ```
- Add optional filters for namespace, namespace_prefix (use `format!("{}%", escape_like(prefix))`), strategy
- `ORDER BY knn.distance ASC LIMIT ?N`
- Return `Vec<(Memory, f64)>` where f64 is the raw distance

### Step 5: Implement `sanitize_fts_query`

In `search.rs`:
- `fn sanitize_fts_query(input: &str) -> Option<String>`
- Split on whitespace, filter each token to `is_alphanumeric() || '_'`, discard empty, `.take(MAX_FTS_TOKENS)`, wrap in `"..."`, join with space
- Return `None` if no tokens survive

### Step 6: Implement `recall` and RRF

In `search.rs`:
- `fn validate_recall_params(params: &RecallParams<'_>) -> Result<(), MemoryError>` — validates all fields per the table above, including `MAX_QUERY_LEN` for query
- `fn reciprocal_rank_fusion(fts: &[(Memory, f64)], vec: &[(Memory, f64)], limit: usize) -> Vec<SearchResult>` — as described in the design
- `pub fn recall(db: &dyn Db, params: &RecallParams<'_>) -> Result<Vec<SearchResult>, MemoryError>` — match on (query, embedding) to dispatch FTS-only, vector-only, or hybrid. Use `SearchFtsParams`/`SearchVectorParams` structs for Db calls. Log `tracing::warn!` when hybrid falls back to vector-only due to empty sanitized query.

### Step 7: Final verification

Module already wired in Step 1. Run:
```bash
cargo check && cargo test && cargo clippy -- -D warnings
```

### Test expectations

In `db.rs` tests (Db trait impl):
- `test_search_fts_basic` — insert 3 memories, search for a term in one, verify it's returned with a positive score
- `test_search_fts_actor_scoping` — memories from other actors not returned
- `test_search_fts_valid_only` — invalidated memories not returned
- `test_search_fts_namespace_filter` — namespace filter works
- `test_search_fts_no_match` — returns empty vec, not error
- `test_search_vector_basic` — insert memories with embeddings, search with similar vector, verify ordering by distance
- `test_search_vector_actor_scoping` — memories from other actors not returned
- `test_search_vector_no_embeddings` — returns empty vec when no embeddings stored
- `test_search_vector_knn_roundtrip` — insert embedding, query with same vector, verify it's returned (validates sqlite-vec KNN blob binding works end-to-end)

In `search.rs` tests:
- `test_sanitize_strips_operators` — `"Rust OR DROP"` → `"Rust" "OR" "DROP"` (case preserved by sanitizer, lowered by FTS5 at match time)
- `test_sanitize_empty_input` — `""` → `None`
- `test_sanitize_only_special_chars` — `"***"` → `None`
- `test_sanitize_preserves_unicode` — `"café naïve"` → `"café" "naïve"`
- `test_sanitize_caps_token_count` — input with 100 words → only 64 tokens in output
- `test_recall_requires_query_or_embedding` — returns InvalidInput
- `test_recall_rejects_long_query` — query > MAX_QUERY_LEN returns InvalidInput
- `test_recall_fts_only` — works with query only
- `test_recall_vector_only` — works with embedding only
- `test_recall_hybrid_rrf` — both provided, results merged, memories in both lists score higher
- `test_recall_validates_inputs` — rejects empty actor_id, wrong embedding dim, both namespace + namespace_prefix
- `test_rrf_known_scores` — unit test `reciprocal_rank_fusion` directly: FTS returns [A, B], vector returns [B, C] → verify A gets `1/(60+1)`, B gets `1/(60+1) + 1/(60+2)`, C gets `1/(60+2)`, and B ranks first
