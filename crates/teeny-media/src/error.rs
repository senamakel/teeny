//! The one error this crate returns.

/// Why a render, a request or a file operation did not produce media.
///
/// `Failed` carries the same sentence the TypeScript service threw, so a log
/// line or a tool answer reads the same after the port.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A failure described in words: a refused URL, a provider's error, a
    /// render that had nothing to draw.
    #[error("{0}")]
    Failed(String),
    /// A file the store, the chart or the reference sheet reads or writes.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// The request never got an answer: a timeout, a refused connection.
    #[error(transparent)]
    Http(#[from] reqwest::Error),
}

impl Error {
    /// A failure with this message.
    pub fn failed(message: impl Into<String>) -> Self {
        Self::Failed(message.into())
    }
}

/// `Result` with this crate's [`Error`].
pub type Result<T, E = Error> = std::result::Result<T, E>;
