//! What can go wrong talking to X.

/// A failed X call.
#[derive(Debug, thiserror::Error)]
pub enum XError {
    /// X answered with an error status. The body is cut to 300 characters.
    #[error("X answered {status}: {body}")]
    Http { status: u16, body: String },
    /// X could not be reached, or the connection broke.
    #[error("could not reach X: {0}")]
    Transport(#[from] reqwest::Error),
    /// X answered 2xx with something that is not the expected JSON.
    #[error("X sent something unreadable: {0}")]
    Decode(String),
    /// X answered, but without the one thing the call is for (a post id, a
    /// media id, the account's identity).
    #[error("{0}")]
    Missing(String),
    /// X took the upload and then could not process it.
    #[error("X failed to process media {0}")]
    MediaFailed(String),
    /// X refused an Article; it gates them on Premium+.
    #[error("X refused the article ({status}): {body}")]
    ArticleRefused { status: u16, body: String },
    /// The error of a read another caller was already making; the cache
    /// shares one request between callers asking the same thing at once.
    #[error("{0}")]
    Shared(std::sync::Arc<XError>),
}

/// `Result` with [`XError`].
pub type XResult<T> = Result<T, XError>;
