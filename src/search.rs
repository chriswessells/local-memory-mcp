use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::db::{Db, EMBEDDING_DIM};
use crate::error::MemoryError;
use crate::events::{MAX_ACTOR_ID_LEN, MAX_PAGE_LIMIT};
use crate::memories::{Memory, MAX_NAMESPACE_LEN, MAX_STRATEGY_LEN};

// --- Constants ---

/// Maximum number of tokens in a sanitized FTS5 query.
/// Bounds the number of posting list intersections FTS5 must perform.
const MAX_FTS_TOKENS: usize = 64;

/// Maximum length of a search query string (bytes).
/// Search queries should be short — not document-sized.
pub const MAX_QUERY_LEN: usize = 4096;

/// Over-fetch multiplier for vector KNN queries. Since sqlite-vec applies
/// post-filters after KNN, we request more candidates to compensate for
/// rows filtered out by actor_id, namespace, strategy, and is_valid.
pub(crate) const VECTOR_OVERFETCH_FACTOR: u32 = 4;

/// Hard cap on KNN candidates regardless of limit * factor.
/// k = min(limit * VECTOR_OVERFETCH_FACTOR, MAX_K_OVERFETCH).
/// Effective overfetch factor drops below VECTOR_OVERFETCH_FACTOR when limit > 250.
pub(crate) const MAX_K_OVERFETCH: u32 = 1000;

/// RRF constant from Cormack et al. (2009). Dampens high-rank contributions.
const RRF_K: f64 = 60.0;

// --- Data types ---

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

/// Parameters for the recall function (business logic layer).
#[derive(Debug, Clone)]
pub struct RecallParams<'a> {
    pub actor_id: &'a str,
    pub query: Option<&'a str>,
    pub embedding: Option<&'a [f32]>,
    pub namespace: Option<&'a str>,
    pub namespace_prefix: Option<&'a str>,
    pub strategy: Option<&'a str>,
    pub limit: u32,
}

/// Parameters for FTS5 search at the Db trait level.
/// `fts_query` must be pre-sanitized by the caller (search.rs).
#[derive(Debug, Clone)]
pub struct SearchFtsParams<'a> {
    pub actor_id: &'a str,
    pub fts_query: &'a str,
    pub namespace: Option<&'a str>,
    pub namespace_prefix: Option<&'a str>,
    pub strategy: Option<&'a str>,
    pub limit: u32,
}

/// Parameters for vector similarity search at the Db trait level.
/// Returns raw L2 distances (lower = closer); callers must convert to similarity if needed.
#[derive(Debug, Clone)]
pub struct SearchVectorParams<'a> {
    pub actor_id: &'a str,
    pub embedding: &'a [f32],
    pub namespace: Option<&'a str>,
    pub namespace_prefix: Option<&'a str>,
    pub strategy: Option<&'a str>,
    pub limit: u32,
}

// --- Validation ---

fn validate_non_empty(value: &str, field: &str) -> Result<(), MemoryError> {
    if value.is_empty() {
        return Err(MemoryError::InvalidInput(format!(
            "{field} must not be empty"
        )));
    }
    Ok(())
}

fn validate_max_len(value: &str, max: usize, field: &str) -> Result<(), MemoryError> {
    if value.len() > max {
        return Err(MemoryError::InvalidInput(format!(
            "{field} exceeds maximum length of {max} bytes"
        )));
    }
    Ok(())
}

fn validate_recall_params(params: &RecallParams<'_>) -> Result<(), MemoryError> {
    validate_non_empty(params.actor_id, "actor_id")?;
    validate_max_len(params.actor_id, MAX_ACTOR_ID_LEN, "actor_id")?;

    if params.query.is_none() && params.embedding.is_none() {
        return Err(MemoryError::InvalidInput(
            "at least one of query or embedding must be provided".into(),
        ));
    }

    if let Some(q) = params.query {
        validate_non_empty(q, "query")?;
        validate_max_len(q, MAX_QUERY_LEN, "query")?;
    }

    if let Some(emb) = params.embedding {
        if emb.len() != EMBEDDING_DIM as usize {
            return Err(MemoryError::InvalidInput(format!(
                "embedding must have exactly {EMBEDDING_DIM} dimensions"
            )));
        }
        if emb.iter().any(|v| !v.is_finite()) {
            return Err(MemoryError::InvalidInput(
                "embedding contains NaN or infinity".into(),
            ));
        }
    }

    if params.namespace.is_some() && params.namespace_prefix.is_some() {
        return Err(MemoryError::InvalidInput(
            "namespace and namespace_prefix are mutually exclusive".into(),
        ));
    }
    if let Some(ns) = params.namespace {
        validate_non_empty(ns, "namespace")?;
        validate_max_len(ns, MAX_NAMESPACE_LEN, "namespace")?;
    }
    if let Some(prefix) = params.namespace_prefix {
        validate_non_empty(prefix, "namespace_prefix")?;
        validate_max_len(prefix, MAX_NAMESPACE_LEN, "namespace_prefix")?;
    }
    if let Some(s) = params.strategy {
        validate_non_empty(s, "strategy")?;
        validate_max_len(s, MAX_STRATEGY_LEN, "strategy")?;
    }

    Ok(())
}

// --- FTS5 sanitization ---

/// Sanitize user input for FTS5 MATCH. Returns None if no valid tokens remain.
/// Strips all FTS5 special characters, caps at MAX_FTS_TOKENS, wraps each token in quotes.
/// Hyphen stripping is intentional — aligned with FTS5's unicode61 tokenizer behavior.
fn sanitize_fts_query(input: &str) -> Option<String> {
    let tokens: Vec<String> = input
        .split_whitespace()
        .map(|t| {
            t.chars()
                .filter(|c| c.is_alphanumeric() || *c == '_')
                .collect::<String>()
        })
        .filter(|t| !t.is_empty())
        .take(MAX_FTS_TOKENS)
        .map(|t| format!("\"{t}\""))
        .collect();
    if tokens.is_empty() {
        None
    } else {
        Some(tokens.join(" "))
    }
}

// --- RRF ---

fn reciprocal_rank_fusion(
    fts_results: &[(Memory, f64)],
    vec_results: &[(Memory, f64)],
    limit: usize,
) -> Vec<SearchResult> {
    let mut scores: HashMap<String, (f64, Memory)> = HashMap::new();

    for (rank, (memory, _)) in fts_results.iter().enumerate() {
        let rrf = 1.0 / (RRF_K + (rank + 1) as f64);
        scores
            .entry(memory.id.clone())
            .and_modify(|(s, _)| *s += rrf)
            // Memory content is identical from both sources; keep whichever is inserted first.
            .or_insert_with(|| (rrf, memory.clone()));
    }
    for (rank, (memory, _)) in vec_results.iter().enumerate() {
        let rrf = 1.0 / (RRF_K + (rank + 1) as f64);
        debug_assert!(rrf.is_finite());
        scores
            .entry(memory.id.clone())
            .and_modify(|(s, _)| *s += rrf)
            .or_insert_with(|| (rrf, memory.clone()));
    }

    let mut results: Vec<SearchResult> = scores
        .into_values()
        .map(|(score, memory)| SearchResult { memory, score })
        .collect();
    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results.truncate(limit);
    results
}

// --- Business logic ---

/// Search memories by text, vector, or both (hybrid RRF).
pub fn recall(db: &dyn Db, params: &RecallParams<'_>) -> Result<Vec<SearchResult>, MemoryError> {
    validate_recall_params(params)?;
    let limit = params.limit.clamp(1, MAX_PAGE_LIMIT);

    match (params.query, params.embedding) {
        (None, None) => unreachable!("validated above"),
        (Some(query), None) => {
            let sanitized = sanitize_fts_query(query).ok_or_else(|| {
                MemoryError::InvalidInput("query contains no searchable terms".into())
            })?;
            let results = db.search_fts(&SearchFtsParams {
                actor_id: params.actor_id,
                fts_query: &sanitized,
                namespace: params.namespace,
                namespace_prefix: params.namespace_prefix,
                strategy: params.strategy,
                limit,
            })?;
            Ok(results
                .into_iter()
                .map(|(m, s)| SearchResult {
                    memory: m,
                    score: s,
                })
                .collect())
        }
        (None, Some(embedding)) => {
            let results = db.search_vector(&SearchVectorParams {
                actor_id: params.actor_id,
                embedding,
                namespace: params.namespace,
                namespace_prefix: params.namespace_prefix,
                strategy: params.strategy,
                limit,
            })?;
            Ok(results
                .into_iter()
                .map(|(m, dist)| SearchResult {
                    memory: m,
                    score: 1.0 / (1.0 + dist.max(0.0)),
                })
                .collect())
        }
        (Some(query), Some(embedding)) => {
            // Hybrid: RRF. Over-fetch for better fusion quality, capped to bound resources.
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
                tracing::warn!(
                    "hybrid search: FTS query sanitized to empty, falling back to vector-only"
                );
                return Ok(vec_results
                    .into_iter()
                    .take(limit as usize)
                    .map(|(m, dist)| SearchResult {
                        memory: m,
                        score: 1.0 / (1.0 + dist.max(0.0)),
                    })
                    .collect());
            }

            Ok(reciprocal_rank_fusion(
                &fts_results,
                &vec_results,
                limit as usize,
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::memories::InsertMemoryParams;
    use tempfile::TempDir;

    fn open_db() -> (TempDir, rusqlite::Connection) {
        let dir = TempDir::new().unwrap();
        let conn = db::open(&dir.path().join("test.db")).unwrap();
        (dir, conn)
    }

    fn mem_params<'a>(actor: &'a str, content: &'a str) -> InsertMemoryParams<'a> {
        InsertMemoryParams {
            actor_id: actor,
            content,
            strategy: "semantic",
            namespace: None,
            metadata: None,
            source_session_id: None,
            embedding: None,
        }
    }

    // --- sanitize_fts_query tests ---

    #[test]
    fn test_sanitize_strips_operators() {
        let result = sanitize_fts_query("Rust OR DROP").unwrap();
        assert_eq!(result, "\"Rust\" \"OR\" \"DROP\"");
    }

    #[test]
    fn test_sanitize_empty_input() {
        assert!(sanitize_fts_query("").is_none());
    }

    #[test]
    fn test_sanitize_only_special_chars() {
        assert!(sanitize_fts_query("*** *** ***").is_none());
    }

    #[test]
    fn test_sanitize_preserves_unicode() {
        let result = sanitize_fts_query("café naïve").unwrap();
        assert_eq!(result, "\"café\" \"naïve\"");
    }

    #[test]
    fn test_sanitize_caps_token_count() {
        let input = (0..100)
            .map(|i| format!("word{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        let result = sanitize_fts_query(&input).unwrap();
        let count = result.split_whitespace().count();
        assert_eq!(count, MAX_FTS_TOKENS);
    }

    // --- RRF unit test with known scores ---

    #[test]
    fn test_rrf_known_scores() {
        // FTS returns [A, B], vector returns [B, C]
        // A: 1/(60+1) from FTS only
        // B: 1/(60+1) + 1/(60+2) from both
        // C: 1/(60+2) from vector only
        // Expected order: B > A > C
        let make_mem = |id: &str| Memory {
            id: id.to_string(),
            actor_id: "a".to_string(),
            namespace: "default".to_string(),
            strategy: "semantic".to_string(),
            content: id.to_string(),
            metadata: None,
            source_session_id: None,
            is_valid: true,
            superseded_by: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        };

        let fts = vec![(make_mem("A"), 1.0), (make_mem("B"), 0.5)];
        let vec = vec![(make_mem("B"), 0.1), (make_mem("C"), 0.2)];

        let results = reciprocal_rank_fusion(&fts, &vec, 10);
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].memory.id, "B");
        assert_eq!(results[1].memory.id, "A");
        assert_eq!(results[2].memory.id, "C");

        let score_a = 1.0 / (RRF_K + 1.0);
        let score_b = 1.0 / (RRF_K + 1.0) + 1.0 / (RRF_K + 2.0);
        let score_c = 1.0 / (RRF_K + 2.0);
        assert!((results[0].score - score_b).abs() < 1e-10);
        assert!((results[1].score - score_a).abs() < 1e-10);
        assert!((results[2].score - score_c).abs() < 1e-10);
    }

    #[test]
    fn test_rrf_disjoint_lists() {
        let make_mem = |id: &str| Memory {
            id: id.to_string(),
            actor_id: "a".to_string(),
            namespace: "default".to_string(),
            strategy: "semantic".to_string(),
            content: id.to_string(),
            metadata: None,
            source_session_id: None,
            is_valid: true,
            superseded_by: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let fts = vec![(make_mem("A"), 1.0), (make_mem("B"), 0.5)];
        let vec = vec![(make_mem("C"), 0.1), (make_mem("D"), 0.2)];
        let results = reciprocal_rank_fusion(&fts, &vec, 10);
        assert_eq!(results.len(), 4);
        // All get single-list scores; A and C (rank 1) should tie, B and D (rank 2) should tie
        let score_rank1 = 1.0 / (RRF_K + 1.0);
        let score_rank2 = 1.0 / (RRF_K + 2.0);
        for r in &results {
            assert!((r.score - score_rank1).abs() < 1e-10 || (r.score - score_rank2).abs() < 1e-10);
        }
    }

    // --- validate_recall_params tests ---

    #[test]
    fn test_recall_requires_query_or_embedding() {
        let params = RecallParams {
            actor_id: "a",
            query: None,
            embedding: None,
            namespace: None,
            namespace_prefix: None,
            strategy: None,
            limit: 10,
        };
        assert!(matches!(
            validate_recall_params(&params),
            Err(MemoryError::InvalidInput(_))
        ));
    }

    #[test]
    fn test_recall_rejects_long_query() {
        let long = "a ".repeat(MAX_QUERY_LEN);
        let params = RecallParams {
            actor_id: "a",
            query: Some(&long),
            embedding: None,
            namespace: None,
            namespace_prefix: None,
            strategy: None,
            limit: 10,
        };
        assert!(matches!(
            validate_recall_params(&params),
            Err(MemoryError::InvalidInput(_))
        ));
    }

    #[test]
    fn test_recall_validates_inputs() {
        // Empty actor_id
        let params = RecallParams {
            actor_id: "",
            query: Some("hello"),
            embedding: None,
            namespace: None,
            namespace_prefix: None,
            strategy: None,
            limit: 10,
        };
        assert!(matches!(
            validate_recall_params(&params),
            Err(MemoryError::InvalidInput(_))
        ));

        // Wrong embedding dim
        let emb = vec![0.1f32; 10];
        let params = RecallParams {
            actor_id: "a",
            query: None,
            embedding: Some(&emb),
            namespace: None,
            namespace_prefix: None,
            strategy: None,
            limit: 10,
        };
        assert!(matches!(
            validate_recall_params(&params),
            Err(MemoryError::InvalidInput(_))
        ));

        // Both namespace + namespace_prefix
        let params = RecallParams {
            actor_id: "a",
            query: Some("hello"),
            embedding: None,
            namespace: Some("ns"),
            namespace_prefix: Some("prefix"),
            strategy: None,
            limit: 10,
        };
        assert!(matches!(
            validate_recall_params(&params),
            Err(MemoryError::InvalidInput(_))
        ));
    }

    // --- recall integration tests ---

    #[test]
    fn test_recall_fts_only() {
        let (_dir, conn) = open_db();
        conn.insert_memory(&mem_params("a1", "Rust programming language"))
            .unwrap();
        conn.insert_memory(&mem_params("a1", "Python scripting"))
            .unwrap();

        let params = RecallParams {
            actor_id: "a1",
            query: Some("Rust"),
            embedding: None,
            namespace: None,
            namespace_prefix: None,
            strategy: None,
            limit: 10,
        };
        let results = recall(&conn, &params).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].memory.content.contains("Rust"));
        assert!(results[0].score > 0.0);
    }

    #[test]
    fn test_recall_vector_only() {
        let (_dir, conn) = open_db();
        let mut emb1 = vec![0.0f32; 384];
        emb1[0] = 1.0;
        let mut emb2 = vec![0.0f32; 384];
        emb2[1] = 1.0;

        conn.insert_memory(&InsertMemoryParams {
            embedding: Some(&emb1),
            ..mem_params("a1", "memory one")
        })
        .unwrap();
        conn.insert_memory(&InsertMemoryParams {
            embedding: Some(&emb2),
            ..mem_params("a1", "memory two")
        })
        .unwrap();

        let params = RecallParams {
            actor_id: "a1",
            query: None,
            embedding: Some(&emb1),
            namespace: None,
            namespace_prefix: None,
            strategy: None,
            limit: 10,
        };
        let results = recall(&conn, &params).unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].memory.content, "memory one");
        assert!(results[0].score > 0.0);
    }

    #[test]
    fn test_recall_hybrid_rrf() {
        let (_dir, conn) = open_db();
        let mut emb = vec![0.0f32; 384];
        emb[0] = 1.0;

        // This memory matches both FTS ("unique") and vector (same embedding)
        conn.insert_memory(&InsertMemoryParams {
            embedding: Some(&emb),
            ..mem_params("a1", "unique term memory")
        })
        .unwrap();
        // This memory matches only FTS
        conn.insert_memory(&mem_params("a1", "unique term only text"))
            .unwrap();
        // This memory matches only vector
        let mut emb2 = vec![0.0f32; 384];
        emb2[0] = 0.99;
        conn.insert_memory(&InsertMemoryParams {
            embedding: Some(&emb2),
            ..mem_params("a1", "vector only memory")
        })
        .unwrap();

        let params = RecallParams {
            actor_id: "a1",
            query: Some("unique"),
            embedding: Some(&emb),
            namespace: None,
            namespace_prefix: None,
            strategy: None,
            limit: 10,
        };
        let results = recall(&conn, &params).unwrap();
        assert!(!results.is_empty());
        // The memory matching both should rank highest
        assert_eq!(results[0].memory.content, "unique term memory");
    }
}
