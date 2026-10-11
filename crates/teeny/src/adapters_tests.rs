use std::path::PathBuf;
use std::sync::Arc;

use serde_json::json;
use teeny_agent::posture::Belt;
use teeny_agent::runtime::RuntimeSettings;
use teeny_jobs::runner::{AgentKey, AgentOutcome, Origin, ScriptedRunner, ToolCallRecord};

use super::*;

#[tokio::test]
async fn runs_an_operator_task_on_the_operator_agent_as_the_operator() {
    let runner = Arc::new(ScriptedRunner::new().outcome(AgentOutcome {
        reply: "done".into(),
        session_id: "s".into(),
        tool_calls: vec![ToolCallRecord {
            name: "web_search".into(),
            arguments: json!({ "query": "tiny" }),
            result: "3 results".into(),
            is_error: false,
        }],
        usage: None,
        model: None,
    }));
    let agent = OperatorAgent(runner.clone());

    let result = agent.run("look this up", Some("thread-1")).await.unwrap();

    assert_eq!(result.output, "done");
    assert_eq!(result.steps.len(), 1);
    assert_eq!(result.steps[0].tool, "web_search");
    assert_eq!(result.steps[0].input, json!({ "query": "tiny" }));
    assert_eq!(result.steps[0].result.as_deref(), Some("3 results"));
    let seen = &runner.seen()[0];
    assert_eq!(seen.agent, AgentKey::Belt(Belt::Operator));
    assert_eq!(seen.origin, Origin::Operator);
    assert_eq!(seen.prompt, "look this up");
    assert_eq!(seen.session.as_deref(), Some("thread-1"));
}

#[tokio::test]
async fn a_failed_task_is_an_error() {
    let agent = OperatorAgent(Arc::new(ScriptedRunner::new().fail("ladder down")));
    assert!(agent.run("x", None).await.is_err());
}

fn settings(module: Option<&str>) -> RuntimeSettings {
    RuntimeSettings {
        data_dir: PathBuf::from("/data"),
        chrome_path: Some("/usr/bin/chromium".into()),
        unattended_actions: vec!["click".into(), "press".into(), "teleport".into()],
        allowed_domains: vec![],
        tinycomputer_module: module.map(PathBuf::from),
        backend_url: None,
        memory: Default::default(),
    }
}

#[tokio::test]
async fn reports_the_browsers_configuration_without_launching_it() {
    let on = BrowserState::from_settings(&settings(Some("/opt/tinycomputer/libtinycomputer.so")));
    assert_eq!(
        on.status().await,
        json!({
            "configured": true,
            "launched": false,
            "module": "/opt/tinycomputer/libtinycomputer.so",
            "chromePath": "/usr/bin/chromium",
            "headless": true,
            "profile": "persistent",
            "profilePath": "/data/chrome-profile",
            "unattendedActions": ["click", "press"],
        })
    );
    let off = BrowserState::from_settings(&settings(None));
    assert_eq!(off.status().await["configured"], false);
}
