//! Video generation: OpenAI's `/videos` (Sora, or anything that speaks the
//! same three routes) and the LLM ladder's `/video/generations` job API.
//!
//! Rendering takes minutes, so both poll on a coarse interval until their own
//! deadline; a video that is not done by then is abandoned, not posted late.

use std::fmt;
use std::time::Duration;

use async_trait::async_trait;
use serde::de::{Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Value, json};
use tokio::time::Instant;

use crate::error::{Error, Result};
use crate::http::{decode_base64_lenient, encode_component, endpoint, error_detail};
use crate::types::{GeneratedVideo, VideoGenerator, VideoOrientation, VideoRequest};

const SORA_SECONDS: [u32; 3] = [4, 8, 12];
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const DEFAULT_POLL: Duration = Duration::from_secs(10);
const CREATE_TIMEOUT: Duration = Duration::from_secs(60);
const STATUS_TIMEOUT: Duration = Duration::from_secs(30);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(120);

/// The polling knobs both generators share.
#[derive(Debug, Clone)]
struct Endpoint {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
    timeout: Duration,
    poll: Duration,
}

impl Endpoint {
    fn new(base_url: String, api_key: String, model: String) -> Self {
        Self { client: reqwest::Client::new(), base_url, api_key, model, timeout: DEFAULT_TIMEOUT, poll: DEFAULT_POLL }
    }

    fn url(&self, path: &str) -> String {
        endpoint(&self.base_url, path)
    }

    async fn post(&self, path: &str, body: &Value) -> Result<(u16, String)> {
        let response = self
            .client
            .post(self.url(path))
            .bearer_auth(&self.api_key)
            .timeout(CREATE_TIMEOUT)
            .json(body)
            .send()
            .await?;
        let status = response.status().as_u16();
        Ok((status, response.text().await.unwrap_or_default()))
    }

    async fn get_text(&self, url: &str) -> Result<(u16, String)> {
        let response = self.client.get(url).bearer_auth(&self.api_key).timeout(STATUS_TIMEOUT).send().await?;
        let status = response.status().as_u16();
        Ok((status, response.text().await.unwrap_or_default()))
    }

    async fn download(&self, url: &str, id: &str) -> Result<Vec<u8>> {
        let response = self.client.get(url).bearer_auth(&self.api_key).timeout(DOWNLOAD_TIMEOUT).send().await?;
        if !response.status().is_success() {
            return Err(Error::failed(format!("video download failed ({})", response.status().as_u16())));
        }
        let data = response.bytes().await?.to_vec();
        if data.is_empty() {
            return Err(Error::failed(format!("video {id} downloaded empty")));
        }
        Ok(data)
    }
}

/// A JSON body as `await response.json().catch(() => ({}))` would read it.
fn parse(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|_| Value::Object(Map::new()))
}

fn is_success(status: u16) -> bool {
    (200..300).contains(&status)
}

/// Video generation through an OpenAI-shaped `/videos` endpoint: create a
/// job, poll it, then download the MP4.
#[derive(Debug, Clone)]
pub struct OpenAiVideoGenerator {
    endpoint: Endpoint,
    seconds: Option<u32>,
}

impl OpenAiVideoGenerator {
    /// A generator on this endpoint and model: ten-minute deadline, ten-second
    /// polls, four-second clips unless asked otherwise.
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self { endpoint: Endpoint::new(base_url.into(), api_key.into(), model.into()), seconds: None }
    }

    /// Clip length when a request names none. Sora accepts 4, 8 or 12.
    pub fn with_seconds(mut self, seconds: u32) -> Self {
        self.seconds = Some(seconds);
        self
    }

    /// How long to wait for a render before giving up.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.endpoint.timeout = timeout;
        self
    }

    /// How long to wait between status checks.
    pub fn with_poll_interval(mut self, poll: Duration) -> Self {
        self.endpoint.poll = poll;
        self
    }
}

#[async_trait]
impl VideoGenerator for OpenAiVideoGenerator {
    async fn generate(&self, request: VideoRequest) -> Result<GeneratedVideo> {
        let api = &self.endpoint;
        // Sora takes 4, 8 or 12 and nothing else; an ask in between rounds up.
        let wanted = request.seconds.or(self.seconds).unwrap_or(4);
        let seconds = SORA_SECONDS.into_iter().find(|step| *step >= wanted).unwrap_or(12);
        let (status, text) = api
            .post(
                "/videos",
                &json!({
                    "model": api.model,
                    "prompt": request.prompt,
                    "seconds": seconds.to_string(),
                    // Sora has no square; a square ask gets the landscape frame.
                    "size": if request.orientation == Some(VideoOrientation::Portrait) { "720x1280" } else { "1280x720" },
                }),
            )
            .await?;
        let job = parse(&text);
        let id = job.get("id").and_then(Value::as_str).filter(|id| !id.is_empty());
        let (true, Some(id)) = (is_success(status), id) else {
            return Err(Error::failed(format!("video generation failed ({status}): {}", error_detail(&job))));
        };

        let deadline = Instant::now() + api.timeout;
        let mut state = job.get("status").and_then(Value::as_str).unwrap_or("queued").to_owned();
        let mut failure = job.pointer("/error/message").and_then(Value::as_str).map(str::to_owned);
        let status_url = api.url(&format!("/videos/{}", encode_component(id)));
        while state != "completed" {
            if state == "failed" {
                return Err(Error::failed(format!(
                    "video {id} failed: {}",
                    failure.as_deref().unwrap_or("no detail")
                )));
            }
            if Instant::now() >= deadline {
                return Err(Error::failed(format!("video {id} was still {state} at the deadline")));
            }
            tokio::time::sleep(api.poll).await;
            let (status, text) = api.get_text(&status_url).await?;
            let next = parse(&text);
            if !is_success(status) {
                return Err(Error::failed(format!("video status failed ({status}): {}", error_detail(&next))));
            }
            if let Some(next_state) = next.get("status").and_then(Value::as_str) {
                state = next_state.to_owned();
            }
            if let Some(message) = next.pointer("/error/message").and_then(Value::as_str) {
                failure = Some(message.to_owned());
            }
        }

        let data = api.download(&format!("{status_url}/content"), id).await?;
        Ok(GeneratedVideo { data, seconds: Some(seconds) })
    }
}

/// The clip length, in every spelling the marketplace's sellers might read.
///
/// Measured on 2026-09-14 against `kling-o3-standard-text-to-video`, every job
/// served by Venice: each spelling alone rendered the seller's own three
/// seconds, while a body carrying all five rendered ten. Narrow it once there
/// is credit to bisect with.
fn length_fields(body: &mut Map<String, Value>, seconds: u32) {
    let text = seconds.to_string();
    body.insert("seconds".into(), json!(text));
    body.insert("duration".into(), json!(text));
    body.insert("video_length".into(), json!(seconds));
    body.insert("length".into(), json!(seconds));
    body.insert("duration_seconds".into(), json!(seconds));
}

fn aspect(orientation: VideoOrientation) -> &'static str {
    match orientation {
        VideoOrientation::Landscape => "16:9",
        VideoOrientation::Portrait => "9:16",
        VideoOrientation::Square => "1:1",
    }
}

const DONE: [&str; 6] = ["completed", "succeeded", "success", "done", "finished", "ready"];
const DEAD: [&str; 6] = ["failed", "error", "canceled", "cancelled", "expired", "rejected"];

/// Video generation through the LLM ladder: `POST /video/generations` answers
/// 202 with a `media.job`, `GET /video/generations/{id}` is polled until it
/// finishes, and the clip is downloaded from wherever the finished job says it
/// is.
///
/// The router relays the marketplace's job object untouched and documents no
/// finished shape, so the result is read leniently: any http(s) string under a
/// key ending in `url` that is not the job's own poll or cancel URL, an MP4
/// one preferred, or a base64 field. The error, when nothing fits, names the
/// keys that were there so the next person can add the right one.
#[derive(Debug, Clone)]
pub struct LadderVideoGenerator {
    endpoint: Endpoint,
}

impl LadderVideoGenerator {
    /// A generator on this endpoint and model: ten-minute deadline, ten-second
    /// polls.
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self { endpoint: Endpoint::new(base_url.into(), api_key.into(), model.into()) }
    }

    /// How long to wait for a render before giving up.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.endpoint.timeout = timeout;
        self
    }

    /// How long to wait between status checks.
    pub fn with_poll_interval(mut self, poll: Duration) -> Self {
        self.endpoint.poll = poll;
        self
    }
}

#[async_trait]
impl VideoGenerator for LadderVideoGenerator {
    async fn generate(&self, request: VideoRequest) -> Result<GeneratedVideo> {
        let api = &self.endpoint;
        let mut body = Map::new();
        body.insert("model".into(), json!(api.model));
        body.insert("prompt".into(), json!(request.prompt));
        body.insert("aspect_ratio".into(), json!(aspect(request.orientation.unwrap_or_default())));
        if let Some(seconds) = request.seconds.filter(|seconds| *seconds > 0) {
            length_fields(&mut body, seconds);
        }
        let (status, text) = api.post("/video/generations", &Value::Object(body)).await?;
        let mut job = JobNode::parse(&text);
        let Some(id) = job.get("id").and_then(JobNode::as_str).map(str::to_owned).filter(|_| is_success(status)) else {
            return Err(Error::failed(format!("video generation failed ({status}): {}", job.error_message())));
        };

        let deadline = Instant::now() + api.timeout;
        let status_url = api.url(&format!("/video/generations/{}", encode_component(&id)));
        loop {
            let state = job.status();
            if DONE.contains(&state.as_str()) {
                break;
            }
            if DEAD.contains(&state.as_str()) {
                return Err(Error::failed(format!(
                    "video {id} {state}: {}",
                    job.failure().unwrap_or("no detail")
                )));
            }
            if Instant::now() >= deadline {
                return Err(Error::failed(format!("video {id} was still {state} at the deadline")));
            }
            tokio::time::sleep(api.poll).await;
            let (status, text) = api.get_text(&status_url).await?;
            let next = JobNode::parse(&text);
            if !is_success(status) {
                return Err(Error::failed(format!("video status failed ({status}): {}", next.error_message())));
            }
            job = next;
        }

        if let Some(data) = base64_of(&job) {
            return Ok(GeneratedVideo { data, seconds: None });
        }
        let Some(url) = video_url_of(&job) else {
            return Err(Error::failed(format!(
                "video {id} finished but nothing in the job looks like a clip; top-level keys: {}",
                job.keys().join(", ")
            )));
        };
        let data = api.download(&url, &id).await?;
        Ok(GeneratedVideo { data, seconds: None })
    }
}

/// A JSON value whose objects keep their keys in the order they arrived.
///
/// `serde_json::Value` sorts them, and the order is part of the result here:
/// the first URL found wins, and the error lists the keys as the job had them.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum JobNode {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<JobNode>),
    Object(Vec<(String, JobNode)>),
}

impl JobNode {
    /// The body, or an empty object when it is not JSON.
    fn parse(text: &str) -> Self {
        serde_json::from_str(text).unwrap_or(Self::Object(Vec::new()))
    }

    fn get(&self, key: &str) -> Option<&Self> {
        match self {
            Self::Object(entries) => entries.iter().find(|(name, _)| name == key).map(|(_, value)| value),
            _ => None,
        }
    }

    fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(text) => Some(text),
            _ => None,
        }
    }

    fn keys(&self) -> Vec<&str> {
        match self {
            Self::Object(entries) => entries.iter().map(|(name, _)| name.as_str()).collect(),
            _ => Vec::new(),
        }
    }

    /// `String(job.status ?? 'queued').toLowerCase()`.
    fn status(&self) -> String {
        match self.get("status") {
            None | Some(Self::Null) => "queued".to_owned(),
            Some(Self::String(text)) => text.to_lowercase(),
            Some(Self::Bool(flag)) => flag.to_string(),
            Some(Self::Number(number)) => crate::format::js_number(*number),
            Some(_) => "[object object]".to_owned(),
        }
    }

    fn error_message(&self) -> &str {
        self.get("error").and_then(|error| error.get("message")).and_then(Self::as_str).unwrap_or("no detail")
    }

    /// The job's own words for why it died.
    fn failure(&self) -> Option<&str> {
        match self.get("error") {
            Some(Self::String(text)) => return Some(text),
            Some(error @ Self::Object(_)) => {
                if let Some(message) = error.get("message").and_then(Self::as_str) {
                    return Some(message);
                }
            }
            _ => {}
        }
        self.get("failure_reason").and_then(Self::as_str)
    }
}

impl<'de> Deserialize<'de> for JobNode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct NodeVisitor;

        impl<'de> Visitor<'de> for NodeVisitor {
            type Value = JobNode;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("any JSON value")
            }
            fn visit_unit<E>(self) -> std::result::Result<JobNode, E> {
                Ok(JobNode::Null)
            }
            fn visit_none<E>(self) -> std::result::Result<JobNode, E> {
                Ok(JobNode::Null)
            }
            fn visit_bool<E>(self, value: bool) -> std::result::Result<JobNode, E> {
                Ok(JobNode::Bool(value))
            }
            fn visit_i64<E>(self, value: i64) -> std::result::Result<JobNode, E> {
                Ok(JobNode::Number(value as f64))
            }
            fn visit_u64<E>(self, value: u64) -> std::result::Result<JobNode, E> {
                Ok(JobNode::Number(value as f64))
            }
            fn visit_f64<E>(self, value: f64) -> std::result::Result<JobNode, E> {
                Ok(JobNode::Number(value))
            }
            fn visit_str<E>(self, value: &str) -> std::result::Result<JobNode, E> {
                Ok(JobNode::String(value.to_owned()))
            }
            fn visit_string<E>(self, value: String) -> std::result::Result<JobNode, E> {
                Ok(JobNode::String(value))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<JobNode, A::Error> {
                let mut items = Vec::new();
                while let Some(item) = seq.next_element()? {
                    items.push(item);
                }
                Ok(JobNode::Array(items))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<JobNode, A::Error> {
                let mut entries: Vec<(String, JobNode)> = Vec::new();
                while let Some((key, value)) = map.next_entry::<String, JobNode>()? {
                    // A repeated key keeps its first place and its last value,
                    // as a JavaScript object would.
                    if let Some(slot) = entries.iter_mut().find(|(name, _)| *name == key) {
                        slot.1 = value;
                    } else {
                        entries.push((key, value));
                    }
                }
                Ok(JobNode::Object(entries))
            }
        }

        deserializer.deserialize_any(NodeVisitor)
    }
}

/// Every `…url` string in the job, however deep, with the key path it sat
/// under, in the order the job listed them.
fn urls_in(value: &JobNode, skip: &[&str], path: &str, found: &mut Vec<(String, String)>) {
    match value {
        JobNode::Array(items) => {
            for item in items {
                urls_in(item, skip, path, found);
            }
        }
        JobNode::Object(entries) => {
            for (key, entry) in entries {
                let here = if path.is_empty() { key.clone() } else { format!("{path}.{key}") };
                match entry {
                    JobNode::String(text) => {
                        let is_url_key = key.to_lowercase().ends_with("url");
                        let is_web = text.starts_with("http://") || text.starts_with("https://");
                        if is_url_key && is_web && !skip.contains(&text.as_str()) {
                            found.push((text.clone(), here.to_lowercase()));
                        }
                    }
                    other => urls_in(other, skip, &here, found),
                }
            }
        }
        _ => {}
    }
}

/// The clip's URL, if the finished job has one: an `.mp4` first, then
/// anything filed under a key that says video, then anything not filed under a
/// key that says it is a still (thumbnail, preview, poster, image).
pub(crate) fn video_url_of(job: &JobNode) -> Option<String> {
    let skip: Vec<&str> = ["poll_url", "cancel_url"].iter().filter_map(|key| job.get(key)?.as_str()).collect();
    let mut urls = Vec::new();
    urls_in(job, &skip, "", &mut urls);
    let is_mp4 = |url: &str| {
        let lower = url.to_lowercase();
        lower.ends_with(".mp4") || lower.contains(".mp4?")
    };
    let still = |path: &str| ["thumb", "preview", "poster", "image", "cover"].iter().any(|word| path.contains(word));
    urls.iter()
        .find(|(url, _)| is_mp4(url))
        .or_else(|| urls.iter().find(|(_, path)| path.contains("video") || path.contains("clip")))
        .or_else(|| urls.iter().find(|(_, path)| !still(path)))
        .or_else(|| urls.first())
        .map(|(url, _)| url.clone())
}

/// An inline clip, when the job carries one.
fn base64_of(job: &JobNode) -> Option<Vec<u8>> {
    let candidates = [job.get("b64_json"), job.get("video_base64"), job.get("output").and_then(|output| output.get("b64_json"))];
    for candidate in candidates.into_iter().flatten() {
        if let Some(text) = candidate.as_str().filter(|text| !text.is_empty()) {
            return Some(decode_base64_lenient(text));
        }
    }
    match job.get("data") {
        Some(JobNode::Array(items)) => items.first()?.get("b64_json")?.as_str().map(decode_base64_lenient),
        _ => None,
    }
}

#[cfg(test)]
#[path = "videos_tests.rs"]
mod tests;
