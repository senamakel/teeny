//! Teeny's core: the configuration (the legacy env surface, typed), the
//! shared types every crate speaks, and the shared error.

pub mod config;
mod error;
mod secret;
pub mod types;

pub use config::Config;
pub use error::{ConfigError, ConfigIssue, Error, Result};
pub use secret::Secret;
