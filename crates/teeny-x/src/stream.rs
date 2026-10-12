//! The X Activity API's other transport, ported from
//! `legacy/src/clients/activity-stream.ts`: `GET /2/activity/stream`, one
//! long HTTP response carrying every event the app's subscriptions match, as
//! newline-delimited JSON with a bare newline every 20 seconds to keep it
//! alive. The envelope is the one the webhook POSTs, so each item goes to the
//! same [`crate::intake::ActivityIntake`].
//!
//! [`ActivityStream::events`] is a `Stream` that reconnects by itself, with
//! the backoff X's streaming guidance describes: linear on a dropped
//! connection, exponential on an HTTP error, longest on a 429. A clean end of
//! stream (X closes connections from time to time) is a reconnect, not a
//! failure. Dropping the stream stops it.
//!
//! One connection per app: X answers a second with 429, so this runs in the
//! ingest role only.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use futures::stream::{self, BoxStream, Stream, StreamExt};
use serde_json::Value;
use teeny_core::Secret;

use crate::client::X_API_BASE;

/// Waits between attempts; injectable so a test does not sleep.
pub type Sleeper = Arc<dyn Fn(Duration) -> BoxFuture<'static, ()> + Send + Sync>;

/// Why a connection attempt failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    /// The connection could not be made, or broke.
    Network,
    /// X answered with this status.
    Http(u16),
}

/// How long to wait before attempt `attempt + 1`, after `attempt`
/// consecutive failures (1-based): at least a minute on a 429, doubling to
/// 15; from 5 seconds doubling to 5 minutes on another HTTP error; 250 ms
/// more per attempt, to 16 seconds, on a dropped connection.
pub fn backoff(failure: Failure, attempt: u32) -> Duration {
    let doubling = |base: Duration, cap: Duration| {
        let factor = 2u32.saturating_pow(attempt.saturating_sub(1));
        base.checked_mul(factor).map_or(cap, |delay| delay.min(cap))
    };
    match failure {
        Failure::Http(429) => doubling(Duration::from_secs(60), Duration::from_secs(15 * 60)),
        Failure::Http(_) => doubling(Duration::from_secs(5), Duration::from_secs(5 * 60)),
        Failure::Network => (Duration::from_millis(250) * attempt).min(Duration::from_secs(16)),
    }
}

/// Bytes in, whole non-blank lines out; a line split across chunks waits for
/// its end.
#[derive(Debug, Default)]
pub struct LineBuffer {
    pending: Vec<u8>,
}

impl LineBuffer {
    pub fn push(&mut self, chunk: &[u8]) -> Vec<String> {
        self.pending.extend_from_slice(chunk);
        let mut lines = Vec::new();
        while let Some(newline) = self.pending.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=newline).collect();
            let line = String::from_utf8_lossy(&line).trim().to_owned();
            // A blank line is the keep-alive.
            if !line.is_empty() {
                lines.push(line);
            }
        }
        lines
    }
}

/// An event envelope (`{ data: ... }`) from one line, or nothing. X reports
/// operational problems inline as `{ errors: [...] }` and then usually
/// closes the stream; that is logged, and the close is what reconnects.
pub fn parse_line(line: &str) -> Option<Value> {
    let parsed: Value = match serde_json::from_str(line) {
        Ok(parsed) => parsed,
        Err(_) => {
            let start: String = line.chars().take(200).collect();
            tracing::warn!(line = start, "activity stream sent a line that is not JSON");
            return None;
        }
    };
    let object = parsed.as_object()?;
    if let Some(errors) = object.get("errors").filter(|errors| errors.is_array()) {
        tracing::warn!(%errors, "activity stream reported errors");
        return None;
    }
    object.get("data").filter(|data| !data.is_null())?;
    Some(parsed)
}

/// A reader of `GET /2/activity/stream`.
pub struct ActivityStream {
    http: reqwest::Client,
    url: String,
    bearer: Secret,
    sleeper: Sleeper,
    max_attempts: u32,
}

impl ActivityStream {
    /// A reader on the app bearer (`X_BEARER_TOKEN`).
    pub fn new(bearer: Secret) -> Self {
        Self {
            http: reqwest::Client::new(),
            url: format!("{X_API_BASE}/2/activity/stream"),
            bearer,
            sleeper: Arc::new(|delay| Box::pin(tokio::time::sleep(delay))),
            max_attempts: 0,
        }
    }

    pub fn with_url(mut self, url: impl Into<String>) -> Self {
        self.url = url.into();
        self
    }

    pub fn with_sleeper(mut self, sleeper: Sleeper) -> Self {
        self.sleeper = sleeper;
        self
    }

    /// Give up after this many consecutive failures; 0 (the default) never
    /// does. When it gives up, the cron poll is all that reads X.
    pub fn with_max_attempts(mut self, max_attempts: u32) -> Self {
        self.max_attempts = max_attempts;
        self
    }

    /// Every event envelope X sends, for as long as the stream is polled.
    pub fn events(self) -> impl Stream<Item = Value> + Send {
        let state = State {
            reader: self,
            body: None,
            buffer: LineBuffer::default(),
            pending: VecDeque::new(),
            attempt: 0,
            done: false,
        };
        stream::unfold(state, |mut state| async move {
            let event = state.next_event().await?;
            Some((event, state))
        })
    }
}

struct State {
    reader: ActivityStream,
    body: Option<BoxStream<'static, reqwest::Result<Vec<u8>>>>,
    buffer: LineBuffer,
    pending: VecDeque<Value>,
    attempt: u32,
    done: bool,
}

impl State {
    async fn next_event(&mut self) -> Option<Value> {
        loop {
            if let Some(event) = self.pending.pop_front() {
                return Some(event);
            }
            if self.done {
                return None;
            }
            match self.body.as_mut() {
                Some(body) => match body.next().await {
                    Some(Ok(chunk)) => self.pending.extend(
                        self.buffer
                            .push(&chunk)
                            .iter()
                            .filter_map(|line| parse_line(line)),
                    ),
                    Some(Err(error)) => {
                        self.body = None;
                        tracing::error!(%error, "activity stream broke");
                        self.fail(Failure::Network).await;
                    }
                    None => {
                        self.body = None;
                        self.attempt = 0;
                        tracing::warn!("activity stream ended; reconnecting");
                    }
                },
                None => self.connect().await,
            }
        }
    }

    async fn connect(&mut self) {
        let response = self
            .reader
            .http
            .get(&self.reader.url)
            .bearer_auth(self.reader.bearer.expose())
            .send()
            .await;
        match response {
            Ok(response) if response.status().is_success() => {
                tracing::info!("activity stream connected");
                self.attempt = 0;
                self.buffer = LineBuffer::default();
                self.body = Some(
                    response
                        .bytes_stream()
                        .map(|chunk| chunk.map(|bytes| bytes.to_vec()))
                        .boxed(),
                );
            }
            Ok(response) => {
                let status = response.status().as_u16();
                let body: String = response
                    .text()
                    .await
                    .unwrap_or_default()
                    .chars()
                    .take(300)
                    .collect();
                tracing::error!(status, body, "activity stream refused");
                self.fail(Failure::Http(status)).await;
            }
            Err(error) => {
                tracing::error!(%error, "activity stream could not connect");
                self.fail(Failure::Network).await;
            }
        }
    }

    async fn fail(&mut self, failure: Failure) {
        self.attempt += 1;
        if self.reader.max_attempts > 0 && self.attempt >= self.reader.max_attempts {
            tracing::error!(
                attempts = self.attempt,
                "activity stream gave up; the cron poll is all that reads X now"
            );
            self.done = true;
            return;
        }
        let delay = backoff(failure, self.attempt);
        tracing::info!(
            attempt = self.attempt,
            delay_ms = delay.as_millis() as u64,
            "activity stream backing off"
        );
        (self.reader.sleeper)(delay).await;
    }
}

#[cfg(test)]
#[path = "stream_tests.rs"]
mod tests;
