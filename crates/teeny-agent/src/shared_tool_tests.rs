//! The adapter must answer every question for the tool it wraps: a method
//! left at the trait default would make a Write tool read as ReadOnly to the
//! gate, which is the failure this exists to prevent.

use std::sync::Arc;

use serde_json::{Value, json};

use super::*;
use crate::tools::{Access, HostTool, PermissionLevel, Tool, ToolResult};

fn write_tool() -> Arc<dyn Tool> {
    HostTool::build(
        "x_post",
        "Publish a post.",
        json!({"type": "object", "properties": {"text": {"type": "string"}}}),
        Access::Write,
        |args: Value| async move { ToolResult::success(format!("posted {}", args["text"])) },
    )
}

#[test]
fn answers_the_gates_questions_as_the_wrapped_tool() {
    let inner = write_tool();
    let shared = SharedTool::new(Arc::clone(&inner));
    let args = json!({"text": "gm"});
    assert_eq!(shared.name(), "x_post");
    assert_eq!(shared.description(), inner.description());
    assert_eq!(shared.parameters_schema(), inner.parameters_schema());
    assert_eq!(shared.permission_level(), PermissionLevel::Write);
    assert_eq!(shared.permission_level_with_args(&args), PermissionLevel::Write);
    assert!(shared.external_effect());
    assert!(shared.external_effect_with_args(&args));
    assert_eq!(
        shared.is_concurrency_safe(&args),
        inner.is_concurrency_safe(&args)
    );
    assert_eq!(shared.exposure(), inner.exposure());
    assert_eq!(shared.spec().name, inner.spec().name);
}

#[tokio::test]
async fn runs_the_wrapped_tool() {
    let shared = SharedTool::new(write_tool());
    let result = shared.execute(json!({"text": "gm"})).await.unwrap();
    assert!(crate::tools::text_of(&result).contains("posted \"gm\""));
}

#[test]
fn an_owned_belt_is_the_shared_belt_by_name_and_order() {
    let belt: Vec<Arc<dyn Tool>> = vec![write_tool(), write_tool()];
    let owned = owned_belt(&belt);
    assert_eq!(owned.len(), 2);
    assert!(owned.iter().all(|tool| tool.name() == "x_post"));
}
