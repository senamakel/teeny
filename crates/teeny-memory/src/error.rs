//! What a memory call can fail with.

/// A memory store refused a call, or could not be reached.
#[derive(Debug, thiserror::Error)]
pub enum MemoryError {
    /// The request never got an answer: DNS, TLS, a timeout.
    #[error("cortexdb request failed: {0}")]
    Transport(#[from] reqwest::Error),
    /// CortexDB answered with an error envelope.
    #[error("[{status} {code}] {message}")]
    Api {
        status: u16,
        /// The envelope's `error_code`, `UNKNOWN` when there was none.
        code: String,
        message: String,
    },
    /// A successful answer that did not read as one.
    #[error("cortexdb answered with an unexpected body: {0}")]
    Decode(#[from] serde_json::Error),
    /// A tool was called with input its schema refuses.
    #[error("{0}")]
    InvalidInput(String),
    /// The call outlived its own deadline.
    #[error("request timed out after {after_ms}ms")]
    Timeout { after_ms: u128 },
    /// The shared memory store refused a call or failed it.
    #[error("memory store: {0}")]
    Store(String),
    /// Recall is off for now; nothing was sent.
    #[error(transparent)]
    Paused(#[from] crate::RecallPaused),
}

impl MemoryError {
    /// The call gave up at its deadline, its own or the transport's. A
    /// recall that did leaves a query running on the store.
    pub fn is_timeout(&self) -> bool {
        match self {
            Self::Timeout { .. } => true,
            Self::Transport(error) => error.is_timeout(),
            _ => false,
        }
    }

    /// CortexDB's answer to a key reused with a different body. The record
    /// is already stored, which is not a failure.
    pub fn is_idempotency_conflict(&self) -> bool {
        matches!(self, Self::Api { code, message, .. }
            if code == "IDEMPOTENCY_CONFLICT" || message.contains("IDEMPOTENCY_CONFLICT"))
    }
}

/// `Result` with [`MemoryError`].
pub type Result<T, E = MemoryError> = std::result::Result<T, E>;
