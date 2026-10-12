//! Fakes and a request helper for the route tests.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, Bytes};
use axum::http::{HeaderMap, Request, StatusCode, header};
use http_body_util::BodyExt as _;
use serde_json::Value;
use teeny_x::activity::ActivityEntry;
use teeny_x::intake::{ActivityIntake, ActivityMemory, IntakeError};
use tower::ServiceExt as _;

use super::Services;

pub const SECRET: &str = "secret";
pub const TOKEN: &str = "a-token-long-enough-to-pass";

/// What a route answered.
pub struct Answer {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Bytes,
}

impl Answer {
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|error| panic!("not JSON ({error}): {}", self.text()))
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

pub async fn call(app: &Router, request: Request<Body>) -> Answer {
    let response = app.clone().oneshot(request).await.expect("infallible");
    let status = response.status();
    let headers = response.headers().clone();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("a body")
        .to_bytes();
    Answer {
        status,
        headers,
        body,
    }
}

pub fn get(uri: &str) -> Request<Body> {
    Request::get(uri).body(Body::empty()).unwrap()
}

pub fn get_as(uri: &str, authorization: &str) -> Request<Body> {
    Request::get(uri)
        .header(header::AUTHORIZATION, authorization)
        .body(Body::empty())
        .unwrap()
}

pub fn post_json(uri: &str, body: &str) -> Request<Body> {
    Request::post(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_owned()))
        .unwrap()
}

pub fn post_json_as(uri: &str, authorization: &str, body: &str) -> Request<Body> {
    Request::post(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, authorization)
        .body(Body::from(body.to_owned()))
        .unwrap()
}

pub fn bearer() -> String {
    format!("Bearer {TOKEN}")
}

/// Memory that keeps what it was given.
#[derive(Default)]
pub struct RecordingMemory(pub Mutex<Vec<ActivityEntry>>);

#[async_trait]
impl ActivityMemory for RecordingMemory {
    async fn capture_activity(&self, entry: ActivityEntry) -> Result<(), IntakeError> {
        self.0.lock().unwrap().push(entry);
        Ok(())
    }
}

/// A server with nothing configured but the webhook.
pub fn services() -> Services {
    Services::new(
        SECRET,
        Arc::new(ActivityIntake::new(Arc::new(RecordingMemory::default()))),
    )
}

/// A directory of its own under the system temp dir, removed on drop.
pub struct Scratch(pub std::path::PathBuf);

impl Scratch {
    pub fn new(name: &str) -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("teeny-http-{name}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    /// Writes `contents` at `relative`, making the directories on the way.
    pub fn write(&self, relative: &str, contents: impl AsRef<[u8]>) -> std::path::PathBuf {
        let path = self.0.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, contents).unwrap();
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
