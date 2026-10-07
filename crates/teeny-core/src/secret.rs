//! A value that must never reach a log line.

use std::fmt;

/// A credential read from the environment: an API key, a token, a secret.
///
/// `Debug` prints a placeholder and there is no `Display`, so a `{:?}` of the
/// whole [`Config`](crate::Config) is safe to log. Read the value with
/// [`Secret::expose`] at the one place that sends it.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    /// Wraps a credential.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The credential itself, for the request that needs it.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}
