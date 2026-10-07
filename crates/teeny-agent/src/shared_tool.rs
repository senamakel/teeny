//! A shared tool, owned for one turn.
//!
//! A belt is built once ([`Belt::build`](crate::posture::Belt::build)) and
//! shared as `Arc<dyn Tool>`; `AgentSpec::tools` asks for an owned
//! `Vec<Box<dyn Tool>>` on every turn, because the session behind an agent is
//! rebuilt per turn and a `Box` cannot survive in between. [`SharedTool`] is
//! the bridge: a `Box` around the `Arc`, minted per turn, so no tool is
//! rebuilt and no client or cache is duplicated.
//!
//! Every trait method is forwarded, not only the four required ones. A
//! method left at its default would answer for the wrapper instead of the
//! tool: a Write tool would read as ReadOnly to the access gate, and an
//! external effect would vanish. OpenCompany's `shared_tool` is the model.

use std::any::Any;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tinytools::{
    PermissionLevel, Tool, ToolCallOptions, ToolCategory, ToolExposure, ToolInjectedArgument,
    ToolPolicy, ToolResult, ToolRunContext, ToolScope, ToolSpec, ToolTimeout,
};

/// One shared tool, owned for the length of a turn.
pub struct SharedTool(Arc<dyn Tool>);

impl SharedTool {
    pub fn new(inner: Arc<dyn Tool>) -> Self {
        Self(inner)
    }
}

impl std::fmt::Debug for SharedTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("SharedTool").field(&self.0.name()).finish()
    }
}

/// The shared belt as an owned one, for a single turn.
pub fn owned_belt(shared: &[Arc<dyn Tool>]) -> Vec<Box<dyn Tool>> {
    shared
        .iter()
        .map(|tool| Box::new(SharedTool::new(Arc::clone(tool))) as Box<dyn Tool>)
        .collect()
}

#[async_trait]
impl Tool for SharedTool {
    fn name(&self) -> &str {
        self.0.name()
    }

    fn description(&self) -> &str {
        self.0.description()
    }

    fn parameters_schema(&self) -> Value {
        self.0.parameters_schema()
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        self.0.execute(args).await
    }

    async fn execute_with_options(
        &self,
        args: Value,
        options: ToolCallOptions,
    ) -> anyhow::Result<ToolResult> {
        self.0.execute_with_options(args, options).await
    }

    async fn execute_with_context(
        &self,
        args: Value,
        options: ToolCallOptions,
        context: Option<&dyn ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        self.0.execute_with_context(args, options, context).await
    }

    fn policy(&self) -> ToolPolicy {
        self.0.policy()
    }

    fn injected_arguments(&self) -> Vec<ToolInjectedArgument> {
        self.0.injected_arguments()
    }

    fn supports_markdown(&self) -> bool {
        self.0.supports_markdown()
    }

    fn permission_level(&self) -> PermissionLevel {
        self.0.permission_level()
    }

    fn permission_level_with_args(&self, args: &Value) -> PermissionLevel {
        self.0.permission_level_with_args(args)
    }

    fn scope(&self) -> ToolScope {
        self.0.scope()
    }

    fn category(&self) -> ToolCategory {
        self.0.category()
    }

    fn exposure(&self) -> ToolExposure {
        self.0.exposure()
    }

    fn family(&self) -> Option<&str> {
        self.0.family()
    }

    fn is_concurrency_safe(&self, args: &Value) -> bool {
        self.0.is_concurrency_safe(args)
    }

    fn external_effect(&self) -> bool {
        self.0.external_effect()
    }

    fn external_effect_with_args(&self, args: &Value) -> bool {
        self.0.external_effect_with_args(args)
    }

    fn max_result_size_chars(&self) -> Option<usize> {
        self.0.max_result_size_chars()
    }

    fn timeout_policy(&self, args: &Value) -> ToolTimeout {
        self.0.timeout_policy(args)
    }

    fn host_extension(&self) -> Option<&(dyn Any + Send + Sync)> {
        self.0.host_extension()
    }

    fn host_call_extension(&self, args: &Value) -> Option<Box<dyn Any + Send + Sync>> {
        self.0.host_call_extension(args)
    }

    fn spec(&self) -> ToolSpec {
        self.0.spec()
    }

    fn display_label(&self, args: &Value) -> Option<String> {
        self.0.display_label(args)
    }

    fn display_detail(&self, args: &Value) -> Option<String> {
        self.0.display_detail(args)
    }

    fn return_direct(&self) -> bool {
        self.0.return_direct()
    }
}

#[cfg(test)]
#[path = "shared_tool_tests.rs"]
mod tests;
