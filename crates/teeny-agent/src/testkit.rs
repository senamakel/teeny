//! The real runtime against a scripted provider, for posture and runner
//! tests. Behind the `testkit` feature; nothing here is in a release build.
//!
//! The provider is a wiremock OpenAI-compatible endpoint driven by the user
//! message, after the embed crate's own lockdown tests: a message carrying
//! `CALL::<tool>::<json>` gets one call to `<tool>` with `<json>` as its
//! arguments, and every later request (one that already carries a tool
//! result) gets [`FINAL_REPLY`]. Every other route answers an empty success,
//! so nothing leaves the machine.
//!
//! The runtime is the process's one ([`crate::runtime::global`]), booted on
//! first use with every Teeny agent registered on [`Deps::fake`], whose own
//! clients reach nothing. Turns run on the runtime's executor through
//! [`run`]: a test thread's 2 MiB stack would overflow a turn.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use serde_json::{Value, json};
use teeny_core::Secret;
use wiremock::matchers::{any, method, path_regex};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use crate::deps::Deps;
use crate::runtime::{self, RuntimeSettings};
use crate::specs::{SpecContext, TeenyAgents};

/// What starts a scripted tool call in a user message.
pub const MARKER: &str = "CALL::";
/// A user message carrying this is answered after [`SLOW_FOR`]: for timeouts.
pub const SLOW: &str = "SLOW::";
/// How long a [`SLOW`] message waits for its answer.
pub const SLOW_FOR: std::time::Duration = std::time::Duration::from_secs(60);
/// The model's answer once a tool has answered, or when none was asked for.
pub const FINAL_REPLY: &str = "done";
/// The model alias every fixture agent runs on.
pub const MODEL: &str = "fixture";

/// A user message that makes the model call `tool` with `args`.
pub fn call(tool: &str, args: Value) -> String {
    format!("{MARKER}{tool}::{args}")
}

struct Scripted;

impl wiremock::Respond for Scripted {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&request.body).unwrap_or_default();
        let messages = body["messages"].as_array().cloned().unwrap_or_default();
        let answered = messages.iter().any(|m| {
            m["role"] == "tool"
                || m["content"]
                    .as_str()
                    .unwrap_or("")
                    .contains("[Tool results]")
        });
        let asked = messages
            .iter()
            .filter(|m| m["role"] == "user")
            .filter_map(|m| m["content"].as_str())
            .find_map(|text| text.split_once(MARKER).map(|(_, rest)| rest.to_owned()));
        let reply = match (asked, answered) {
            (Some(rest), false) => {
                let (tool, args) = rest.split_once("::").unwrap_or((rest.as_str(), "{}"));
                let args = args.lines().next().unwrap_or("{}").trim();
                tool_call(tool, args)
            }
            _ => completion(FINAL_REPLY),
        };
        let slow = messages.iter().filter(|m| m["role"] == "user").any(|m| {
            m["content"]
                .as_str()
                .is_some_and(|text| text.contains(SLOW))
        });
        let response = ResponseTemplate::new(200).set_body_json(reply);
        if slow {
            response.set_delay(SLOW_FOR)
        } else {
            response
        }
    }
}

fn usage() -> Value {
    json!({"prompt_tokens": 11, "completion_tokens": 7, "total_tokens": 18})
}

fn completion(content: &str) -> Value {
    json!({
        "id": "chatcmpl-teeny-test",
        "object": "chat.completion",
        "created": 1_700_000_000_u64,
        "model": MODEL,
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": content},
            "finish_reason": "stop"
        }],
        "usage": usage()
    })
}

fn tool_call(tool: &str, arguments: &str) -> Value {
    json!({
        "id": "chatcmpl-teeny-test-tool",
        "object": "chat.completion",
        "created": 1_700_000_000_u64,
        "model": MODEL,
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": "call_teeny_1",
                    "type": "function",
                    "function": {"name": tool, "arguments": arguments}
                }]
            },
            "finish_reason": "tool_calls"
        }],
        "usage": usage()
    })
}

/// The scripted provider, also standing in for the hosted backend.
pub async fn provider() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path_regex(r"/chat/completions$"))
        .respond_with(Scripted)
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(any())
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "success": true,
            "data": {"id": "teeny-test", "email": "local@openhuman.local"}
        })))
        .with_priority(10)
        .mount(&server)
        .await;
    server
}

/// The chat completions the provider has answered, oldest first.
pub async fn chat_requests(server: &MockServer) -> Vec<Request> {
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|request| request.url.path().ends_with("/chat/completions"))
        .collect()
}

fn body(request: &Request) -> Value {
    serde_json::from_slice(&request.body).unwrap_or_default()
}

/// The tool specs a request advertised, by name.
pub fn tool_specs(request: &Request) -> Vec<(String, Value)> {
    body(request)["tools"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|tool| {
            let name = tool
                .pointer("/function/name")
                .or_else(|| tool.get("name"))?
                .as_str()?
                .to_owned();
            Some((name, tool))
        })
        .collect()
}

/// The names of the tools a request advertised.
pub fn tool_names(request: &Request) -> Vec<String> {
    tool_specs(request)
        .into_iter()
        .map(|(name, _)| name)
        .collect()
}

/// Whether a request's first user message carries `needle`: which turn it
/// belongs to.
pub fn asks(request: &Request, needle: &str) -> bool {
    body(request)["messages"]
        .as_array()
        .is_some_and(|messages| {
            messages.iter().filter(|m| m["role"] == "user").any(|m| {
                m["content"]
                    .as_str()
                    .is_some_and(|text| text.contains(needle))
            })
        })
}

/// The tool results a request handed back to the model.
pub fn tool_results(request: &Request) -> String {
    body(request)["messages"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|m| {
            let content = match m.get("content")? {
                Value::String(text) => text.clone(),
                other => other.to_string(),
            };
            match m["role"].as_str() {
                Some("tool") => Some(content),
                Some("user") if content.contains("[Tool results]") => content
                    .split_once("[Tool results]")
                    .map(|(_, rest)| rest.to_owned()),
                _ => None,
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The booted runtime, its agents and its provider.
pub struct Booted {
    pub agents: Arc<TeenyAgents>,
    pub server: MockServer,
    pub data_dir: PathBuf,
}

static BOOTED: OnceLock<Booted> = OnceLock::new();

/// Runs `future` to completion on the runtime's executor.
pub fn run<F>(future: F) -> F::Output
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    let executor = runtime::executor();
    executor
        .block_on(executor.spawn(future))
        .expect("the test turn panicked")
}

/// The process's runtime with every Teeny agent on it, booted once.
pub fn booted() -> &'static Booted {
    BOOTED.get_or_init(|| run(boot()))
}

async fn boot() -> Booted {
    let server = provider().await;
    let data_dir = std::env::temp_dir().join(format!("teeny-testkit-{}", std::process::id()));
    let settings = RuntimeSettings {
        data_dir: data_dir.clone(),
        chrome_path: None,
        unattended_actions: vec!["click".into(), "press".into()],
        allowed_domains: vec!["dexscreener.com".into()],
        tinycomputer_module: None,
        backend_url: Some(server.uri()),
        memory: runtime::MemorySettings::default(),
    };
    let runtime = runtime::global(settings)
        .await
        .expect("the OpenHuman runtime boots");
    let context = SpecContext {
        base_url: format!("{}/v1", server.uri()),
        api_key: Secret::new("fixture"),
        model: MODEL.into(),
        skills_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../skills"),
        max_iterations: 4,
        memory_root: teeny_memory::DEFAULT_ROOT.into(),
    };
    let agents = TeenyAgents::register(runtime, &Deps::fake(), context)
        .expect("every Teeny agent registers");
    Booted {
        agents: Arc::new(agents),
        server,
        data_dir,
    }
}
