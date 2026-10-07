//! The activity stream reader: X's `GET /2/activity/stream`, one long
//! response of events, each handed to the same [`ActivityIntake`] the
//! webhook route delivers to. The intake's event-id dedupe is what keeps an
//! event both transports carry from being queued twice, which is why the
//! reader and the route share one intake rather than each building theirs.
//!
//! One connection per app (X answers a second with 429), so `serve` runs it
//! in the ingest role only, and only with `X_BEARER_TOKEN`.

use std::sync::Arc;

use futures::{Stream, StreamExt as _};
use serde_json::Value;
use teeny_core::Secret;
use teeny_x::intake::{ActivityIntake, Source};
use teeny_x::stream::ActivityStream;
use tokio::task::JoinHandle;

/// Delivers every event of `events` to `intake`, in order, until the stream
/// ends; how many it read. A delivery that fails is logged and passed: the
/// mention poll catches what the stream drops.
pub async fn feed(events: impl Stream<Item = Value>, intake: Arc<ActivityIntake>) -> usize {
    let mut events = std::pin::pin!(events);
    let mut read = 0;
    while let Some(event) = events.next().await {
        read += 1;
        match intake.deliver(&event, Source::Stream).await {
            Ok(delivered) if delivered.queued > 0 => {
                tracing::info!(queued = delivered.queued, "activity stream: queued mentions");
            }
            Ok(_) => {}
            Err(error) => tracing::warn!(%error, "activity stream: could not take an event"),
        }
    }
    read
}

/// The reader, running.
pub struct StreamTask(JoinHandle<()>);

impl StreamTask {
    /// Connects with `bearer` and feeds `intake` until stopped; the stream
    /// reconnects by itself with X's backoff.
    pub fn start(bearer: Secret, intake: Arc<ActivityIntake>) -> Self {
        Self(tokio::spawn(async move {
            tracing::info!("activity stream: connecting");
            feed(ActivityStream::new(bearer).events(), intake).await;
            tracing::warn!("activity stream: ended");
        }))
    }

    /// Drops the connection.
    pub async fn stop(self) {
        self.0.abort();
        let _ = self.0.await;
        tracing::info!("activity stream: stopped");
    }
}

#[cfg(test)]
#[path = "stream_tests.rs"]
mod tests;
