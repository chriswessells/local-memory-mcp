use serde::{Deserialize, Serialize};

use crate::db::Db;
use crate::error::MemoryError;
use crate::events::MAX_PAGE_LIMIT;
use crate::memories::MAX_NAMESPACE_LEN;

pub const MAX_DESCRIPTION_LEN: usize = 1_024;
pub const NAMESPACE_DELETE_CHUNK_SIZE: usize = 500;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Namespace {
    pub name: String,
    pub description: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct CreateNamespaceParams<'a> {
    pub name: &'a str,
    pub description: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct ListNamespacesParams<'a> {
    pub prefix: Option<&'a str>,
    pub limit: u32,
    pub offset: u32,
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

/// Validate a namespace name. Exported so memories.rs can call it when validating
/// the namespace field on memory inserts, ensuring both paths enforce the same rules.
pub fn validate_namespace_name(name: &str) -> Result<(), MemoryError> {
    if name.is_empty() {
        return Err(MemoryError::InvalidInput(
            "namespace name must not be empty".into(),
        ));
    }
    if name.len() > MAX_NAMESPACE_LEN {
        return Err(MemoryError::InvalidInput(format!(
            "namespace name exceeds maximum length of {MAX_NAMESPACE_LEN} bytes (UTF-8)"
        )));
    }
    // Reject null bytes and ASCII control characters (blocks log/terminal injection).
    // We do NOT enforce a strict charset allowlist to stay compatible with AgentCore-style
    // paths (which allow '/', '-', '.', ':', '@', emoji, etc.). Control chars are the
    // practical security boundary; printable Unicode is permitted intentionally.
    if name.bytes().any(|b| b == 0x00 || b < 0x20 || b == 0x7F) {
        return Err(MemoryError::InvalidInput(
            "namespace name must not contain control characters or null bytes".into(),
        ));
    }
    Ok(())
}

fn validate_description(description: &str) -> Result<(), MemoryError> {
    if description.len() > MAX_DESCRIPTION_LEN {
        return Err(MemoryError::InvalidInput(format!(
            "description exceeds maximum length of {MAX_DESCRIPTION_LEN} bytes"
        )));
    }
    Ok(())
}

fn validate_prefix(prefix: &str) -> Result<(), MemoryError> {
    if prefix.is_empty() {
        return Err(MemoryError::InvalidInput("prefix must not be empty".into()));
    }
    if prefix.len() > MAX_NAMESPACE_LEN {
        return Err(MemoryError::InvalidInput(format!(
            "prefix exceeds maximum length of {MAX_NAMESPACE_LEN} bytes"
        )));
    }
    Ok(())
}

// --- Business logic ---

pub fn create_namespace(
    db: &dyn Db,
    params: &CreateNamespaceParams<'_>,
) -> Result<Namespace, MemoryError> {
    validate_namespace_name(params.name)?;
    if let Some(desc) = params.description {
        validate_description(desc)?;
    }
    db.create_namespace(params.name, params.description)
}

pub fn list_namespaces(
    db: &dyn Db,
    params: &ListNamespacesParams<'_>,
) -> Result<Vec<Namespace>, MemoryError> {
    if let Some(prefix) = params.prefix {
        validate_prefix(prefix)?;
    }
    let clamped = ListNamespacesParams {
        limit: params.limit.clamp(1, MAX_PAGE_LIMIT),
        ..*params
    };
    db.list_namespaces(&clamped)
}

pub fn delete_namespace(db: &dyn Db, actor_id: &str, name: &str) -> Result<u64, MemoryError> {
    validate_non_empty(actor_id, "actor_id")?;
    validate_namespace_name(name)?;
    db.delete_namespace(actor_id, name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use tempfile::TempDir;

    fn open_db() -> (TempDir, rusqlite::Connection) {
        let dir = TempDir::new().unwrap();
        let conn = db::open(&dir.path().join("test.db")).unwrap();
        (dir, conn)
    }

    // --- Validation tests ---

    #[test]
    fn test_validate_name_empty() {
        assert!(matches!(
            validate_namespace_name(""),
            Err(MemoryError::InvalidInput(_))
        ));
    }

    #[test]
    fn test_validate_name_too_long() {
        let long = "a".repeat(MAX_NAMESPACE_LEN + 1);
        assert!(matches!(
            validate_namespace_name(&long),
            Err(MemoryError::InvalidInput(_))
        ));
    }

    #[test]
    fn test_validate_name_null_byte() {
        assert!(matches!(
            validate_namespace_name("foo\0bar"),
            Err(MemoryError::InvalidInput(_))
        ));
    }

    #[test]
    fn test_validate_name_control_char() {
        for ch in ['\n', '\t', '\x1b'] {
            let s = format!("foo{ch}bar");
            assert!(
                matches!(validate_namespace_name(&s), Err(MemoryError::InvalidInput(_))),
                "expected error for control char {:?}",
                ch
            );
        }
    }

    #[test]
    fn test_validate_name_del_char() {
        assert!(matches!(
            validate_namespace_name("foo\x7fbar"),
            Err(MemoryError::InvalidInput(_))
        ));
    }

    #[test]
    fn test_validate_description_too_long() {
        let long = "x".repeat(MAX_DESCRIPTION_LEN + 1);
        assert!(matches!(
            validate_description(&long),
            Err(MemoryError::InvalidInput(_))
        ));
    }

    #[test]
    fn test_validate_prefix_empty() {
        assert!(matches!(
            validate_prefix(""),
            Err(MemoryError::InvalidInput(_))
        ));
    }

    // --- Db-level tests ---

    #[test]
    fn test_create_namespace_basic() {
        let (_dir, conn) = open_db();
        let ns = conn
            .create_namespace("/user/alice", Some("Alice's namespace"))
            .unwrap();
        assert_eq!(ns.name, "/user/alice");
        assert_eq!(ns.description.as_deref(), Some("Alice's namespace"));
        assert!(!ns.created_at.is_empty());
    }

    #[test]
    fn test_create_namespace_idempotent() {
        let (_dir, conn) = open_db();
        let ns1 = conn
            .create_namespace("/user/alice", Some("first"))
            .unwrap();
        let ns2 = conn
            .create_namespace("/user/alice", Some("second"))
            .unwrap();
        // Description unchanged on second call
        assert_eq!(ns2.description.as_deref(), Some("first"));
        assert_eq!(ns1.name, ns2.name);
        assert_eq!(ns1.created_at, ns2.created_at);
    }

    #[test]
    fn test_create_namespace_no_description() {
        let (_dir, conn) = open_db();
        let ns = conn.create_namespace("/ns/no-desc", None).unwrap();
        assert!(ns.description.is_none());
    }

    #[test]
    fn test_list_namespaces_empty() {
        let (_dir, conn) = open_db();
        let params = ListNamespacesParams {
            prefix: None,
            limit: 100,
            offset: 0,
        };
        let result = conn.list_namespaces(&params).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_list_namespaces_ordered() {
        let (_dir, conn) = open_db();
        for name in ["/b", "/a", "/c"] {
            conn.create_namespace(name, None).unwrap();
        }
        let params = ListNamespacesParams {
            prefix: None,
            limit: 100,
            offset: 0,
        };
        let result = conn.list_namespaces(&params).unwrap();
        let names: Vec<&str> = result.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, vec!["/a", "/b", "/c"]);
    }

    #[test]
    fn test_list_namespaces_prefix() {
        let (_dir, conn) = open_db();
        for name in ["/user/alice", "/user/bob", "/system/logs"] {
            conn.create_namespace(name, None).unwrap();
        }
        let params = ListNamespacesParams {
            prefix: Some("/user"),
            limit: 100,
            offset: 0,
        };
        let result = conn.list_namespaces(&params).unwrap();
        let names: Vec<&str> = result.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, vec!["/user/alice", "/user/bob"]);
    }

    #[test]
    fn test_list_namespaces_prefix_escaping() {
        let (_dir, conn) = open_db();
        conn.create_namespace("/a%b/x", None).unwrap();
        conn.create_namespace("/a_b/y", None).unwrap();
        conn.create_namespace("/acb/z", None).unwrap();

        let params = ListNamespacesParams {
            prefix: Some("/a%b"),
            limit: 100,
            offset: 0,
        };
        let result = conn.list_namespaces(&params).unwrap();
        let names: Vec<&str> = result.iter().map(|n| n.name.as_str()).collect();
        // Only exact prefix "/a%b" matches, not "/acb"
        assert_eq!(names, vec!["/a%b/x"]);
    }

    #[test]
    fn test_list_namespaces_pagination() {
        let (_dir, conn) = open_db();
        for name in ["/a", "/b", "/c", "/d"] {
            conn.create_namespace(name, None).unwrap();
        }
        let params = ListNamespacesParams {
            prefix: None,
            limit: 2,
            offset: 1,
        };
        let result = conn.list_namespaces(&params).unwrap();
        let names: Vec<&str> = result.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, vec!["/b", "/c"]);
    }

    #[test]
    fn test_delete_namespace_not_found() {
        let (_dir, conn) = open_db();
        let result = conn.delete_namespace("actor1", "/nonexistent");
        assert!(matches!(result, Err(MemoryError::NotFound(_))));
    }

    #[test]
    fn test_delete_namespace_no_memories() {
        let (_dir, conn) = open_db();
        conn.create_namespace("/empty", None).unwrap();
        let deleted = conn.delete_namespace("actor1", "/empty").unwrap();
        assert_eq!(deleted, 0);
        // Registry entry removed
        let params = ListNamespacesParams {
            prefix: None,
            limit: 100,
            offset: 0,
        };
        let list = conn.list_namespaces(&params).unwrap();
        assert!(list.is_empty());
    }

    #[test]
    fn test_delete_namespace_with_memories() {
        use crate::memories::InsertMemoryParams;
        let (_dir, conn) = open_db();
        conn.create_namespace("/ns/test", None).unwrap();

        for i in 0..3 {
            let p = InsertMemoryParams {
                actor_id: "actor1",
                content: &format!("memory {i}"),
                strategy: "raw",
                namespace: Some("/ns/test"),
                metadata: None,
                source_session_id: None,
                embedding: None,
            };
            conn.insert_memory(&p).unwrap();
        }

        let deleted = conn.delete_namespace("actor1", "/ns/test").unwrap();
        assert_eq!(deleted, 3);

        // Memories gone
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE namespace = '/ns/test' AND actor_id = 'actor1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_delete_namespace_actor_scoped() {
        use crate::memories::InsertMemoryParams;
        let (_dir, conn) = open_db();
        conn.create_namespace("/shared/ns", None).unwrap();

        for actor in ["actor1", "actor2"] {
            let p = InsertMemoryParams {
                actor_id: actor,
                content: "memory",
                strategy: "raw",
                namespace: Some("/shared/ns"),
                metadata: None,
                source_session_id: None,
                embedding: None,
            };
            conn.insert_memory(&p).unwrap();
        }

        let deleted = conn.delete_namespace("actor1", "/shared/ns").unwrap();
        assert_eq!(deleted, 1);

        // actor2's memory still present
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE namespace = '/shared/ns' AND actor_id = 'actor2'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_delete_namespace_exact_match_only() {
        use crate::memories::InsertMemoryParams;
        let (_dir, conn) = open_db();
        conn.create_namespace("/a", None).unwrap();
        conn.create_namespace("/a/b", None).unwrap();

        let p = InsertMemoryParams {
            actor_id: "actor1",
            content: "sub memory",
            strategy: "raw",
            namespace: Some("/a/b"),
            metadata: None,
            source_session_id: None,
            embedding: None,
        };
        conn.insert_memory(&p).unwrap();

        // Delete "/a" — should not touch "/a/b" memories
        conn.delete_namespace("actor1", "/a").unwrap();

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE namespace = '/a/b'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_delete_namespace_cascades_edges() {
        use crate::graph::{add_edge, InsertEdgeParams};
        use crate::memories::InsertMemoryParams;
        let (_dir, conn) = open_db();
        conn.create_namespace("/ns/edges", None).unwrap();

        let m1 = conn
            .insert_memory(&InsertMemoryParams {
                actor_id: "actor1",
                content: "from",
                strategy: "raw",
                namespace: Some("/ns/edges"),
                metadata: None,
                source_session_id: None,
                embedding: None,
            })
            .unwrap();
        let m2 = conn
            .insert_memory(&InsertMemoryParams {
                actor_id: "actor1",
                content: "to",
                strategy: "raw",
                namespace: Some("/ns/edges"),
                metadata: None,
                source_session_id: None,
                embedding: None,
            })
            .unwrap();
        add_edge(
            &conn,
            &InsertEdgeParams {
                actor_id: "actor1",
                from_memory_id: &m1.id,
                to_memory_id: &m2.id,
                label: "relates-to",
                properties: None,
            },
        )
        .unwrap();

        conn.delete_namespace("actor1", "/ns/edges").unwrap();

        let edge_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM knowledge_edges", [], |r| r.get(0))
            .unwrap();
        assert_eq!(edge_count, 0);
    }

    #[test]
    fn test_delete_namespace_cleans_memory_vec() {
        use crate::memories::InsertMemoryParams;
        let (_dir, conn) = open_db();
        conn.create_namespace("/ns/vec", None).unwrap();

        let embedding: Vec<f32> = (0..384).map(|i| i as f32 / 384.0).collect();
        conn.insert_memory(&InsertMemoryParams {
            actor_id: "actor1",
            content: "vec memory",
            strategy: "raw",
            namespace: Some("/ns/vec"),
            metadata: None,
            source_session_id: None,
            embedding: Some(&embedding),
        })
        .unwrap();

        // Verify vec row exists before delete
        let before: i64 = conn
            .query_row("SELECT COUNT(*) FROM memory_vec", [], |r| r.get(0))
            .unwrap();
        assert_eq!(before, 1);

        conn.delete_namespace("actor1", "/ns/vec").unwrap();

        let after: i64 = conn
            .query_row("SELECT COUNT(*) FROM memory_vec", [], |r| r.get(0))
            .unwrap();
        assert_eq!(after, 0);
    }
}
