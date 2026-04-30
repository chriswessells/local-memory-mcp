use std::sync::{Arc, Mutex};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::handler::server::ServerHandler;
use rmcp::model::{Implementation, ServerCapabilities, ServerInfo};
use rmcp::{schemars, tool, tool_handler, tool_router};
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

// WARNING: sent verbatim to the LLM on every connection — do NOT interpolate runtime values.
const SERVER_INSTRUCTIONS: &str = "\
local-memory-mcp gives you three layers of persistent, queryable memory:

EVENTS — immutable conversation turns. Use memory.create_event to record each message. \
Use memory.list_events to retrieve a session's history. Use memory.get_event to fetch one by ID.

MEMORIES — long-term records extracted from events. Use memory.create_memory_record to save an \
insight or preference. Use memory.retrieve_memory_records to search by keyword or semantic \
similarity. Use memory.list_memory_records to enumerate memories by namespace. \
Use memory.get_memory_record to fetch one by ID.

KNOWLEDGE GRAPH — typed edges between memories. Use graph.create_edge to link two memories. \
Use graph.traverse to walk the graph from a starting memory.

actor_id: Every tool requires actor_id except the store.* tools, which operate globally. \
In single-user deployments, pass a constant like \"default\". In multi-user deployments, use a \
stable per-user identifier (e.g., UUID or opaque per-user token). NEVER share actor_id across \
users — it scopes all data access. The store.* tools (store.switch, store.current, store.list, \
store.delete) do not require actor_id.

namespace: A slash-separated path grouping related memories, e.g. \"/user/alice/preferences\" \
or \"/project/myapp/decisions\". All memories in a namespace are deleted together with \
memory.delete_namespace.

strategy: A free-form label describing how a memory was produced. Suggested values: \
\"summarization\", \"user_preference\", \"semantic\", \"verbatim\", \"extraction\".

embedding: The server does NOT compute embeddings. Pass a caller-computed float array \
(384 dims, matching all-MiniLM-L6-v2 or compatible model) to enable vector search; \
omit it to use FTS-only keyword search.

metadata: A JSON object string, e.g. '{\"source\":\"user\",\"confidence\":0.9}'. \
Stored as-is; filter on it via memory.list_memory_records.

Intent guide:
- Record a conversation turn           → memory.create_event
- Save an extracted insight            → memory.create_memory_record
- Search memories by keyword or meaning→ memory.retrieve_memory_records
- Enumerate memories by namespace      → memory.list_memory_records
- Fetch one memory by ID               → memory.get_memory_record
- Link two memories in graph           → graph.create_edge
- Walk the knowledge graph             → graph.traverse";

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
pub struct CreateEventParams {
    #[schemars(
        description = "Stable identifier scoping all data access for one user or agent. See server instructions for how to choose."
    )]
    actor_id: String,
    #[schemars(
        description = "Identifies the conversation session. Use a stable per-conversation UUID."
    )]
    session_id: String,
    event_type: EventType,
    #[serde(default)]
    role: Option<Role>,
    #[serde(default)]
    content: Option<String>,
    #[schemars(description = "Base64-encoded binary data. Required when event_type is 'blob'.")]
    #[serde(default)]
    blob_data: Option<String>,
    #[schemars(description = r#"JSON object string, e.g. '{"source":"user"}'. Stored as-is."#)]
    #[serde(default)]
    metadata: Option<String>,
    #[serde(default)]
    branch_id: Option<String>,
    #[schemars(
        description = "ISO 8601 UTC timestamp for event expiry, e.g. '2025-01-15T14:30:00Z'. Optional."
    )]
    #[serde(default)]
    expires_at: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetEventParams {
    #[schemars(
        description = "Stable identifier scoping all data access for one user or agent. See server instructions for how to choose."
    )]
    actor_id: String,
    #[schemars(
        description = "UUID of the event, returned by memory.create_event or memory.list_events."
    )]
    event_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListEventsParams {
    #[schemars(
        description = "Stable identifier scoping all data access for one user or agent. See server instructions for how to choose."
    )]
    actor_id: String,
    #[schemars(
        description = "Identifies the conversation session. Use a stable per-conversation UUID."
    )]
    session_id: String,
    #[schemars(
        description = "Branch filter: 'all' (default), 'main' (main timeline only), or a specific branch ID."
    )]
    #[serde(default)]
    branch_filter: Option<String>,
    #[serde(default)]
    limit: Option<u32>,
    #[serde(default)]
    offset: Option<u32>,
    #[schemars(
        description = "ISO 8601 UTC timestamp to bound the event query window, e.g. '2025-01-15T14:30:00Z'."
    )]
    #[serde(default)]
    before: Option<String>,
    #[schemars(
        description = "ISO 8601 UTC timestamp to bound the event query window, e.g. '2025-01-15T14:30:00Z'."
    )]
    #[serde(default)]
    after: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListSessionsParams {
    #[schemars(
        description = "Stable identifier scoping all data access for one user or agent. See server instructions for how to choose."
    )]
    actor_id: String,
    #[serde(default)]
    limit: Option<u32>,
    #[serde(default)]
    offset: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateMemoryRecordParams {
    #[schemars(
        description = "Stable identifier scoping all data access for one user or agent. See server instructions for how to choose."
    )]
    actor_id: String,
    content: String,
    #[schemars(
        description = "Free-form label for how this memory was produced. Suggested: 'summarization', 'user_preference', 'semantic', 'verbatim', 'extraction'."
    )]
    strategy: String,
    #[schemars(
        description = "Slash-separated path grouping related memories, e.g. '/user/alice/preferences'."
    )]
    #[serde(default)]
    namespace: Option<String>,
    #[schemars(description = r#"JSON object string, e.g. '{"source":"user"}'. Stored as-is."#)]
    #[serde(default)]
    metadata: Option<String>,
    #[serde(default)]
    source_session_id: Option<String>,
    #[schemars(
        description = "Caller-computed float array (384 dims). Omit for FTS-only search. Server does not generate embeddings."
    )]
    #[schemars(extend("minItems" = 384, "maxItems" = 384))]
    #[serde(default)]
    embedding: Option<Vec<f32>>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetMemoryRecordParams {
    #[schemars(
        description = "Stable identifier scoping all data access for one user or agent. See server instructions for how to choose."
    )]
    actor_id: String,
    #[schemars(
        description = "UUID of the memory record, returned by memory.create_memory_record or memory.list_memory_records."
    )]
    memory_record_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListMemoryRecordsParams {
    #[schemars(
        description = "Stable identifier scoping all data access for one user or agent. See server instructions for how to choose."
    )]
    actor_id: String,
    #[schemars(
        description = "Slash-separated path grouping related memories, e.g. '/user/alice/preferences'."
    )]
    #[serde(default)]
    namespace: Option<String>,
    #[schemars(
        description = "Namespace prefix to filter results. Returns memories whose namespace starts with this value."
    )]
    #[serde(default)]
    namespace_prefix: Option<String>,
    #[schemars(
        description = "Free-form label for how this memory was produced. Suggested: 'summarization', 'user_preference', 'semantic', 'verbatim', 'extraction'."
    )]
    #[serde(default)]
    strategy: Option<String>,
    #[schemars(
        description = "Filter to valid (non-consolidated) memories only. Default: true. Pass false to include superseded memories."
    )]
    #[serde(default = "default_true")]
    valid_only: bool,
    #[serde(default)]
    limit: Option<u32>,
    #[serde(default)]
    offset: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct UpdateMemoryRecordParams {
    #[schemars(
        description = "Stable identifier scoping all data access for one user or agent. See server instructions for how to choose."
    )]
    actor_id: String,
    #[schemars(
        description = "UUID of the memory record, returned by memory.create_memory_record or memory.list_memory_records."
    )]
    memory_record_id: String,
    #[schemars(
        description = "Action to perform: 'update' (replaces content, requires new_content) or 'invalidate' (marks the record superseded with no replacement)."
    )]
    action: ConsolidateActionType,
    #[schemars(description = "Replacement content. Required when action is 'update'.")]
    #[serde(default)]
    new_content: Option<String>,
    #[schemars(
        description = "Caller-computed float array (384 dims). Omit for FTS-only search. Server does not generate embeddings."
    )]
    #[schemars(extend("minItems" = 384, "maxItems" = 384))]
    #[serde(default)]
    new_embedding: Option<Vec<f32>>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DeleteMemoryRecordParams {
    #[schemars(
        description = "Stable identifier scoping all data access for one user or agent. See server instructions for how to choose."
    )]
    actor_id: String,
    #[schemars(
        description = "UUID of the memory record, returned by memory.create_memory_record or memory.list_memory_records."
    )]
    memory_record_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RetrieveMemoryRecordsParams {
    #[schemars(
        description = "Stable identifier scoping all data access for one user or agent. See server instructions for how to choose."
    )]
    actor_id: String,
    #[schemars(
        description = "Keyword query for full-text search. At least one of search_query or embedding must be provided."
    )]
    #[serde(default)]
    search_query: Option<String>,
    #[schemars(
        description = "Caller-computed float array (384 dims). Omit for FTS-only search. Server does not generate embeddings."
    )]
    #[schemars(extend("minItems" = 384, "maxItems" = 384))]
    #[serde(default)]
    embedding: Option<Vec<f32>>,
    #[schemars(
        description = "Slash-separated path grouping related memories, e.g. '/user/alice/preferences'."
    )]
    #[serde(default)]
    namespace: Option<String>,
    #[schemars(
        description = "Namespace prefix to filter results. Returns memories whose namespace starts with this value."
    )]
    #[serde(default)]
    namespace_prefix: Option<String>,
    #[schemars(
        description = "Free-form label for how this memory was produced. Suggested: 'summarization', 'user_preference', 'semantic', 'verbatim', 'extraction'."
    )]
    #[serde(default)]
    strategy: Option<String>,
    #[schemars(
        description = "Maximum number of results to return. Corresponds to AgentCore topK. Default: 10."
    )]
    #[serde(default)]
    top_k: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SwitchStoreParams {
    #[schemars(
        description = "Store name: 1–64 alphanumeric characters plus hyphens and underscores (e.g. 'project-alpha')."
    )]
    name: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DeleteStoreParams {
    #[schemars(description = "Name of the store to delete. Cannot be the currently active store.")]
    name: String,
}

// -- Graph param structs --

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateEdgeParams {
    #[schemars(
        description = "Stable identifier scoping all data access for one user or agent. See server instructions for how to choose."
    )]
    actor_id: String,
    #[schemars(
        description = "UUID of the source memory record, returned by memory.create_memory_record."
    )]
    from_memory_record_id: String,
    #[schemars(
        description = "UUID of the target memory record, returned by memory.create_memory_record."
    )]
    to_memory_record_id: String,
    label: String,
    #[schemars(
        description = r#"JSON object string of edge properties, e.g. '{"weight":0.9}'. Optional."#
    )]
    #[serde(default)]
    properties: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetNeighborsParams {
    #[schemars(
        description = "Stable identifier scoping all data access for one user or agent. See server instructions for how to choose."
    )]
    actor_id: String,
    #[schemars(
        description = "UUID of the memory record, returned by memory.create_memory_record or memory.list_memory_records."
    )]
    memory_record_id: String,
    #[schemars(description = "Direction: 'out' (default), 'in', or 'both'.")]
    #[serde(default)]
    direction: Option<Direction>,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TraverseParams {
    #[schemars(
        description = "Stable identifier scoping all data access for one user or agent. See server instructions for how to choose."
    )]
    actor_id: String,
    #[schemars(
        description = "UUID of the memory record to start traversal from, returned by memory.create_memory_record or memory.list_memory_records."
    )]
    start_memory_record_id: String,
    #[schemars(description = "Maximum traversal depth. Default: 2, max: 5.")]
    #[serde(default)]
    max_depth: Option<u32>,
    #[serde(default)]
    label: Option<String>,
    #[schemars(description = "Direction: 'out' (default), 'in', or 'both'.")]
    #[serde(default)]
    direction: Option<Direction>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct UpdateEdgeToolParams {
    #[schemars(
        description = "Stable identifier scoping all data access for one user or agent. See server instructions for how to choose."
    )]
    actor_id: String,
    #[schemars(
        description = "UUID of the edge, returned by graph.create_edge or graph.get_neighbors."
    )]
    edge_id: String,
    #[serde(default)]
    label: Option<String>,
    #[schemars(
        description = r#"JSON object string of edge properties, e.g. '{"weight":0.9}'. Optional."#
    )]
    #[serde(default)]
    properties: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DeleteEdgeParams {
    #[schemars(
        description = "Stable identifier scoping all data access for one user or agent. See server instructions for how to choose."
    )]
    actor_id: String,
    #[schemars(
        description = "UUID of the edge, returned by graph.create_edge or graph.get_neighbors."
    )]
    edge_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListLabelsParams {
    #[schemars(
        description = "Stable identifier scoping all data access for one user or agent. See server instructions for how to choose."
    )]
    actor_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetStatsParams {
    #[schemars(
        description = "Stable identifier scoping all data access for one user or agent. See server instructions for how to choose."
    )]
    actor_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateNamespaceToolParams {
    #[schemars(
        description = "Namespace path, e.g. '/user/alice/preferences'. Up to 512 bytes (UTF-8). Must not contain control characters."
    )]
    #[schemars(length(max = 512))]
    name: String,
    #[serde(default)]
    #[schemars(length(max = 1024))]
    description: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListNamespacesToolParams {
    #[schemars(
        description = "If provided, return only namespaces whose name starts with this prefix."
    )]
    #[serde(default)]
    prefix: Option<String>,
    #[serde(default)]
    limit: Option<u32>,
    #[serde(default)]
    offset: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DeleteNamespaceToolParams {
    #[schemars(
        description = "Stable identifier scoping all data access for one user or agent. See server instructions for how to choose."
    )]
    actor_id: String,
    name: String,
}

// -- Session param structs (Component 6) --

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateCheckpointParams {
    #[schemars(
        description = "Stable identifier scoping all data access for one user or agent. See server instructions for how to choose."
    )]
    actor_id: String,
    #[schemars(
        description = "Identifies the conversation session. Use a stable per-conversation UUID."
    )]
    session_id: String,
    #[schemars(
        description = "Short label for this checkpoint, e.g. 'before-refactor'. Max 64 chars."
    )]
    name: String,
    #[schemars(
        description = "UUID of the event, returned by memory.create_event or memory.list_events."
    )]
    event_id: String,
    #[schemars(description = r#"JSON object string, e.g. '{"source":"user"}'. Stored as-is."#)]
    #[serde(default)]
    metadata: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateBranchParams {
    #[schemars(
        description = "Stable identifier scoping all data access for one user or agent. See server instructions for how to choose."
    )]
    actor_id: String,
    #[schemars(
        description = "Identifies the conversation session. Use a stable per-conversation UUID."
    )]
    session_id: String,
    #[schemars(
        description = "UUID of the event to fork from, returned by memory.create_event or memory.list_events."
    )]
    root_event_id: String,
    #[schemars(description = "Name of the new branch, e.g. 'experiment-a'. Max 64 chars.")]
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    parent_branch_id: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListCheckpointsToolParams {
    #[schemars(
        description = "Stable identifier scoping all data access for one user or agent. See server instructions for how to choose."
    )]
    actor_id: String,
    #[schemars(
        description = "Identifies the conversation session. Use a stable per-conversation UUID."
    )]
    session_id: String,
    #[serde(default)]
    limit: Option<u32>,
    #[serde(default)]
    offset: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListBranchesToolParams {
    #[schemars(
        description = "Stable identifier scoping all data access for one user or agent. See server instructions for how to choose."
    )]
    actor_id: String,
    #[schemars(
        description = "Identifies the conversation session. Use a stable per-conversation UUID."
    )]
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

#[tool_router]
impl MemoryServer {
    #[tool(
        name = "memory.create_event",
        description = "Append an immutable event to a session timeline. Use this for raw \
                       conversation turns and binary blobs that the agent should later extract \
                       insights from; use memory.create_memory_record directly for \
                       already-extracted insights. Event type must be 'conversation' (requires \
                       content) or 'blob' (requires base64-encoded blob_data). Returns the full \
                       event with its generated ID and created_at timestamp. \
                       (AgentCore equivalent: CreateEvent.)",
        annotations(title = "Create event")
    )]
    pub async fn create_event(
        &self,
        Parameters(params): Parameters<CreateEventParams>,
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
        description = "Fetch a single event by its UUID. Use this when you have an event_id \
                       and need the full event object; use memory.list_events instead for all \
                       events in a session. Returns the full event object including blob_data \
                       (base64-encoded) if present. (AgentCore equivalent: GetEvent.)",
        annotations(title = "Get event", read_only_hint = true)
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
        name = "memory.list_events",
        description = "List events for an actor + session, ordered chronologically (oldest \
                       first). Use this for windowed reads of a conversation timeline; use \
                       memory.get_event for a single event by ID, and memory.list_sessions to \
                       enumerate the sessions belonging to an actor. Supports branch filter, \
                       time range, and limit/offset pagination. \
                       (AgentCore equivalent: ListEvents.)",
        annotations(title = "List session events", read_only_hint = true)
    )]
    pub async fn list_events(
        &self,
        Parameters(params): Parameters<ListEventsParams>,
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
        description = "Enumerate all sessions for an actor, ordered by last event time \
                       descending. Use this to discover session IDs; use memory.list_events \
                       for events within a session. Returns session summaries with event \
                       counts and date ranges. (AgentCore equivalent: ListSessions.)",
        annotations(title = "List sessions", read_only_hint = true)
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
        name = "memory.delete_expired_events",
        description = "Delete all events whose expires_at timestamp is in the past. Use this \
                       as a periodic maintenance operation to reclaim storage. No parameters \
                       required. Returns a JSON object with the count of deleted events. \
                       (Local-only extension: AgentCore has no automatic-TTL-deletion op.)",
        annotations(title = "Delete expired events", destructive_hint = true)
    )]
    pub async fn delete_expired_events(&self) -> Result<String, String> {
        self.run(|mgr| {
            let db = mgr.db()?;
            let count = events::delete_expired(db)?;
            Ok(serde_json::json!({ "deleted": count }))
        })
        .await
    }

    #[tool(
        name = "memory.create_memory_record",
        description = "Create a long-term memory record for an actor. Use this when the agent \
                       has extracted an insight worth retaining beyond the current session; use \
                       memory.create_event for raw conversation turns instead. The optional \
                       384-dim embedding is caller-computed — the server does not generate \
                       embeddings. Returns the created record with its generated ID. \
                       (AgentCore equivalent: CreateMemoryRecord / BatchCreateMemoryRecords.)",
        annotations(title = "Create memory record")
    )]
    pub async fn create_memory_record(
        &self,
        Parameters(params): Parameters<CreateMemoryRecordParams>,
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
        name = "memory.get_memory_record",
        description = "Fetch a single memory record by its UUID. Use this when you have a \
                       memory_record_id; use memory.retrieve_memory_records to search by \
                       content, or memory.list_memory_records to enumerate by namespace or \
                       strategy. Returns the full record including content, strategy, \
                       namespace, and validity. (AgentCore equivalent: GetMemoryRecord.)",
        annotations(title = "Get memory record", read_only_hint = true)
    )]
    pub async fn get_memory_record(
        &self,
        Parameters(params): Parameters<GetMemoryRecordParams>,
    ) -> Result<String, String> {
        self.run(move |mgr| {
            let db = mgr.db()?;
            memories::get_memory(db, &params.actor_id, &params.memory_record_id)
        })
        .await
    }

    #[tool(
        name = "memory.list_memory_records",
        description = "Enumerate memory records with optional filters (namespace, \
                       namespace_prefix, strategy, valid_only). Use this for filtered \
                       enumeration when you need all matching records in insertion order; use \
                       memory.retrieve_memory_records for ranked search by keyword or meaning \
                       instead. Results ordered by created_at descending; supports limit/offset \
                       pagination. (AgentCore equivalent: ListMemoryRecords.)",
        annotations(title = "List memory records", read_only_hint = true)
    )]
    pub async fn list_memory_records(
        &self,
        Parameters(params): Parameters<ListMemoryRecordsParams>,
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
        name = "memory.update_memory_record",
        description = "Update or invalidate a memory record. Use this to supersede an \
                       outdated record with new content (action 'update' — creates a \
                       replacement and marks the old one invalid) or to retire a record \
                       entirely (action 'invalidate' — no replacement). Use \
                       memory.delete_memory_record instead if you want hard deletion with \
                       no audit trail. The immutable audit trail of superseded records \
                       preserves history for replay. (AgentCore equivalent: closest match is \
                       BatchUpdateMemoryRecords with our additional invalidation semantics.)",
        annotations(title = "Update memory record")
    )]
    pub async fn update_memory_record(
        &self,
        Parameters(params): Parameters<UpdateMemoryRecordParams>,
    ) -> Result<String, String> {
        self.run(move |mgr| {
            let db = mgr.db()?;
            let action = parse_consolidate_action(
                &params.action,
                params.new_content.as_deref(),
                params.new_embedding.as_deref(),
            )?;
            memories::consolidate_memory(db, &params.actor_id, &params.memory_record_id, &action)
        })
        .await
    }

    #[tool(
        name = "memory.delete_memory_record",
        description = "Hard-delete a memory record and its embedding permanently. No recovery \
                       is possible. Use this only when data must be removed entirely; use \
                       memory.update_memory_record instead to supersede a record while keeping \
                       an audit trail. Returns {\"deleted\": true} on success. \
                       (AgentCore equivalent: DeleteMemoryRecord.)",
        annotations(title = "Delete memory record", destructive_hint = true)
    )]
    pub async fn delete_memory_record(
        &self,
        Parameters(params): Parameters<DeleteMemoryRecordParams>,
    ) -> Result<String, String> {
        self.run(move |mgr| {
            let db = mgr.db()?;
            memories::delete_memory(db, &params.actor_id, &params.memory_record_id)?;
            Ok(serde_json::json!({ "deleted": true }))
        })
        .await
    }

    #[tool(
        name = "memory.retrieve_memory_records",
        description = "Search memory records for an actor by text query, embedding vector, or \
                       hybrid (Reciprocal Rank Fusion). Use this when you have a query and want \
                       relevance-ranked results; use memory.list_memory_records instead for \
                       filtered enumeration. At least one of search_query or embedding must be \
                       provided. Embeddings are caller-computed 384-dim float32 vectors — the \
                       server does not generate them. Returns ranked memory records with scores; \
                       scores are not comparable across modes. \
                       (AgentCore equivalent: RetrieveMemoryRecords.)",
        annotations(title = "Retrieve memory records", read_only_hint = true)
    )]
    pub async fn retrieve_memory_records(
        &self,
        Parameters(params): Parameters<RetrieveMemoryRecordsParams>,
    ) -> Result<String, String> {
        self.run(move |mgr| {
            let db = mgr.db()?;
            let p = RecallParams {
                actor_id: &params.actor_id,
                query: params.search_query.as_deref(),
                embedding: params.embedding.as_deref(),
                namespace: params.namespace.as_deref(),
                namespace_prefix: params.namespace_prefix.as_deref(),
                strategy: params.strategy.as_deref(),
                limit: params.top_k.unwrap_or(10),
            };
            search::recall(db, &p)
        })
        .await
    }

    #[tool(
        name = "store.switch",
        description = "Switch the active SQLite store, creating it if it does not exist. Use \
                       this to isolate memory across projects, environments, or tenants — each \
                       store is a separate .db file. The previously active store is \
                       checkpointed and closed before the switch. Store names must be 1–64 \
                       alphanumeric characters (plus hyphens/underscores). Idempotent — \
                       switching to the already-active store is a no-op. Returns the new active \
                       store name. Does not require actor_id. \
                       (Local-only extension: AgentCore manages a single memory resource per \
                       agent.)",
        annotations(title = "Switch store", idempotent_hint = true)
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
        name = "store.current",
        description = "Return the name of the currently active store. Returns a JSON object \
                       with the store name, or null if no store is open. Does not require \
                       actor_id. (Local-only extension.)",
        annotations(title = "Current store", read_only_hint = true)
    )]
    pub async fn current_store(&self) -> Result<String, String> {
        self.run(|mgr| {
            let name = mgr.active_name().map(|s| s.to_string());
            Ok(serde_json::json!({ "store": name }))
        })
        .await
    }

    #[tool(
        name = "store.list",
        description = "List all stores in the base directory with their names and sizes in \
                       bytes. Returns an array of store info objects sorted alphabetically by \
                       name. Does not require actor_id. (Local-only extension.)",
        annotations(title = "List stores", read_only_hint = true)
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
        name = "store.delete",
        description = "Delete a named store and its auxiliary files. Cannot delete the \
                       currently active store. Returns {\"deleted\": true} on success, or an \
                       error if the store is active or not found. Does not require actor_id. \
                       (Local-only extension.)",
        annotations(title = "Delete store", destructive_hint = true)
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
        description = "Register a namespace with optional description. Idempotent — if the \
                       namespace already exists, returns the existing entry unchanged. Namespace \
                       names are UTF-8 strings up to 512 bytes, e.g. '/user/alice/preferences'. \
                       Must not contain control characters. Use memory.delete_namespace to \
                       bulk-delete all memories in a namespace. (Local-only extension.)",
        annotations(title = "Create namespace", idempotent_hint = true)
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
        description = "List registered namespaces ordered alphabetically. Only namespaces \
                       explicitly created via memory.create_namespace are returned — not all \
                       namespaces referenced by memories. Supports optional prefix filter and \
                       limit/offset pagination. (Local-only extension.)",
        annotations(title = "List namespaces", read_only_hint = true)
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
        description = "Delete all memories belonging to actor_id in the named namespace, clean \
                       up their vector rows, and remove the namespace registry entry. Scoped to \
                       actor_id — other actors' memories in the same namespace path are not \
                       affected. Deletes in chunks to avoid blocking. Returns not_found if the \
                       namespace is not registered. (Local-only extension.)",
        annotations(title = "Delete namespace", destructive_hint = true)
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
        name = "graph.create_edge",
        description = "Create a directed, labeled edge between two memory records. Use this to \
                       record typed relationships ('supersedes', 'references', 'contradicts') \
                       between extracted insights for graph traversal. Both records must belong \
                       to the same actor; self-edges are rejected. Returns the full edge object \
                       with its generated ID. (Local-only extension: AgentCore Memory does not \
                       expose a graph layer.)",
        annotations(title = "Create edge")
    )]
    pub async fn create_edge(
        &self,
        Parameters(params): Parameters<CreateEdgeParams>,
    ) -> Result<String, String> {
        self.run(move |mgr| {
            let db = mgr.db()?;
            let p = GraphInsertEdgeParams {
                actor_id: &params.actor_id,
                from_memory_id: &params.from_memory_record_id,
                to_memory_id: &params.to_memory_record_id,
                label: &params.label,
                properties: params.properties.as_deref(),
            };
            graph::add_edge(db, &p)
        })
        .await
    }

    #[tool(
        name = "graph.get_neighbors",
        description = "Return the direct neighbors (one hop) of a memory record in the \
                       knowledge graph. Use this to find immediately connected records; use \
                       graph.traverse instead for multi-hop traversal. Direction: 'out' \
                       (default), 'in', or 'both'. Returns edges and connected memory records. \
                       (Local-only extension.)",
        annotations(title = "Get neighbors", read_only_hint = true)
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
                &params.memory_record_id,
                params.direction.unwrap_or_default(),
                params.label.as_deref(),
                params.limit.unwrap_or(graph::DEFAULT_NEIGHBOR_LIMIT),
            )
        })
        .await
    }

    #[tool(
        name = "graph.traverse",
        description = "Walk the knowledge graph from a starting memory record up to a given \
                       depth. Use this for multi-hop exploration of connected memories; use \
                       graph.get_neighbors instead for direct neighbors only (one hop). \
                       Direction: 'out' (default), 'in', or 'both'. Max depth 5. Returns \
                       visited memory records with depth and path. (Local-only extension.)",
        annotations(title = "Traverse graph", read_only_hint = true)
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
                &params.start_memory_record_id,
                params.max_depth.unwrap_or(graph::DEFAULT_TRAVERSE_DEPTH),
                params.label.as_deref(),
                params.direction.unwrap_or_default(),
            )
        })
        .await
    }

    #[tool(
        name = "graph.update_edge",
        description = "Update an edge's label and/or properties. At least one of label or \
                       properties must be provided. Use graph.create_edge to add new edges and \
                       graph.delete_edge to remove them. Returns the updated edge object. \
                       (Local-only extension.)",
        annotations(title = "Update edge")
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
        description = "Delete an edge by ID, scoped to the given actor. Returns \
                       {\"deleted\": true} on success. (Local-only extension.)",
        annotations(title = "Delete edge", destructive_hint = true)
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
        description = "List all distinct edge labels used by the given actor, with their \
                       occurrence counts, ordered by count descending. Use this to discover \
                       what relationship types exist in the graph before traversing. \
                       (Local-only extension.)",
        annotations(title = "List edge labels", read_only_hint = true)
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
        name = "graph.get_stats",
        description = "Get graph statistics for the given actor: total edge count, label \
                       distribution, and top 10 most-connected memory records. Use this to \
                       understand the shape of the knowledge graph before traversal. \
                       (Local-only extension.)",
        annotations(title = "Graph stats", read_only_hint = true)
    )]
    pub async fn get_stats(
        &self,
        Parameters(params): Parameters<GetStatsParams>,
    ) -> Result<String, String> {
        self.run(move |mgr| {
            let db = mgr.db()?;
            graph::graph_stats(db, &params.actor_id)
        })
        .await
    }

    // -- Session tools (Component 6) --

    #[tool(
        name = "memory.create_checkpoint",
        description = "Create a named checkpoint at a specific event within a session. \
                       Checkpoints are named snapshots used for workflow resumption and \
                       conversation bookmarks. Name must be unique per session. Use \
                       memory.list_checkpoints to enumerate existing checkpoints. Returns \
                       the created checkpoint object. (Local-only extension.)",
        annotations(title = "Create checkpoint")
    )]
    pub async fn create_checkpoint(
        &self,
        Parameters(params): Parameters<CreateCheckpointParams>,
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
            sessions::create_checkpoint(db, &p).map(|cp| serde_json::json!({ "checkpoint": cp }))
        })
        .await
    }

    #[tool(
        name = "memory.create_branch",
        description = "Fork a conversation by creating a branch from a specific event. \
                       Branches enable alternative conversation paths, message editing, and \
                       what-if scenarios. Returns the created branch object with its ID to use \
                       as branch_id in memory.create_event. Use memory.list_branches to \
                       enumerate branches. (Local-only extension.)",
        annotations(title = "Create branch")
    )]
    pub async fn create_branch(
        &self,
        Parameters(params): Parameters<CreateBranchParams>,
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
            sessions::create_branch(db, &p).map(|br| serde_json::json!({ "branch": br }))
        })
        .await
    }

    #[tool(
        name = "memory.list_checkpoints",
        description = "List all checkpoints for a session, ordered by creation time. Returns \
                       an array of checkpoint objects with names and event IDs. Use \
                       memory.create_checkpoint to add new checkpoints. (Local-only extension.)",
        annotations(title = "List checkpoints", read_only_hint = true)
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
            sessions::list_checkpoints(db, &p).map(|cps| serde_json::json!({ "checkpoints": cps }))
        })
        .await
    }

    #[tool(
        name = "memory.list_branches",
        description = "List branches for a session, ordered by creation time. Returns an array \
                       of branch objects including their root event IDs and optional names. Use \
                       the returned branch id as branch_id in memory.create_event. Use \
                       memory.create_branch to add new branches. (Local-only extension.)",
        annotations(title = "List branches", read_only_hint = true)
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
            sessions::list_branches(db, &p).map(|brs| serde_json::json!({ "branches": brs }))
        })
        .await
    }
}

#[tool_handler]
impl ServerHandler for MemoryServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new("local-memory-mcp", env!("CARGO_PKG_VERSION"))
                    .with_title("Local Memory")
                    .with_description(
                        "Local agent memory server: events, long-term memories, and knowledge graph over SQLite.",
                    ),
            )
            .with_instructions(SERVER_INSTRUCTIONS)
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
                sessions::create_branch(db, &p).map(|br| serde_json::json!({ "branch": br }))
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
                sessions::list_branches(db, &p).map(|brs| serde_json::json!({ "branches": brs }))
            })
            .await;
        assert!(result.is_ok());
        let v: serde_json::Value = serde_json::from_str(&result.unwrap()).unwrap();
        assert_eq!(v["branches"], serde_json::json!([]));
    }
}

#[cfg(test)]
mod discoverability_tests {
    use std::sync::{Arc, Mutex};

    use rmcp::handler::server::ServerHandler;
    use tempfile::TempDir;

    use super::{MemoryServer, SERVER_INSTRUCTIONS};
    use crate::store::StoreManager;

    #[test]
    fn server_instructions_contains_required_vocabulary() {
        for keyword in &["actor_id", "namespace", "strategy", "embedding"] {
            assert!(
                SERVER_INSTRUCTIONS.contains(keyword),
                "SERVER_INSTRUCTIONS missing keyword: {keyword}"
            );
        }
    }

    #[test]
    fn get_info_returns_correct_identity_and_instructions() {
        let dir = TempDir::new().unwrap();
        let mut mgr = StoreManager::with_base_dir(dir.path().to_path_buf()).unwrap();
        mgr.open_default().unwrap();
        let server = MemoryServer::new(Arc::new(Mutex::new(mgr)));

        let info = server.get_info();
        assert_eq!(info.server_info.name, "local-memory-mcp");
        let instructions = info
            .instructions
            .expect("get_info() must return non-None instructions");
        assert!(
            instructions.contains("actor_id"),
            "instructions must contain 'actor_id'"
        );
    }
}
