//! What can go wrong reading the market.

/// A market call that did not produce data.
///
/// Messages keep the TypeScript's wording: several of them reach the model
/// verbatim ("Holder data is unavailable right now: ...").
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MarketError {
    /// The provider answered with an error status.
    #[error("{url} answered {status}: {body}{}", retry_after.as_ref().map(|after| format!(" (retry-after: {after})")).unwrap_or_default())]
    Status {
        url: String,
        status: u16,
        /// The first 300 characters of the body.
        body: String,
        /// The `Retry-After` header, when there was one.
        retry_after: Option<String>,
    },
    /// No answer inside the deadline.
    #[error("{url} timed out")]
    Timeout { url: String },
    /// The connection failed, or the body was not the JSON expected.
    #[error("{url} failed: {message}")]
    Transport { url: String, message: String },
    /// The provider answered, but not with what was asked for, or there is
    /// not enough data to say anything. The message is the whole story.
    #[error("{0}")]
    Data(String),
}

impl MarketError {
    /// A [`MarketError::Data`] with this message.
    pub fn data(message: impl Into<String>) -> Self {
        Self::Data(message.into())
    }
}

/// `Result` with [`MarketError`].
pub type Result<T, E = MarketError> = std::result::Result<T, E>;
