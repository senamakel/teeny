//! What a repository call can fail with.

use mongodb::bson;

/// A store that could not be read or written.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// Mongo refused the call or could not be reached.
    #[error(transparent)]
    Mongo(#[from] mongodb::error::Error),
    /// A row could not be turned into a document.
    #[error(transparent)]
    Encode(#[from] bson::ser::Error),
    /// A document did not read back as the row it should be.
    #[error(transparent)]
    Decode(#[from] bson::de::Error),
    /// A file the store reads or writes.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// A file whose JSON did not parse.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

/// `Result` with [`StoreError`].
pub type Result<T, E = StoreError> = std::result::Result<T, E>;
