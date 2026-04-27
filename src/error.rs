use thiserror::Error;

#[derive(Debug, Error)]
pub enum MemoryError {
    #[error("Invalid store name: {0}")]
    InvalidName(String),
    #[error("Store not found: {0}")]
    NotFound(String),
    #[error("Cannot delete active store: {0}")]
    ActiveStoreDeletion(String),
    #[error("Store disconnected")]
    Disconnected,
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),
    #[error("Schema error: {0}")]
    SchemaError(String),
    #[error("Schema version {0} is newer than this binary supports ({1})")]
    SchemaVersionTooNew(u32, u32),
    #[error("Invalid path: {0}")]
    InvalidPath(String),
    #[error("Query failed: {0}")]
    QueryFailed(String),
    #[error("Invalid input: {0}")]
    InvalidInput(String),
    #[error("Delete failed: {0}")]
    DeleteFailed(String),
    #[error("Database corrupted: {0}")]
    DatabaseCorrupted(String),
    #[error("Store is locked by another process: {0}")]
    StoreLocked(String),
}
