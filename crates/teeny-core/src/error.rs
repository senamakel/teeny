//! The errors every Teeny crate can surface.

use std::fmt;

/// One configuration value that could not be read.
///
/// The message describes what was expected and never repeats the value: the
/// value may be a credential, and this ends up in the boot log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigIssue {
    /// The environment variable, e.g. `X_API_KEY`.
    pub key: String,
    /// What is wrong with it, e.g. `is required`.
    pub message: String,
}

/// The configuration was refused. Lists every bad value, not just the first,
/// so one failed boot names everything that needs fixing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    issues: Vec<ConfigIssue>,
}

impl ConfigError {
    /// An error carrying these issues.
    pub fn new(issues: Vec<ConfigIssue>) -> Self {
        Self { issues }
    }

    /// Every value that was refused, in the order the schema reads them.
    pub fn issues(&self) -> &[ConfigIssue] {
        &self.issues
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid configuration")?;
        for issue in &self.issues {
            write!(f, "\n  {}: {}", issue.key, issue.message)?;
        }
        Ok(())
    }
}

impl std::error::Error for ConfigError {}

/// The shared error type.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The environment did not describe a runnable service.
    #[error(transparent)]
    Config(#[from] ConfigError),
    /// A file the service reads or writes.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// `Result` with the shared [`Error`].
pub type Result<T, E = Error> = std::result::Result<T, E>;
