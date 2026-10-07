//! Every custom tool, as a host tool OpenHuman can put on a belt.
//!
//! Each tool is a [`HostTool`]: a name, the description and JSON schema the
//! TypeScript tool had (the descriptions are prompt text, so they are copied
//! word for word), an [`Access`] level, and a handler that returns the text
//! the legacy tool returned. Only the tools a belt needs are constructed (see
//! [`crate::posture`]); OpenHuman advertises everything it is handed.
//!
//! Declarations, which the host enforces:
//! - a [`Access::Write`] tool declares [`PermissionLevel::Write`] and
//!   `external_effect = true`: `x_post`, `remember`,
//!   `generate_image`, `generate_video`;
//! - every other tool is [`PermissionLevel::ReadOnly`] with no external
//!   effect, and safe to run concurrently.
//!
//! A refusal ("Refused: ...", the words the legacy tool returned so the model
//! rewrites) comes back as a tool error with the same text.

// The validation helpers return `Result<_, ToolResult>`: the error is the
// tool's answer, built once per call and handed straight back, so boxing it
// would add an indirection at every call site and save nothing.
#![allow(clippy::result_large_err)]

pub mod chart;
pub mod market;
pub mod media;
pub mod memory;
pub mod post;
pub mod web;
pub mod x;

#[cfg(test)]
pub(crate) mod test_support;

use std::future::Future;
use std::sync::Arc;

use async_trait::async_trait;
use futures::future::BoxFuture;
pub use openhuman_core::tools::{PermissionLevel, Tool, ToolResult};
use serde::de::DeserializeOwned;
use serde_json::Value;

/// What a tool may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Reads: X, the web, memory, the market. No outside effect.
    Read,
    /// Publishes, writes memory or spends a generation budget.
    Write,
}

type Handler = Arc<dyn Fn(Value) -> BoxFuture<'static, ToolResult> + Send + Sync>;

/// One tool.
pub struct HostTool {
    name: String,
    description: String,
    schema: Value,
    access: Access,
    handler: Handler,
}

impl HostTool {
    /// A tool answering with `handler`.
    pub fn build<F, Fut>(
        name: &str,
        description: &str,
        schema: Value,
        access: Access,
        handler: F,
    ) -> Arc<dyn Tool>
    where
        F: Fn(Value) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ToolResult> + Send + 'static,
    {
        Arc::new(Self {
            name: name.to_owned(),
            description: description.to_owned(),
            schema,
            access,
            handler: Arc::new(move |args| Box::pin(handler(args))),
        })
    }
}

#[async_trait]
impl Tool for HostTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters_schema(&self) -> Value {
        self.schema.clone()
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        Ok((self.handler)(args).await)
    }

    fn permission_level(&self) -> PermissionLevel {
        match self.access {
            Access::Read => PermissionLevel::ReadOnly,
            Access::Write => PermissionLevel::Write,
        }
    }

    fn external_effect(&self) -> bool {
        self.access == Access::Write
    }

    fn is_concurrency_safe(&self, _args: &Value) -> bool {
        self.access == Access::Read
    }
}

/// The legacy tool's text: a refusal is a tool error, anything else a result.
pub fn answer(text: String) -> ToolResult {
    if text.starts_with("Refused:") {
        ToolResult::error(text)
    } else {
        ToolResult::success(text)
    }
}

/// A failure the model should see, in its own words.
pub fn failed(error: impl std::fmt::Display) -> ToolResult {
    ToolResult::error(error.to_string())
}

/// The call's arguments, or the error that tells the model what was wrong.
/// No arguments at all reads as an empty object.
pub fn parse<T: DeserializeOwned>(args: Value) -> Result<T, ToolResult> {
    let args = if args.is_null() {
        Value::Object(serde_json::Map::new())
    } else {
        args
    };
    serde_json::from_value(args)
        .map_err(|error| ToolResult::error(format!("Invalid arguments: {error}")))
}

/// An argument outside its schema's range.
pub fn invalid(message: impl std::fmt::Display) -> ToolResult {
    ToolResult::error(format!("Invalid arguments: {message}"))
}

/// The text the model sees.
pub fn text_of(result: &ToolResult) -> String {
    result.text()
}

/// Whether `id` is all digits, as an X id is.
pub(crate) fn is_x_id(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())
}

/// Length as JavaScript counts it, which is what the zod schemas measured.
pub(crate) fn js_len(text: &str) -> usize {
    text.encode_utf16().count()
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
