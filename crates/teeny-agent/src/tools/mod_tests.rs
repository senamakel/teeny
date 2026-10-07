//! The host-tool shape every Teeny tool shares: its declared access, and how
//! arguments and refusals reach the model.

use serde::Deserialize;
use serde_json::json;

use super::*;

fn echo(access: Access) -> Arc<dyn Tool> {
    HostTool::build(
        "echo",
        "Says it back.",
        json!({ "type": "object" }),
        access,
        |args| async move { answer(args["text"].as_str().unwrap_or("").to_owned()) },
    )
}

#[test]
fn a_read_tool_is_read_only_and_reaches_nothing() {
    let tool = echo(Access::Read);
    assert_eq!(tool.name(), "echo");
    assert_eq!(tool.description(), "Says it back.");
    assert_eq!(tool.permission_level(), PermissionLevel::ReadOnly);
    assert!(!tool.external_effect());
}

#[test]
fn a_write_tool_declares_write_and_an_external_effect() {
    let tool = echo(Access::Write);
    assert_eq!(tool.permission_level(), PermissionLevel::Write);
    assert!(tool.external_effect());
    assert!(tool.external_effect_with_args(&json!({})));
}

#[tokio::test]
async fn a_refusal_is_a_tool_error_with_the_same_words() {
    let tool = echo(Access::Read);
    let refused = tool
        .execute(json!({ "text": "Refused: rewrite it." }))
        .await
        .unwrap();
    assert!(refused.is_error);
    assert_eq!(text_of(&refused), "Refused: rewrite it.");
    let fine = tool.execute(json!({ "text": "gm" })).await.unwrap();
    assert!(!fine.is_error);
    assert_eq!(text_of(&fine), "gm");
}

#[derive(Debug, Deserialize)]
struct Args {
    #[serde(default)]
    n: u32,
}

#[test]
fn arguments_parse_and_a_missing_object_is_empty() {
    assert_eq!(parse::<Args>(json!({ "n": 3 })).unwrap().n, 3);
    assert_eq!(parse::<Args>(serde_json::Value::Null).unwrap().n, 0);
    let bad = parse::<Args>(json!({ "n": "three" })).unwrap_err();
    assert!(bad.is_error);
    assert!(text_of(&bad).starts_with("Invalid arguments"));
}
