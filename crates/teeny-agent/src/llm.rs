//! A single-shot OpenAI-compatible client against the ladder: the calls that
//! are one request and one answer rather than a conversation with tools (the
//! safety judge, the algo critic, the media gate, the reference sheet's
//! distillation, proofreading, captions).
//!
//! Every call that comes back leaves one row in the inference ledger, shaped
//! as `legacy/src/ledger/completion.ts` wrote it: the messages without the
//! system prompt, a digest of the prompt instead, the answer, the usage. A
//! ledger that is down costs a log line, never the answer.
//!
//! Structured answers ([`JudgeModel`]) are a forced function call, as the
//! legacy `withStructuredOutput(..., { method: 'functionCalling' })` made
//! them; a rung that answers in plain JSON text instead is still read.

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::Serialize;
use serde_json::{Value, json};
pub use teeny_core::Secret;
use teeny_core::config::LlmConfig;
use teeny_safety::model::{JsonRequest, JudgeModel, ModelError};
use teeny_store::InferenceLedger;
use teeny_store::inference::{
    InferenceInput, Lane, LedgerMessage, MAX_MESSAGE_CHARS, Role, cut, record_quietly, sha256,
    tokens_from_usage,
};

/// How long one call may take.
const TIMEOUT: Duration = Duration::from_secs(60);
/// How much of an answer the ledger keeps.
const MAX_OUTPUT_CHARS: usize = 4_000;
/// How much of an error body is kept.
const ERROR_BODY_CHARS: usize = 300;

/// Why a call produced nothing usable.
#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    /// The endpoint answered with an error status.
    #[error("LLM request failed ({status}): {body}")]
    Http { status: u16, body: String },
    /// The endpoint could not be reached.
    #[error("could not reach the model: {0}")]
    Transport(#[from] reqwest::Error),
    /// It answered, but not with anything that can be used.
    #[error("{0}")]
    Unusable(String),
}

/// One message of a request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChatMessage {
    pub role: &'static str,
    pub content: String,
}

/// A function the answer must be shaped as.
#[derive(Debug, Clone, PartialEq)]
pub struct AnswerShape {
    pub name: String,
    pub schema: Value,
}

/// One call.
#[derive(Debug, Clone, PartialEq)]
pub struct Completion {
    /// What the call is for, as the ledger names it.
    pub source: String,
    pub messages: Vec<ChatMessage>,
    pub temperature: f32,
    pub max_tokens: Option<u32>,
    /// Present, the answer is a forced call of this function.
    pub shape: Option<AnswerShape>,
}

/// What came back.
#[derive(Debug, Clone, PartialEq)]
pub struct Completed {
    /// The message text, trimmed. Empty when the model only called the function.
    pub text: String,
    /// The function call's arguments, parsed, when there was one.
    pub arguments: Option<Value>,
}

/// The client. Cheap to clone; [`with_model`](Self::with_model) gives the
/// same endpoint under another alias of the ladder.
#[derive(Clone)]
pub struct LlmClient {
    http: reqwest::Client,
    base_url: String,
    api_key: Secret,
    model: String,
    /// The ledger's name for plain text calls.
    source: String,
    temperature: f32,
    ledger: Option<Arc<dyn InferenceLedger>>,
}

impl std::fmt::Debug for LlmClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlmClient")
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("source", &self.source)
            .finish_non_exhaustive()
    }
}

impl LlmClient {
    /// `model` on the OpenAI-compatible endpoint at `base_url`. Plain text
    /// calls are ledgered as `agent` at temperature 0.2 until told otherwise.
    pub fn new(base_url: impl Into<String>, api_key: Secret, model: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(TIMEOUT)
                .build()
                .unwrap_or_default(),
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            api_key,
            model: model.into(),
            source: "agent".into(),
            temperature: 0.2,
            ledger: None,
        }
    }

    /// `LLM_BASE_URL`, `LLM_API_KEY`, `LLM_MODEL`.
    pub fn from_config(config: &LlmConfig) -> Self {
        Self::new(&config.base_url, config.api_key.clone(), &config.model)
    }

    /// Where every call is written down.
    pub fn with_ledger(mut self, ledger: Arc<dyn InferenceLedger>) -> Self {
        self.ledger = Some(ledger);
        self
    }

    /// The ledger's name for this client's plain text calls.
    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = source.into();
        self
    }

    /// The temperature of this client's plain text calls.
    pub fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = temperature;
        self
    }

    /// The same endpoint and ledger, another model alias (`flash`, `vision`...).
    pub fn with_model(&self, model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            ..self.clone()
        }
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    /// One system line, one prompt, the answer's text.
    pub async fn complete(&self, system: Option<&str>, prompt: &str) -> Result<String, LlmError> {
        let mut messages = Vec::new();
        if let Some(system) = system.filter(|system| !system.is_empty()) {
            messages.push(ChatMessage {
                role: "system",
                content: system.to_owned(),
            });
        }
        messages.push(ChatMessage {
            role: "user",
            content: prompt.to_owned(),
        });
        let completed = self
            .call(Completion {
                source: self.source.clone(),
                messages,
                temperature: self.temperature,
                max_tokens: None,
                shape: None,
            })
            .await?;
        if completed.text.is_empty() {
            return Err(LlmError::Unusable("the model returned nothing".into()));
        }
        Ok(completed.text)
    }

    /// Makes the call and writes its ledger row.
    pub async fn call(&self, request: Completion) -> Result<Completed, LlmError> {
        let started = Instant::now();
        let mut body = json!({
            "model": self.model,
            "temperature": decimal(request.temperature),
            "messages": request.messages,
        });
        if let Some(max_tokens) = request.max_tokens {
            body["max_tokens"] = json!(max_tokens);
        }
        if let Some(shape) = &request.shape {
            body["tools"] = json!([{
                "type": "function",
                "function": { "name": shape.name, "parameters": shape.schema }
            }]);
            body["tool_choice"] = json!({ "type": "function", "function": { "name": shape.name } });
        }
        let response = self
            .http
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(self.api_key.expose())
            .json(&body)
            .send()
            .await?;
        let status = response.status();
        let text = response.text().await?;
        if !status.is_success() {
            return Err(LlmError::Http {
                status: status.as_u16(),
                body: text.chars().take(ERROR_BODY_CHARS).collect(),
            });
        }
        let answer: Value = serde_json::from_str(&text).map_err(|error| {
            LlmError::Unusable(format!("the model sent unreadable JSON: {error}"))
        })?;
        let message = answer.pointer("/choices/0/message");
        let content = message.map(|message| content_text(message.get("content")));
        let content = content.unwrap_or_default();
        let call = message
            .and_then(|message| message.pointer("/tool_calls/0/function"))
            .map(|function| {
                (
                    function.get("name").and_then(Value::as_str).unwrap_or(""),
                    function
                        .get("arguments")
                        .and_then(Value::as_str)
                        .unwrap_or(""),
                )
            });
        let reasoning = message
            .and_then(|message| message.get("reasoning_content"))
            .and_then(Value::as_str)
            .filter(|reasoning| !reasoning.trim().is_empty());

        let output = match call {
            Some((name, arguments)) => {
                [content.trim().to_owned(), format!("→ {name}({arguments})")]
                    .into_iter()
                    .filter(|line| !line.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n")
            }
            None => content.clone(),
        };
        self.record(&request, &answer, &output, reasoning, started)
            .await;

        let arguments = match call {
            Some((_, arguments)) => Some(serde_json::from_str(arguments).map_err(|error| {
                LlmError::Unusable(format!("the function arguments are not JSON: {error}"))
            })?),
            None => None,
        };
        Ok(Completed {
            text: content.trim().to_owned(),
            arguments,
        })
    }

    async fn record(
        &self,
        request: &Completion,
        answer: &Value,
        output: &str,
        reasoning: Option<&str>,
        started: Instant,
    ) {
        let Some(ledger) = self.ledger.as_deref() else {
            return;
        };
        let input = request
            .messages
            .iter()
            .filter(|message| message.role != "system")
            .map(|message| LedgerMessage {
                role: if message.role == "assistant" {
                    Role::Assistant
                } else {
                    Role::User
                },
                tool: None,
                text: cut(&message.content, MAX_MESSAGE_CHARS),
            })
            .collect();
        let system: Vec<&str> = request
            .messages
            .iter()
            .filter(|message| message.role == "system")
            .map(|message| message.content.as_str())
            .collect();
        let sent = serde_json::to_string(&request.messages).unwrap_or_default();
        let received = json!({ "content": output, "reasoning": reasoning }).to_string();
        let mut row = InferenceInput::new(
            Lane::Mascot,
            &request.source,
            answer
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or(&self.model),
            tokens_from_usage(answer.get("usage")),
        );
        row.input = Some(input);
        row.reasoning = reasoning.map(|reasoning| cut(reasoning, MAX_OUTPUT_CHARS));
        row.output = Some(cut(output, MAX_OUTPUT_CHARS));
        row.response_id = answer.get("id").and_then(Value::as_str).map(str::to_owned);
        row.digest = Some(sha256(&[&sent, &received]));
        row.prompt_digest = (!system.is_empty()).then(|| sha256(&system));
        row.duration_ms = Some(i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX));
        record_quietly(Some(ledger), row).await;
    }
}

/// An `f32` as the decimal it was written as (0.2, not 0.20000000298...).
fn decimal(value: f32) -> f64 {
    value.to_string().parse().unwrap_or(f64::from(value))
}

/// A message's content as text, whether a string or a list of parts.
fn content_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect(),
        _ => String::new(),
    }
}

/// JSON from a text answer, with or without a code fence around it.
fn json_in_text(text: &str) -> Option<Value> {
    let trimmed = text.trim();
    let unfenced = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .and_then(|rest| rest.trim_end().strip_suffix("```"))
        .unwrap_or(trimmed);
    serde_json::from_str::<Value>(unfenced.trim())
        .ok()
        .filter(Value::is_object)
}

#[async_trait]
impl JudgeModel for LlmClient {
    async fn complete_json(&self, request: JsonRequest) -> Result<Value, ModelError> {
        let messages = request
            .messages
            .iter()
            .map(|message| {
                let value = serde_json::to_value(message).unwrap_or_default();
                ChatMessage {
                    role: if value["role"] == "system" {
                        "system"
                    } else {
                        "user"
                    },
                    content: message.content.clone(),
                }
            })
            .collect();
        let completed = self
            .call(Completion {
                source: request.source.to_owned(),
                messages,
                temperature: request.temperature,
                max_tokens: request.max_tokens,
                shape: Some(AnswerShape {
                    name: request.name.to_owned(),
                    schema: request.schema,
                }),
            })
            .await
            .map_err(|error| ModelError(error.to_string()))?;
        completed
            .arguments
            .or_else(|| json_in_text(&completed.text))
            .ok_or_else(|| ModelError("the model did not answer in the JSON shape".into()))
    }
}

#[async_trait]
impl teeny_media::reference::TextModel for LlmClient {
    async fn complete(&self, system: Option<&str>, prompt: &str) -> teeny_media::Result<String> {
        LlmClient::complete(self, system, prompt)
            .await
            .map_err(|error| teeny_media::Error::failed(error.to_string()))
    }
}

#[cfg(test)]
#[path = "llm_tests.rs"]
mod tests;
