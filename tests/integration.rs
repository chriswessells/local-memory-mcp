mod common;

use base64::Engine;
use common::{parse_err, parse_ok, setup};
use rmcp::handler::server::wrapper::Parameters;
use serde_json::json;

#[tokio::test]
async fn test_event_lifecycle() {
    let (_dir, server) = setup();

    // add_event (conversation)
    let ev = parse_ok(
        server
            .add_event(Parameters(serde_json::from_value(json!({
                "actor_id": "a1", "session_id": "s1", "event_type": "conversation",
                "role": "user", "content": "hello world"
            })).unwrap()))
            .await,
    );
    assert!(ev["id"].is_string());
    assert_eq!(ev["actor_id"], "a1");
    assert!(ev["created_at"].is_string());
    let event_id = ev["id"].as_str().unwrap();

    // get_event
    let ev2 = parse_ok(
        server
            .get_event(Parameters(serde_json::from_value(json!({
                "actor_id": "a1", "event_id": event_id
            })).unwrap()))
            .await,
    );
    assert_eq!(ev2["id"], event_id);

    // get_events
    let evts = parse_ok(
        server
            .get_events(Parameters(serde_json::from_value(json!({
                "actor_id": "a1", "session_id": "s1"
            })).unwrap()))
            .await,
    );
    assert_eq!(evts.as_array().unwrap().len(), 1);

    // list_sessions
    let sessions = parse_ok(
        server
            .list_sessions(Parameters(serde_json::from_value(json!({
                "actor_id": "a1"
            })).unwrap()))
            .await,
    );
    assert_eq!(sessions[0]["event_count"], 1);

    // add expired event + delete_expired
    parse_ok(
        server
            .add_event(Parameters(serde_json::from_value(json!({
                "actor_id": "a1", "session_id": "s1", "event_type": "conversation",
                "role": "user", "content": "expired",
                "expires_at": "2000-01-01T00:00:00Z"
            })).unwrap()))
            .await,
    );
    let del = parse_ok(server.delete_expired().await);
    assert_eq!(del["deleted"], 1);
}

#[tokio::test]
async fn test_memory_lifecycle() {
    let (_dir, server) = setup();

    // store
    let mem = parse_ok(
        server
            .store_memory(Parameters(serde_json::from_value(json!({
                "actor_id": "a1", "content": "rust is great", "strategy": "core"
            })).unwrap()))
            .await,
    );
    assert!(mem["id"].is_string());
    assert_eq!(mem["is_valid"], true);
    let mem_id = mem["id"].as_str().unwrap();

    // get
    let m2 = parse_ok(
        server
            .get_memory(Parameters(serde_json::from_value(json!({
                "actor_id": "a1", "memory_id": mem_id
            })).unwrap()))
            .await,
    );
    assert_eq!(m2["id"], mem_id);

    // list
    let list = parse_ok(
        server
            .list_memories(Parameters(serde_json::from_value(json!({
                "actor_id": "a1"
            })).unwrap()))
            .await,
    );
    assert_eq!(list.as_array().unwrap().len(), 1);

    // consolidate (update)
    let new_mem = parse_ok(
        server
            .consolidate(Parameters(serde_json::from_value(json!({
                "actor_id": "a1", "memory_id": mem_id,
                "action": "update", "new_content": "rust is amazing"
            })).unwrap()))
            .await,
    );
    let new_id = new_mem["id"].as_str().unwrap();
    assert_ne!(new_id, mem_id);

    // list valid_only → only new memory
    let list = parse_ok(
        server
            .list_memories(Parameters(serde_json::from_value(json!({
                "actor_id": "a1", "valid_only": true
            })).unwrap()))
            .await,
    );
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["id"], new_id);

    // delete
    let del = parse_ok(
        server
            .delete_memory(Parameters(serde_json::from_value(json!({
                "actor_id": "a1", "memory_id": new_id
            })).unwrap()))
            .await,
    );
    assert_eq!(del["deleted"], true);

    // get deleted → not_found
    parse_err(
        server
            .get_memory(Parameters(serde_json::from_value(json!({
                "actor_id": "a1", "memory_id": new_id
            })).unwrap()))
            .await,
        "not_found",
    );
}

#[tokio::test]
async fn test_recall_fts() {
    let (_dir, server) = setup();

    for content in ["the quick brown fox", "lazy dog sleeps", "rust programming language"] {
        parse_ok(
            server
                .store_memory(Parameters(serde_json::from_value(json!({
                    "actor_id": "a1", "content": content, "strategy": "core"
                })).unwrap()))
                .await,
        );
    }

    let results = parse_ok(
        server
            .recall(Parameters(serde_json::from_value(json!({
                "actor_id": "a1", "query": "fox"
            })).unwrap()))
            .await,
    );
    let arr = results.as_array().unwrap();
    assert!(!arr.is_empty());
    assert!(arr[0]["score"].as_f64().unwrap() > 0.0);
    assert!(arr[0]["content"].as_str().unwrap().contains("fox"));
}

#[tokio::test]
async fn test_graph_lifecycle() {
    let (_dir, server) = setup();

    let m1 = parse_ok(
        server
            .store_memory(Parameters(serde_json::from_value(json!({
                "actor_id": "a1", "content": "memory one", "strategy": "core"
            })).unwrap()))
            .await,
    )["id"].as_str().unwrap().to_string();

    let m2 = parse_ok(
        server
            .store_memory(Parameters(serde_json::from_value(json!({
                "actor_id": "a1", "content": "memory two", "strategy": "core"
            })).unwrap()))
            .await,
    )["id"].as_str().unwrap().to_string();

    // add_edge
    let edge = parse_ok(
        server
            .add_edge(Parameters(serde_json::from_value(json!({
                "actor_id": "a1", "from_memory_id": m1, "to_memory_id": m2, "label": "relates_to"
            })).unwrap()))
            .await,
    );
    assert!(edge["id"].is_string());
    assert_eq!(edge["label"], "relates_to");
    let edge_id = edge["id"].as_str().unwrap();

    // get_neighbors
    let neighbors = parse_ok(
        server
            .get_neighbors(Parameters(serde_json::from_value(json!({
                "actor_id": "a1", "memory_id": m1
            })).unwrap()))
            .await,
    );
    assert_eq!(neighbors.as_array().unwrap().len(), 1);

    // traverse
    let nodes = parse_ok(
        server
            .traverse(Parameters(serde_json::from_value(json!({
                "actor_id": "a1", "start_memory_id": m1
            })).unwrap()))
            .await,
    );
    assert!(!nodes.as_array().unwrap().is_empty());

    // update_edge
    let updated = parse_ok(
        server
            .update_edge(Parameters(serde_json::from_value(json!({
                "actor_id": "a1", "edge_id": edge_id, "label": "depends_on"
            })).unwrap()))
            .await,
    );
    assert_eq!(updated["label"], "depends_on");

    // delete_edge
    let del = parse_ok(
        server
            .delete_edge(Parameters(serde_json::from_value(json!({
                "actor_id": "a1", "edge_id": edge_id
            })).unwrap()))
            .await,
    );
    assert_eq!(del["deleted"], true);

    // list_labels
    let labels = parse_ok(
        server
            .list_labels(Parameters(serde_json::from_value(json!({
                "actor_id": "a1"
            })).unwrap()))
            .await,
    );
    assert!(labels.is_array());

    // graph_stats
    let stats = parse_ok(
        server
            .graph_stats(Parameters(serde_json::from_value(json!({
                "actor_id": "a1"
            })).unwrap()))
            .await,
    );
    assert_eq!(stats["total_edges"], 0);
}

#[tokio::test]
async fn test_store_isolation() {
    let (_dir, server) = setup();

    // Store memory in default
    parse_ok(
        server
            .store_memory(Parameters(serde_json::from_value(json!({
                "actor_id": "a1", "content": "default mem", "strategy": "core"
            })).unwrap()))
            .await,
    );

    // current_store
    let cs = parse_ok(server.current_store().await);
    assert_eq!(cs["store"], "default");

    // switch to "other"
    parse_ok(
        server
            .switch_store(Parameters(serde_json::from_value(json!({
                "name": "other"
            })).unwrap()))
            .await,
    );

    // list in other → empty
    let list = parse_ok(
        server
            .list_memories(Parameters(serde_json::from_value(json!({
                "actor_id": "a1"
            })).unwrap()))
            .await,
    );
    assert_eq!(list.as_array().unwrap().len(), 0);

    // store in other
    parse_ok(
        server
            .store_memory(Parameters(serde_json::from_value(json!({
                "actor_id": "a1", "content": "other mem", "strategy": "core"
            })).unwrap()))
            .await,
    );

    // switch back to default
    parse_ok(
        server
            .switch_store(Parameters(serde_json::from_value(json!({
                "name": "default"
            })).unwrap()))
            .await,
    );

    // list → only original
    let list = parse_ok(
        server
            .list_memories(Parameters(serde_json::from_value(json!({
                "actor_id": "a1"
            })).unwrap()))
            .await,
    );
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["content"], "default mem");

    // list_stores → both
    let stores = parse_ok(server.list_stores().await);
    assert_eq!(stores.as_array().unwrap().len(), 2);

    // delete "other"
    let del = parse_ok(
        server
            .delete_store(Parameters(serde_json::from_value(json!({
                "name": "other"
            })).unwrap()))
            .await,
    );
    assert_eq!(del["deleted"], true);
}

#[tokio::test]
async fn test_actor_isolation() {
    let (_dir, server) = setup();

    // Store 2 memories as alice
    let alice_m1 = parse_ok(
        server
            .store_memory(Parameters(serde_json::from_value(json!({
                "actor_id": "alice", "content": "alice mem 1", "strategy": "core"
            })).unwrap()))
            .await,
    )["id"].as_str().unwrap().to_string();

    let alice_m2 = parse_ok(
        server
            .store_memory(Parameters(serde_json::from_value(json!({
                "actor_id": "alice", "content": "alice mem 2", "strategy": "core"
            })).unwrap()))
            .await,
    )["id"].as_str().unwrap().to_string();

    // bob can't see alice's memory
    parse_err(
        server
            .get_memory(Parameters(serde_json::from_value(json!({
                "actor_id": "bob", "memory_id": alice_m1
            })).unwrap()))
            .await,
        "not_found",
    );

    // bob stores his own
    parse_ok(
        server
            .store_memory(Parameters(serde_json::from_value(json!({
                "actor_id": "bob", "content": "bob mem", "strategy": "core"
            })).unwrap()))
            .await,
    );

    // alice list → only her 2
    let list = parse_ok(
        server
            .list_memories(Parameters(serde_json::from_value(json!({
                "actor_id": "alice"
            })).unwrap()))
            .await,
    );
    assert_eq!(list.as_array().unwrap().len(), 2);

    // add edge between alice's memories
    parse_ok(
        server
            .add_edge(Parameters(serde_json::from_value(json!({
                "actor_id": "alice", "from_memory_id": alice_m1,
                "to_memory_id": alice_m2, "label": "link"
            })).unwrap()))
            .await,
    );

    // bob can't see alice's neighbors
    let neighbors = parse_ok(
        server
            .get_neighbors(Parameters(serde_json::from_value(json!({
                "actor_id": "bob", "memory_id": alice_m1
            })).unwrap()))
            .await,
    );
    assert_eq!(neighbors.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn test_blob_event_roundtrip() {
    let (_dir, server) = setup();

    let original = base64::engine::general_purpose::STANDARD.encode(b"hello binary");

    let ev = parse_ok(
        server
            .add_event(Parameters(serde_json::from_value(json!({
                "actor_id": "a1", "session_id": "s1", "event_type": "blob",
                "blob_data": original
            })).unwrap()))
            .await,
    );
    let event_id = ev["id"].as_str().unwrap();

    let ev2 = parse_ok(
        server
            .get_event(Parameters(serde_json::from_value(json!({
                "actor_id": "a1", "event_id": event_id
            })).unwrap()))
            .await,
    );
    assert_eq!(ev2["blob_data"].as_str().unwrap(), original);
}

#[tokio::test]
async fn test_error_responses() {
    let (_dir, server) = setup();

    // get nonexistent event → not_found
    let e = parse_err(
        server
            .get_event(Parameters(serde_json::from_value(json!({
                "actor_id": "a1", "event_id": "nonexistent"
            })).unwrap()))
            .await,
        "not_found",
    );
    assert!(e["message"].is_string());

    // store empty content → invalid_input
    parse_err(
        server
            .store_memory(Parameters(serde_json::from_value(json!({
                "actor_id": "a1", "content": "", "strategy": "core"
            })).unwrap()))
            .await,
        "invalid_input",
    );

    // self-edge → invalid_input
    let mid = parse_ok(
        server
            .store_memory(Parameters(serde_json::from_value(json!({
                "actor_id": "a1", "content": "for edge test", "strategy": "core"
            })).unwrap()))
            .await,
    )["id"].as_str().unwrap().to_string();

    parse_err(
        server
            .add_edge(Parameters(serde_json::from_value(json!({
                "actor_id": "a1", "from_memory_id": mid,
                "to_memory_id": mid, "label": "self"
            })).unwrap()))
            .await,
        "invalid_input",
    );
}
