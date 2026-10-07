//! The seam to a language model, for the calls that are classifiers rather
//! than conversations: the safety judge, the algo critic and the media gate.
//!
//! Each is one request with a fixed system prompt and a JSON shape to answer
//! in; the answer comes back as JSON and is read here, field by field, so a
//! model that answers in the wrong shape is a refusal rather than a panic.
//! The OpenAI-compatible client (function calling through the ladder) lives
//! in `teeny-agent`; tests use [`ScriptedModel`].

use std::collections::VecDeque;
use std::sync::Mutex;

use async_trait::async_trait;
use serde::Serialize;
use serde_json::Value;

/// Who says a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
}

/// One message of the request.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }
}

/// One structured-output call.
#[derive(Debug, Clone, PartialEq)]
pub struct JsonRequest {
    /// Who is asking, for the usage meter: `judge`, `critic`, `media-gate`.
    pub source: &'static str,
    /// The function the answer is shaped as, e.g. `content_verdict`.
    pub name: &'static str,
    /// The answer's JSON schema.
    pub schema: Value,
    pub messages: Vec<Message>,
    pub temperature: f32,
    pub max_tokens: Option<u32>,
}

/// The model could not be asked, or did not answer.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ModelError(pub String);

/// A model that answers a [`JsonRequest`] with JSON.
#[async_trait]
pub trait JudgeModel: Send + Sync {
    async fn complete_json(&self, request: JsonRequest) -> Result<Value, ModelError>;
}

/// A model that answers from a script, in order, and keeps every request.
/// For tests here and in the crates that wire the judge, critic and gate.
pub struct ScriptedModel {
    answers: Mutex<VecDeque<Result<Value, String>>>,
    requests: Mutex<Vec<JsonRequest>>,
}

impl ScriptedModel {
    pub fn new(answers: impl IntoIterator<Item = Result<Value, String>>) -> Self {
        Self {
            answers: Mutex::new(answers.into_iter().collect()),
            requests: Mutex::new(Vec::new()),
        }
    }

    /// Every request made so far.
    pub fn requests(&self) -> Vec<JsonRequest> {
        self.requests.lock().expect("not poisoned").clone()
    }
}

#[async_trait]
impl JudgeModel for ScriptedModel {
    async fn complete_json(&self, request: JsonRequest) -> Result<Value, ModelError> {
        self.requests.lock().expect("not poisoned").push(request);
        self.answers
            .lock()
            .expect("not poisoned")
            .pop_front()
            .unwrap_or_else(|| Err("the model was asked more often than scripted".into()))
            .map_err(ModelError)
    }
}
