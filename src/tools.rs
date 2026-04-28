use std::sync::{Arc, Mutex};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::{schemars, tool, tool_router};
use serde::Deserialize;

use crate::error::MemoryError;
use crate::events::{self, BranchFilter, Event, InsertEventParams, DEFAULT_PAGE_LIMIT};
use crate::graph::{
    self, Direction, InsertEdgeParams as GraphInsertEdgeParams,
    UpdateEdgeParams as GraphUpdateEdgeParams,
};
use crate::memories::{self, ConsolidateAction, InsertMemoryParams, ListMemoriesParams};
use crate::namespaces;
use crate::search::{self, RecallParams};
use crate::sessions;
use crate::store::StoreManager;

// --- MemoryServer ---

#[derive(Clone)]
pub struct MemoryServer {
    store: Arc<Mutex<StoreManager>>,
}

impl MemoryServer {
    pub fn new(store: Arc<Mutex<StoreManager>>) -> Self {
        Self { store }
    }

    async fn run<F, T>(&self, f: F) -> Result<String, String>
    where
        F: FnOnce(&mut StoreManager) -> Result<T, MemoryError> + Send + 'static,
        T: serde::Serialize + Send + 'static,
    {
        let store = self.store.clone();
        match tokio::task::spawn_blocking(move || {
            let mut mgr = store.lock().unwrap_or_else(|e| {
                tracing::warn!("mutex was poisoned by a previous panic, recovering");
                e.into_inner()
            });
            f(&mut mgr)
        })
        .await
        {
            Ok(Ok(value)) => serde_json::to_string(&value).map_err(|e| {
                serde_json::json!({"code": "internal", "message": format!("serialization error: {e}")}).to_string()
            }),
            Ok(Err(e)) => {
                let code = match &e {
                    MemoryError::NotFound(_) => "not_found",
                    MemoryError::InvalidInput(_) | MemoryError::InvalidName(_) => "invalid_input",
                    MemoryError::ActiveStoreDeletion(_) => "invalid_input",
                    _ => "internal",
                };
                Err(format!(
                    r#"{{"code":"{code}","message":{}}}"#,
                    serde_json::to_string(&e.to_string()).unwrap_or_else(|_| format!("\"{e}\""))
                ))
            }
            Err(join_err) => {
                tracing::error!("tool handler panicked: {join_err}");
                Err(r#"{"code":"internal","message":"internal error"}"#.into())
            }
        }
    }
}

// --- MCP-facing enums ---

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    Conversation,
    Blob,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
    Tool,
    System,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConsolidateActionType {
    Update,
    Invalidate,
}

// --- Param structs ---

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AddEventParams {
    actor_id: String,
    session_id: String,
    event_type: EventType,
    #[serde(default)]
    role: Option<Role>,
    #[serde(default)]
    content: Option<String>,
    /// Base64-encoded binary data for blob events
    #[serde(default)]
    blob_data: Option<String>,
    #[serde(default)]
    metadata: Option<String>,
    #[serde(default)]
    branch_id: Option<String>,
    #[serde(default)]
    expires_at: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetEventParams {
    actor_id: String,
    event_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetEventsToolParams {
    actor_id: String,
    session_id: String,
    /// "all" (default), "main" (main timeline only), or a specific branch ID
    #[serde(default)]
    branch_filter: Option<String>,
    #[serde(default)]
    limit: Option<u32>,
    #[serde(default)]
    offset: Option<u32>,
    #[serde(default)]
    before: Option<String>,
    #[serde(default)]
    after: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListSessionsParams {
    actor_id: String,
    #[serde(default)]
    limit: Option<u32>,
    #[serde(default)]
    offset: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct StoreMemoryParams {
    actor_id: String,
    content: String,
    strategy: String,
    #[serde(default)]
    namespace: Option<String>,
    #[serde(default)]
    metadata: Option<String>,
    #[serde(default)]
    source_session_id: Option<String>,
    /// 384-dimensional float32 embedding vector
    #[schemars(extend("minItems" = 384, "maxItems" = 384))]
    #[serde(default)]
    embedding: Option<Vec<f32>>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetMemoryParams {
    actor_id: String,
    memory_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListMemoriesToolParams {
    actor_id: String,
    #[serde(default)]
    namespace: Option<String>,
    #[serde(default)]
    namespace_prefix: Option<String>,
    #[serde(default)]
    strategy: Option<String>,
    #[serde(default = "default_true")]
    valid_only: bool,
    #[serde(default)]
    limit: Option<u32>,
    #[serde(default)]
    offset: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ConsolidateParams {
    actor_id: String,
    memory_id: String,
    action: ConsolidateActionType,
    /// Required when action is "update"
    #[serde(default)]
    new_content: Option<String>,
    /// 384-dimensional float32 embedding vector for the replacement memory
    #[schemars(extend("minItems" = 384, "maxItems" = 384))]
    #[serde(default)]
    new_embedding: Option<Vec<f32>>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DeleteMemoryParams {
    actor_id: String,
    memory_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RecallToolParams {
    actor_id: String,
    #[serde(default)]
    query: Option<String>,
    /// 384-dimensional float32 embedding vector
    #[schemars(extend("minItems" = 384, "maxItems" = 384))]
    #[serde(default)]
    embedding: Option<Vec<f32>>,
    #[serde(default)]
    namespace: Option<String>,
    #[serde(default)]
    namespace_prefix: Option<String>,
    #[serde(default)]
    strategy: Option<String>,
    #[serde(default)]
    limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SwitchStoreParams {
    name: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DeleteStoreParams {
    name: String,
}

// -- Graph param structs --

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AddEdgeParams {
    actor_id: String,
    from_memory_id: String,
    to_memory_id: String,
    label: String,
    #[serde(default)]
    properties: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetNeighborsParams {
    actor_id: String,
    memory_id: String,
    /// Direction: "out" (default), "in", or "both"
    #[serde(default)]
    direction: Option<Direction>,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TraverseParams {
    actor_id: String,
    start_memory_id: String,
    /// Max traversal depth (default 2, max 5)
    #[serde(default)]
    max_depth: Option<u32>,
    #[serde(default)]
    label: Option<String>,
    /// Direction: "out" (default), "in", or "both"
    #[serde(default)]
    direction: Option<Direction>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct UpdateEdgeToolParams {
    actor_id: String,
    edge_id: String,
    #[serde(default)]
    label: Option<String>,
    /// JSON object string for edge properties
    #[serde(default)]
    properties: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DeleteEdgeParams {
    actor_id: String,
    edge_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListLabelsParams {
    actor_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GraphStatsParams {
    actor_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateNamespaceToolParams {
    /// Namespace path, e.g. "/user/alice/preferences". Up to 512 bytes (UTF-8).
    /// Must not contain control characters.
    #[schemars(length(max = 512))]
    name: String,
    #[serde(default)]
    #[schemars(length(max = 1024))]
    description: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListNamespacesToolParams {
    /// If provided, return only namespaces whose name starts with this prefix.
    #[serde(default)]
    prefix: Option<String>,
    #[serde(default)]
    limit: Option<u32>,
    #[serde(default)]
    offset: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DeleteNamespaceToolParams {
    actor_id: String,
    name: String,
}

// -- Session param structs (Component 6) --

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CheckpointToolParams {
    actor_id: String,
    session_id: String,
    name: String,
    event_id: String,
    #[serde(default)]
    metadata: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct BranchToolParams {
    actor_id: String,
    session_id: String,
    root_event_id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    parent_branch_id: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListCheckpointsToolParams {
    actor_id: String,
    session_id: String,
    #[serde(default)]
    limit: Option<u32>,
    #[serde(default)]
    offset: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListBranchesToolParams {
    actor_id: String,
    session_id: String,
    #[serde(default)]
    limit: Option<u32>,
    #[serde(default)]
    offset: Option<u32>,
}

// --- Helpers ---

fn parse_branch_filter(s: Option<&str>) -> BranchFilter<'_> {
    match s {
        None | Some("all") => BranchFilter::All,
        Some("main") => BranchFilter::MainOnly,
        Some(id) => BranchFilter::Specific(id),
    }
}

fn parse_consolidate_action<'a>(
    action: &ConsolidateActionType,
    new_content: Option<&'a str>,
    new_embedding: Option<&'a [f32]>,
) -> Result<ConsolidateAction<'a>, MemoryError> {
    match action {
        ConsolidateActionType::Update => {
            let content = new_content.ok_or_else(|| {
                MemoryError::InvalidInput("new_content is required for update action".into())
            })?;
            Ok(ConsolidateAction::Update {
                content,
                embedding: new_embedding,
            })
        }
        ConsolidateActionType::Invalidate => Ok(ConsolidateAction::Invalidate),
    }
}

/// Encode blob_data in an Event to base64 for JSON transport.
fn encode_event_blob(event: &Event) -> Result<serde_json::Value, MemoryError> {
    let mut v = serde_json::to_value(event)
        .map_err(|e| MemoryError::QueryFailed(format!("event serialization failed: {e}")))?;
    if let Some(blob) = &event.blob_data {
        v["blob_data"] = serde_json::Value::String(BASE64.encode(blob));
    }
    Ok(v)
}

fn encode_event_blobs(events: &[Event]) -> Result<Vec<serde_json::Value>, MemoryError> {
    events.iter().map(encode_event_blob).collect()
}

// --- Tool implementations ---

#[tool_router(server_handler)]
impl MemoryServer {
    #[tool(
        name = "memory.add_event",
        description = "Store an immutable conversation or blob event in a session timeline. Event type must be 'conversation' (requires content) or 'blob' (requires base64-encoded blob_data). Returns the full event object with generated id and created_at timestamp."
    )]
    pub async fn add_event(
        &self,
        Parameters(params): Parameters<AddEventParams>,
    ) -> Result<String, String> {
        let blob_bytes = match &params.blob_data {
            Some(b64) => Some(BASE64.decode(b64).map_err(|e| {
                format!(r#"{{"code":"invalid_input","message":"invalid base64 blob_data: {e}"}}"#)
            })?),
            None => None,
        };
        self.run(move |mgr| {
            let db = mgr.db()?;
            let event_type = match params.event_type {
                EventType::Conversation => "conversation",
                EventType::Blob => "blob",
            };
            let role = params.role.as_ref().map(|r| match r {
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::Tool => "tool",
                Role::System => "system",
            });
            let p = InsertEventParams {
                actor_id: &params.actor_id,
                session_id: &params.session_id,
                event_type,
                role,
                content: params.content.as_deref(),
                blob_data: blob_bytes.as_deref(),
                metadata: params.metadata.as_deref(),
                branch_id: params.branch_id.as_deref(),
                expires_at: params.expires_at.as_deref(),
            };
            let event = events::add_event(db, &p)?;
            encode_event_blob(&event)
        })
        .await
    }

    #[tool(
        name = "memory.get_event",
        description = "Retrieve a single event by its ID, scoped to the given actor. Returns the full event object or a not_found error if the event does not exist for this actor."
    )]
    pub async fn get_event(
        &self,
        Parameters(params): Parameters<GetEventParams>,
    ) -> Result<String, String> {
        self.run(move |mgr| {
            let db = mgr.db()?;
            let event = events::get_event(db, &params.actor_id, &params.event_id)?;
            encode_event_blob(&event)
        })
        .await
    }

    #[tool(
        name = "memory.get_events",
        description = "Retrieve events for an actor+session with optional branch, time range, and pagination filters. Events are returned in chronological order (oldest first). Use branch_filter 'all' (default), 'main', or a specific branch ID."
    )]
    pub async fn get_events(
        &self,
        Parameters(params): Parameters<GetEventsToolParams>,
    ) -> Result<String, String> {
        self.run(move |mgr| {
            let db = mgr.db()?;
            let bf = parse_branch_filter(params.branch_filter.as_deref());
            let p = events::GetEventsParams {
                actor_id: &params.actor_id,
                session_id: &params.session_id,
                branch_id: bf,
                limit: params.limit.unwrap_or(100),
                offset: params.offset.unwrap_or(0),
                before: params.before.as_deref(),
                after: params.after.as_deref(),
            };
            let evts = events::get_events(db, &p)?;
            encode_event_blobs(&evts)
        })
        .await
    }

    #[tool(
        name = "memory.list_sessions",
        description = "List distinct sessions for an actor with event counts and date ranges. Results are ordered by last event time descending. Supports limit/offset pagination."
    )]
    pub async fn list_sessions(
        &self,
        Parameters(params): Parameters<ListSessionsParams>,
    ) -> Result<String, String> {
        self.run(move |mgr| {
            let db = mgr.db()?;
            events::list_sessions(
                db,
                &params.actor_id,
                params.limit.unwrap_or(100),
                params.offset.unwrap_or(0),
            )
        })
        .await
    }

    #[tool(
        name = "memory.delete_expired",
        description = "Delete all events whose expires_at timestamp is in the past. Returns a JSON object with the count of deleted events. This is a maintenance operation with no required parameters."
    )]
    pub async fn delete_expired(&self) -> Result<String, String> {
        self.run(|mgr| {
            let db = mgr.db()?;
            let count = events::delete_expired(db)?;
            Ok(serde_json::json!({ "deleted": count }))
        })
        .await
    }

    #[tool(
        name = "memory.store",
        description = "Store an extracted insight as a long-term memory. Requires actor_id, content, and strategy. Optionally provide a 384-dim embedding vector for vector search. Returns the full memory object with generated id."
    )]
    pub async fn store_memory(
        &self,
        Parameters(params): Parameters<StoreMemoryParams>,
    ) -> Result<String, String> {
        self.run(move |mgr| {
            let db = mgr.db()?;
            let p = InsertMemoryParams {
                actor_id: &params.actor_id,
                content: &params.content,
                strategy: &params.strategy,
                namespace: params.namespace.as_deref(),
                metadata: params.metadata.as_deref(),
                source_session_id: params.source_session_id.as_deref(),
                embedding: params.embedding.as_deref(),
            };
            memories::store_memory(db, &p)
        })
        .await
    }

    #[tool(
        name = "memory.get",
        description = "Retrieve a single memory by its ID, scoped to the given actor. Returns the full memory object or a not_found error."
    )]
    pub async fn get_memory(
        &self,
        Parameters(params): Parameters<GetMemoryParams>,
    ) -> Result<String, String> {
        self.run(move |mgr| {
            let db = mgr.db()?;
            memories::get_memory(db, &params.actor_id, &params.memory_id)
        })
        .await
    }

    #[tool(
        name = "memory.list",
        description = "List memories for an actor with optional namespace, namespace_prefix, strategy, and validity filters. Results are ordered by created_at descending. Supports limit/offset pagination. By default only valid memories are returned."
    )]
    pub async fn list_memories(
        &self,
        Parameters(params): Parameters<ListMemoriesToolParams>,
    ) -> Result<String, String> {
        self.run(move |mgr| {
            let db = mgr.db()?;
            let p = ListMemoriesParams {
                actor_id: &params.actor_id,
                namespace: params.namespace.as_deref(),
                namespace_prefix: params.namespace_prefix.as_deref(),
                strategy: params.strategy.as_deref(),
                valid_only: params.valid_only,
                limit: params.limit.unwrap_or(100),
                offset: params.offset.unwrap_or(0),
            };
            memories::list_memories(db, &p)
        })
        .await
    }

    #[tool(
        name = "memory.consolidate",
        description = "Update or invalidate an existing memory. Action 'update' requires new_content and creates a replacement memory, marking the old one invalid. Action 'invalidate' marks the memory invalid with no replacement. Returns the resulting memory object."
    )]
    pub async fn consolidate(
        &self,
        Parameters(params): Parameters<ConsolidateParams>,
    ) -> Result<String, String> {
        self.run(move |mgr| {
            let db = mgr.db()?;
            let action = parse_consolidate_action(
                &params.action,
                params.new_content.as_deref(),
                params.new_embedding.as_deref(),
            )?;
            memories::consolidate_memory(db, &params.actor_id, &params.memory_id, &action)
        })
        .await
    }

    #[tool(
        name = "memory.delete",
        description = "Permanently delete a memory and its embedding, scoped to the given actor. Returns {\"deleted\": true} on success or a not_found error if the memory does not exist."
    )]
    pub async fn delete_memory(
        &self,
        Parameters(params): Parameters<DeleteMemoryParams>,
    ) -> Result<String, String> {
        self.run(move |mgr| {
            let db = mgr.db()?;
            memories::delete_memory(db, &params.actor_id, &params.memory_id)?;
            Ok(serde_json::json!({ "deleted": true }))
        })
        .await
    }

    #[tool(
        name = "memory.recall",
        description = "Search memories by text query, embedding vector, or both (hybrid RRF fusion). At least one of query or embedding must be provided. Returns a list of matching memories with relevance scores. Scores are not comparable across different search modes."
    )]
    pub async fn recall(
        &self,
        Parameters(params): Parameters<RecallToolParams>,
    ) -> Result<String, String> {
        self.run(move |mgr| {
            let db = mgr.db()?;
            let p = RecallParams {
                actor_id: &params.actor_id,
                query: params.query.as_deref(),
                embedding: params.embedding.as_deref(),
                namespace: params.namespace.as_deref(),
                namespace_prefix: params.namespace_prefix.as_deref(),
                strategy: params.strategy.as_deref(),
                limit: params.limit.unwrap_or(10),
            };
            search::recall(db, &p)
        })
        .await
    }

    #[tool(
        name = "memory.switch_store",
        description = "Switch to a different named store, creating it if it does not exist. The previous store is checkpointed and closed. Store names must be 1-64 alphanumeric characters (plus hyphens/underscores)."
    )]
    pub async fn switch_store(
        &self,
        Parameters(params): Parameters<SwitchStoreParams>,
    ) -> Result<String, String> {
        self.run(move |mgr| {
            mgr.switch(&params.name)?;
            Ok(serde_json::json!({ "store": params.name }))
        })
        .await
    }

    #[tool(
        name = "memory.current_store",
        description = "Return the name of the currently active store. Returns a JSON object with the store name, or null if no store is open."
    )]
    pub async fn current_store(&self) -> Result<String, String> {
        self.run(|mgr| {
            let name = mgr.active_name().map(|s| s.to_string());
            Ok(serde_json::json!({ "store": name }))
        })
        .await
    }

    #[tool(
        name = "memory.list_stores",
        description = "List all stores in the base directory with their names and sizes in bytes. Returns an array of store info objects sorted alphabetically by name."
    )]
    pub async fn list_stores(&self) -> Result<String, String> {
        self.run(|mgr| {
            let stores = mgr.list()?;
            let out: Vec<serde_json::Value> = stores
                .into_iter()
                .map(|s| serde_json::json!({ "name": s.name, "size_bytes": s.size_bytes }))
                .collect();
            Ok(out)
        })
        .await
    }

    #[tool(
        name = "memory.delete_store",
        description = "Delete a named store and its auxiliary files. Cannot delete the currently active store. Returns {\"deleted\": true} on success or an error if the store is active or not found."
    )]
    pub async fn delete_store(
        &self,
        Parameters(params): Parameters<DeleteStoreParams>,
    ) -> Result<String, String> {
        self.run(move |mgr| {
            mgr.delete(&params.name)?;
            Ok(serde_json::json!({ "deleted": true }))
        })
        .await
    }

    // -- Namespace tools (Component 8) --

    #[tool(
        name = "memory.create_namespace",
        description = "Register a namespace with optional description. Idempotent — if the namespace already exists, returns the existing entry unchanged. Namespace names are UTF-8 strings up to 512 bytes (e.g., '/user/alice/preferences'). Must not contain control characters."
    )]
    pub async fn create_namespace(
        &self,
        Parameters(params): Parameters<CreateNamespaceToolParams>,
    ) -> Result<String, String> {
        self.run(move |mgr| {
            let db = mgr.db()?;
            let p = namespaces::CreateNamespaceParams {
                name: &params.name,
                description: params.description.as_deref(),
            };
            let ns = namespaces::create_namespace(db, &p)?;
            Ok(serde_json::json!({ "namespace": ns }))
        })
        .await
    }

    #[tool(
        name = "memory.list_namespaces",
        description = "List registered namespaces ordered alphabetically. Only namespaces explicitly created via memory.create_namespace are returned — not all namespaces referenced by memories. Supports optional prefix filter and limit/offset pagination."
    )]
    pub async fn list_namespaces(
        &self,
        Parameters(params): Parameters<ListNamespacesToolParams>,
    ) -> Result<String, String> {
        self.run(move |mgr| {
            let db = mgr.db()?;
            let p = namespaces::ListNamespacesParams {
                prefix: params.prefix.as_deref(),
                limit: params.limit.unwrap_or(DEFAULT_PAGE_LIMIT),
                offset: params.offset.unwrap_or(0),
            };
            let list = namespaces::list_namespaces(db, &p)?;
            Ok(serde_json::json!({ "namespaces": list }))
        })
        .await
    }

    #[tool(
        name = "memory.delete_namespace",
        description = "Delete all memories belonging to actor_id in the named namespace, clean up their vector rows, and remove the namespace registry entry. Scoped to actor_id — other actors' memories in the same namespace path are not affected. Deletes in chunks of 500 to avoid blocking the server. Returns not_found if the namespace is not registered."
    )]
    pub async fn delete_namespace(
        &self,
        Parameters(params): Parameters<DeleteNamespaceToolParams>,
    ) -> Result<String, String> {
        self.run(move |mgr| {
            let db = mgr.db()?;
            let memories_deleted =
                namespaces::delete_namespace(db, &params.actor_id, &params.name)?;
            Ok(serde_json::json!({ "deleted": true, "memories_deleted": memories_deleted }))
        })
        .await
    }

    // -- Graph tools (Component 5) --

    #[tool(
        name = "graph.add_edge",
        description = "Create a directed edge between two memories. Both memories must belong to the same actor. Self-edges are not allowed. Returns the full edge object."
    )]
    pub async fn add_edge(
        &self,
        Parameters(params): Parameters<AddEdgeParams>,
    ) -> Result<String, String> {
        self.run(move |mgr| {
            let db = mgr.db()?;
            let p = GraphInsertEdgeParams {
                actor_id: &params.actor_id,
                from_memory_id: &params.from_memory_id,
                to_memory_id: &params.to_memory_id,
                label: &params.label,
                properties: params.properties.as_deref(),
            };
            graph::add_edge(db, &p)
        })
        .await
    }

    #[tool(
        name = "graph.get_neighbors",
        description = "Get neighbors of a memory via its edges. Returns edges and connected memories. Direction: 'out' (default), 'in', or 'both'."
    )]
    pub async fn get_neighbors(
        &self,
        Parameters(params): Parameters<GetNeighborsParams>,
    ) -> Result<String, String> {
        self.run(move |mgr| {
            let db = mgr.db()?;
            graph::get_neighbors(
                db,
                &params.actor_id,
                &params.memory_id,
                params.direction.unwrap_or_default(),
                params.label.as_deref(),
                params.limit.unwrap_or(graph::DEFAULT_NEIGHBOR_LIMIT),
            )
        })
        .await
    }

    #[tool(
        name = "graph.traverse",
        description = "BFS traversal from a start memory through edges. Returns visited memories with depth and path. Direction: 'out' (default), 'in', or 'both'. Max depth 5."
    )]
    pub async fn traverse(
        &self,
        Parameters(params): Parameters<TraverseParams>,
    ) -> Result<String, String> {
        self.run(move |mgr| {
            let db = mgr.db()?;
            graph::traverse(
                db,
                &params.actor_id,
                &params.start_memory_id,
                params.max_depth.unwrap_or(graph::DEFAULT_TRAVERSE_DEPTH),
                params.label.as_deref(),
                params.direction.unwrap_or_default(),
            )
        })
        .await
    }

    #[tool(
        name = "graph.update_edge",
        description = "Update an edge's label and/or properties. At least one must be provided. Returns the updated edge object."
    )]
    pub async fn update_edge(
        &self,
        Parameters(params): Parameters<UpdateEdgeToolParams>,
    ) -> Result<String, String> {
        self.run(move |mgr| {
            let db = mgr.db()?;
            let p = GraphUpdateEdgeParams {
                actor_id: &params.actor_id,
                edge_id: &params.edge_id,
                label: params.label.as_deref(),
                properties: params.properties.as_deref(),
            };
            graph::update_edge(db, &p)
        })
        .await
    }

    #[tool(
        name = "graph.delete_edge",
        description = "Delete an edge by ID, scoped to the given actor. Returns {\"deleted\": true} on success."
    )]
    pub async fn delete_edge(
        &self,
        Parameters(params): Parameters<DeleteEdgeParams>,
    ) -> Result<String, String> {
        self.run(move |mgr| {
            let db = mgr.db()?;
            graph::delete_edge(db, &params.actor_id, &params.edge_id)?;
            Ok(serde_json::json!({ "deleted": true }))
        })
        .await
    }

    #[tool(
        name = "graph.list_labels",
        description = "List all distinct edge labels with their counts for the given actor, ordered by count descending."
    )]
    pub async fn list_labels(
        &self,
        Parameters(params): Parameters<ListLabelsParams>,
    ) -> Result<String, String> {
        self.run(move |mgr| {
            let db = mgr.db()?;
            graph::list_labels(db, &params.actor_id)
        })
        .await
    }

    #[tool(
        name = "graph.stats",
        description = "Get graph statistics for the given actor: total edge count, label distribution, and top 10 most connected memories."
    )]
    pub async fn graph_stats(
        &self,
        Parameters(params): Parameters<GraphStatsParams>,
    ) -> Result<String, String> {
        self.run(move |mgr| {
            let db = mgr.db()?;
            graph::graph_stats(db, &params.actor_id)
        })
        .await
    }

    // -- Session tools (Component 6) --

    #[tool(
        name = "memory.checkpoint",
        description = "Create a named checkpoint at a specific event within a session. \
                       Checkpoints are named snapshots used for workflow resumption and \
                       conversation bookmarks. Name must be unique per session. Returns \
                       the created checkpoint object."
    )]
    pub async fn checkpoint(
        &self,
        Parameters(params): Parameters<CheckpointToolParams>,
    ) -> Result<String, String> {
        self.run(move |mgr| {
            let db = mgr.db()?;
            let p = sessions::InsertCheckpointParams {
                actor_id: &params.actor_id,
                session_id: &params.session_id,
                name: &params.name,
                event_id: &params.event_id,
                metadata: params.metadata.as_deref(),
            };
            sessions::create_checkpoint(db, &p)
                .map(|cp| serde_json::json!({ "checkpoint": cp }))
        })
        .await
    }

    #[tool(
        name = "memory.branch",
        description = "Fork a conversation by creating a branch from a specific event. \
                       Branches enable alternative conversation paths, message editing, \
                       and what-if scenarios. Returns the created branch object with its ID \
                       to use as branch_id in memory.add_event."
    )]
    pub async fn branch(
        &self,
        Parameters(params): Parameters<BranchToolParams>,
    ) -> Result<String, String> {
        self.run(move |mgr| {
            let db = mgr.db()?;
            let p = sessions::InsertBranchParams {
                actor_id: &params.actor_id,
                session_id: &params.session_id,
                root_event_id: &params.root_event_id,
                name: params.name.as_deref(),
                parent_branch_id: params.parent_branch_id.as_deref(),
            };
            sessions::create_branch(db, &p)
                .map(|br| serde_json::json!({ "branch": br }))
        })
        .await
    }

    #[tool(
        name = "memory.list_checkpoints",
        description = "List all checkpoints for a session, ordered by creation time. \
                       Returns an array of checkpoint objects with names and event IDs."
    )]
    pub async fn list_checkpoints(
        &self,
        Parameters(params): Parameters<ListCheckpointsToolParams>,
    ) -> Result<String, String> {
        self.run(move |mgr| {
            let db = mgr.db()?;
            let p = sessions::ListCheckpointsParams {
                actor_id: &params.actor_id,
                session_id: &params.session_id,
                limit: params.limit.unwrap_or(sessions::DEFAULT_CHECKPOINT_LIMIT),
                offset: params.offset.unwrap_or(0),
            };
            sessions::list_checkpoints(db, &p)
                .map(|cps| serde_json::json!({ "checkpoints": cps }))
        })
        .await
    }

    #[tool(
        name = "memory.list_branches",
        description = "List branches for a session, ordered by creation time. \
                       Returns an array of branch objects including their root event IDs \
                       and optional names. Use the returned branch id as branch_id in memory.add_event."
    )]
    pub async fn list_branches(
        &self,
        Parameters(params): Parameters<ListBranchesToolParams>,
    ) -> Result<String, String> {
        self.run(move |mgr| {
            let db = mgr.db()?;
            let p = sessions::ListBranchesParams {
                actor_id: &params.actor_id,
                session_id: &params.session_id,
                limit: params.limit.unwrap_or(sessions::DEFAULT_CHECKPOINT_LIMIT),
                offset: params.offset.unwrap_or(0),
            };
            sessions::list_branches(db, &p)
                .map(|brs| serde_json::json!({ "branches": brs }))
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_parse_branch_filter() {
        assert!(matches!(parse_branch_filter(None), BranchFilter::All));
        assert!(matches!(
            parse_branch_filter(Some("all")),
            BranchFilter::All
        ));
        assert!(matches!(
            parse_branch_filter(Some("main")),
            BranchFilter::MainOnly
        ));
        match parse_branch_filter(Some("abc")) {
            BranchFilter::Specific(id) => assert_eq!(id, "abc"),
            _ => panic!("expected Specific"),
        }
    }

    #[test]
    fn test_parse_consolidate_action() {
        // Update with content
        let action =
            parse_consolidate_action(&ConsolidateActionType::Update, Some("new"), None).unwrap();
        assert!(matches!(
            action,
            ConsolidateAction::Update {
                content: "new",
                embedding: None
            }
        ));

        // Update without content → error
        let err = parse_consolidate_action(&ConsolidateActionType::Update, None, None).unwrap_err();
        assert!(matches!(err, MemoryError::InvalidInput(_)));

        // Invalidate
        let action =
            parse_consolidate_action(&ConsolidateActionType::Invalidate, None, None).unwrap();
        assert!(matches!(action, ConsolidateAction::Invalidate));
    }

    #[tokio::test]
    async fn test_run_ok() {
        let dir = TempDir::new().unwrap();
        let mut mgr = StoreManager::with_base_dir(dir.path().to_path_buf()).unwrap();
        mgr.open_default().unwrap();
        let server = MemoryServer::new(Arc::new(Mutex::new(mgr)));

        let result = server.run(|_mgr| Ok("hello")).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "\"hello\"");
    }

    #[tokio::test]
    async fn test_run_error() {
        let dir = TempDir::new().unwrap();
        let mut mgr = StoreManager::with_base_dir(dir.path().to_path_buf()).unwrap();
        mgr.open_default().unwrap();
        let server = MemoryServer::new(Arc::new(Mutex::new(mgr)));

        let result = server
            .run(|_mgr| Err::<(), _>(MemoryError::NotFound("x".into())))
            .await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("not_found"));
    }

    fn make_server() -> MemoryServer {
        let dir = TempDir::new().unwrap();
        let mut mgr = StoreManager::with_base_dir(dir.path().to_path_buf()).unwrap();
        mgr.open_default().unwrap();
        // Keep dir alive by leaking — acceptable in tests
        std::mem::forget(dir);
        MemoryServer::new(Arc::new(Mutex::new(mgr)))
    }

    #[tokio::test]
    async fn test_tool_create_namespace() {
        let server = make_server();
        let params = CreateNamespaceToolParams {
            name: "/user/test".into(),
            description: Some("test namespace".into()),
        };
        let result = server
            .run(move |mgr| {
                let db = mgr.db()?;
                let p = namespaces::CreateNamespaceParams {
                    name: &params.name,
                    description: params.description.as_deref(),
                };
                let ns = namespaces::create_namespace(db, &p)?;
                Ok(serde_json::json!({ "namespace": ns }))
            })
            .await;
        assert!(result.is_ok());
        let v: serde_json::Value = serde_json::from_str(&result.unwrap()).unwrap();
        assert_eq!(v["namespace"]["name"], "/user/test");
        assert_eq!(v["namespace"]["description"], "test namespace");
    }

    #[tokio::test]
    async fn test_tool_list_namespaces_empty() {
        let server = make_server();
        let result = server
            .run(move |mgr| {
                let db = mgr.db()?;
                let p = namespaces::ListNamespacesParams {
                    prefix: None,
                    limit: 100,
                    offset: 0,
                };
                let list = namespaces::list_namespaces(db, &p)?;
                Ok(serde_json::json!({ "namespaces": list }))
            })
            .await;
        assert!(result.is_ok());
        let v: serde_json::Value = serde_json::from_str(&result.unwrap()).unwrap();
        assert_eq!(v["namespaces"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn test_tool_delete_namespace_not_found() {
        let server = make_server();
        let result = server
            .run(move |mgr| {
                let db = mgr.db()?;
                namespaces::delete_namespace(db, "actor1", "/nonexistent")
                    .map(|n| serde_json::json!({ "deleted": true, "memories_deleted": n }))
            })
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not_found"));
    }

    // -- Session tool tests (Component 6) --

    fn add_test_event(server: &MemoryServer, actor: &str, session: &str) -> String {
        let actor = actor.to_string();
        let session = session.to_string();
        // Use a blocking call pattern compatible with the test
        let store = server.store.clone();
        let event = std::thread::spawn(move || {
            let mgr = store.lock().unwrap();
            let db = mgr.db().unwrap();
            crate::events::add_event(
                db,
                &crate::events::InsertEventParams {
                    actor_id: &actor,
                    session_id: &session,
                    event_type: "conversation",
                    role: Some("user"),
                    content: Some("hello"),
                    blob_data: None,
                    metadata: None,
                    branch_id: None,
                    expires_at: None,
                },
            )
            .unwrap()
        })
        .join()
        .unwrap();
        event.id
    }

    #[tokio::test]
    async fn test_tool_checkpoint_basic() {
        let server = make_server();
        let event_id = add_test_event(&server, "actor1", "session1");
        let result = server
            .run(move |mgr| {
                let db = mgr.db()?;
                let p = sessions::InsertCheckpointParams {
                    actor_id: "actor1",
                    session_id: "session1",
                    name: "my-checkpoint",
                    event_id: &event_id,
                    metadata: None,
                };
                sessions::create_checkpoint(db, &p)
                    .map(|cp| serde_json::json!({ "checkpoint": cp }))
            })
            .await;
        assert!(result.is_ok(), "unexpected error: {:?}", result);
        let v: serde_json::Value = serde_json::from_str(&result.unwrap()).unwrap();
        assert_eq!(v["checkpoint"]["name"], "my-checkpoint");
    }

    #[tokio::test]
    async fn test_tool_branch_basic() {
        let server = make_server();
        let event_id = add_test_event(&server, "actor1", "session1");
        let eid = event_id.clone();
        let result = server
            .run(move |mgr| {
                let db = mgr.db()?;
                let p = sessions::InsertBranchParams {
                    actor_id: "actor1",
                    session_id: "session1",
                    root_event_id: &eid,
                    name: None,
                    parent_branch_id: None,
                };
                sessions::create_branch(db, &p)
                    .map(|br| serde_json::json!({ "branch": br }))
            })
            .await;
        assert!(result.is_ok(), "unexpected error: {:?}", result);
        let v: serde_json::Value = serde_json::from_str(&result.unwrap()).unwrap();
        assert_eq!(v["branch"]["root_event_id"], event_id);
    }

    #[tokio::test]
    async fn test_tool_list_checkpoints_empty() {
        let server = make_server();
        let result = server
            .run(move |mgr| {
                let db = mgr.db()?;
                let p = sessions::ListCheckpointsParams {
                    actor_id: "actor1",
                    session_id: "session1",
                    limit: 100,
                    offset: 0,
                };
                sessions::list_checkpoints(db, &p)
                    .map(|cps| serde_json::json!({ "checkpoints": cps }))
            })
            .await;
        assert!(result.is_ok());
        let v: serde_json::Value = serde_json::from_str(&result.unwrap()).unwrap();
        assert_eq!(v["checkpoints"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn test_tool_list_branches_empty() {
        let server = make_server();
        let result = server
            .run(move |mgr| {
                let db = mgr.db()?;
                let p = sessions::ListBranchesParams {
                    actor_id: "actor1",
                    session_id: "session1",
                    limit: 100,
                    offset: 0,
                };
                sessions::list_branches(db, &p)
                    .map(|brs| serde_json::json!({ "branches": brs }))
            })
            .await;
        assert!(result.is_ok());
        let v: serde_json::Value = serde_json::from_str(&result.unwrap()).unwrap();
        assert_eq!(v["branches"], serde_json::json!([]));
    }
}
