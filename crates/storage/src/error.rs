use thiserror::Error;

/// Storage error type covering all backend failure modes.
#[derive(Debug, Clone, Error)]
pub enum StorageError {
    /// A generic backend operation failed.
    #[error("backend error: {0}")]
    Backend(String),

    /// The requested resource was not found.
    #[error("not found: {0}")]
    NotFound(String),

    /// A unique constraint or primary key conflict occurred.
    #[error("duplicate: {0}")]
    Duplicate(String),

    /// Failed to connect to the storage backend.
    #[error("connection error: {0}")]
    Connection(String),

    /// Schema migration failed.
    #[error("migration error: {0}")]
    Migration(String),
}
