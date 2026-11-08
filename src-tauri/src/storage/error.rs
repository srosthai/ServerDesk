#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("migration failed: {0}")]
    Migration(String),

    #[error("storage lock poisoned")]
    LockPoisoned,

    #[error("record not found")]
    NotFound,

    #[error("conflicting record: {0}")]
    Conflict(String),

    #[error("invalid stored value: {0}")]
    InvalidValue(String),
}
