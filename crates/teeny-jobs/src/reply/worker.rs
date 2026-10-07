//! Drains the mention queue continuously instead of on a timer: a port of
//! `legacy/src/jobs/reply-worker.ts`.
//!
//! The queue is a plain Mongo collection, so this is a poll loop: straight
//! back round when a batch found work, a short idle wait when it did not, a
//! longer one after a failure so a broken model does not spin. Batches never
//! overlap. The spacing between published replies is the job's pacing, not
//! this loop's, so a large batch cannot reintroduce the burst.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::ctx::JobCtx;

/// One batch of work; `true` when it found some.
#[async_trait]
pub trait Batch: Send + Sync {
    async fn run_batch(&self) -> anyhow::Result<bool>;
}

/// The reply job as a batch.
pub struct ReplyBatch(pub Arc<JobCtx>);

#[async_trait]
impl Batch for ReplyBatch {
    async fn run_batch(&self) -> anyhow::Result<bool> {
        Ok(super::job::run(&self.0).await?.ran)
    }
}

/// The loop, before it starts.
pub struct ReplyWorker {
    batch: Arc<dyn Batch>,
    idle: Duration,
    error: Duration,
}

/// The loop, running.
pub struct Running {
    stop: watch::Sender<bool>,
    handle: JoinHandle<()>,
}

impl ReplyWorker {
    pub fn new(batch: Arc<dyn Batch>, idle: Duration, error: Duration) -> Self {
        Self { batch, idle, error }
    }

    /// The reply job, idling `REPLY_IDLE_MS` and backing off at least five
    /// seconds after a failure.
    pub fn for_replies(ctx: Arc<JobCtx>) -> Self {
        let idle = Duration::from_millis(ctx.config.reply.idle_ms);
        Self::new(
            Arc::new(ReplyBatch(ctx)),
            idle,
            idle.max(Duration::from_secs(5)),
        )
    }

    pub fn start(self) -> Running {
        let (stop, mut stopped) = watch::channel(false);
        tracing::info!(
            idle_ms = self.idle.as_millis() as u64,
            "reply worker draining the queue continuously"
        );
        let handle = tokio::spawn(async move {
            while !*stopped.borrow() {
                let wait = match self.batch.run_batch().await {
                    // Straight back round when there was work.
                    Ok(true) => continue,
                    Ok(false) => self.idle,
                    Err(error) => {
                        tracing::error!(%error, "reply worker iteration failed");
                        self.error
                    }
                };
                // A stop wakes the wait early, so it is immediate rather than
                // up to one idle interval away.
                tokio::select! {
                    () = tokio::time::sleep(wait) => {}
                    _ = stopped.changed() => {}
                }
            }
        });
        Running { stop, handle }
    }
}

impl Running {
    /// Lets the batch in flight finish rather than cutting a reply in half,
    /// without sitting through the idle wait.
    pub async fn stop(self) {
        let _ = self.stop.send(true);
        if let Err(error) = self.handle.await {
            tracing::error!(%error, "the reply worker ended badly");
        }
    }
}

#[cfg(test)]
#[path = "worker_tests.rs"]
mod tests;
