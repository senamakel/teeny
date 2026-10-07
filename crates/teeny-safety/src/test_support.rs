//! Test doubles shared by the gate and review tests.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;

use crate::holds::{
    DecisionDetail, Hold, HoldCounts, HoldDecision, HoldInput, HoldLog, HoldStatus, InMemoryHoldLog,
};
use crate::judge::{Judge, JudgeVerdict, Judgement};
use crate::review::{Release, Requeue, ReviewError};
use crate::screen::Direction;
use teeny_store::StoreError;

/// A judge that always answers the same and remembers what it was asked.
pub(crate) struct SpyJudge {
    verdict: JudgeVerdict,
    pub(crate) calls: Mutex<Vec<(String, Direction)>>,
}

impl SpyJudge {
    pub(crate) fn new(verdict: JudgeVerdict) -> Self {
        Self {
            verdict,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn clean() -> Self {
        Self::new(JudgeVerdict {
            verdict: Judgement::Clean,
            category: None,
            reason: String::new(),
        })
    }

    pub(crate) fn prohibited(category: &str, reason: &str) -> Self {
        Self::new(JudgeVerdict {
            verdict: Judgement::Prohibited,
            category: Some(category.into()),
            reason: reason.into(),
        })
    }

    pub(crate) fn calls(&self) -> Vec<(String, Direction)> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl Judge for SpyJudge {
    async fn judge(&self, text: &str, direction: Direction) -> JudgeVerdict {
        self.calls
            .lock()
            .unwrap()
            .push((text.to_owned(), direction));
        self.verdict.clone()
    }
}

/// A memory log that can be told to fail, as Mongo going away would.
#[derive(Default)]
pub(crate) struct FlakyLog {
    pub(crate) inner: InMemoryHoldLog,
    pub(crate) fail_record: AtomicBool,
    pub(crate) fail_allowed_links: AtomicBool,
    /// How many of the next decisions fail.
    pub(crate) fail_decides: std::sync::atomic::AtomicUsize,
}

fn down() -> StoreError {
    StoreError::Io(std::io::Error::other("mongo down"))
}

#[async_trait]
impl HoldLog for FlakyLog {
    async fn record(&self, entry: HoldInput) -> Result<Hold, StoreError> {
        if self.fail_record.load(Ordering::SeqCst) {
            return Err(down());
        }
        self.inner.record(entry).await
    }
    async fn list(
        &self,
        status: Option<HoldStatus>,
        limit: Option<usize>,
    ) -> Result<Vec<Hold>, StoreError> {
        self.inner.list(status, limit).await
    }
    async fn get(&self, id: &str) -> Result<Option<Hold>, StoreError> {
        self.inner.get(id).await
    }
    async fn decide(
        &self,
        id: &str,
        decision: HoldDecision,
        detail: &DecisionDetail,
    ) -> Result<Option<Hold>, StoreError> {
        let failing = self
            .fail_decides
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1));
        if failing.is_ok() {
            return Err(down());
        }
        self.inner.decide(id, decision, detail).await
    }
    async fn claim(&self, id: &str) -> Result<Option<Hold>, StoreError> {
        self.inner.claim(id).await
    }
    async fn unclaim(&self, id: &str, error: &str) -> Result<Option<Hold>, StoreError> {
        self.inner.unclaim(id, error).await
    }
    async fn counts(&self) -> Result<HoldCounts, StoreError> {
        self.inner.counts().await
    }
    async fn allowed_links(&self) -> Result<Vec<String>, StoreError> {
        if self.fail_allowed_links.load(Ordering::SeqCst) {
            return Err(down());
        }
        self.inner.allowed_links().await
    }
    async fn allow_link(&self, entry: &str, hold_id: &str) -> Result<(), StoreError> {
        self.inner.allow_link(entry, hold_id).await
    }
}

/// A queue that answers `requeue` with a fixed answer and remembers the ids.
pub(crate) struct SpyQueue {
    answer: bool,
    pub(crate) calls: Mutex<Vec<String>>,
}

impl SpyQueue {
    pub(crate) fn answering(answer: bool) -> Self {
        Self {
            answer,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl Requeue for SpyQueue {
    async fn requeue(&self, post_id: &str) -> Result<bool, ReviewError> {
        self.calls.lock().unwrap().push(post_id.to_owned());
        Ok(self.answer)
    }
}

/// A publisher whose answer can change between calls.
pub(crate) struct ScriptedRelease {
    pub(crate) answer: Mutex<Result<String, String>>,
}

#[async_trait]
impl Release for ScriptedRelease {
    async fn release(&self, _hold: &Hold) -> Result<String, ReviewError> {
        self.answer
            .lock()
            .unwrap()
            .clone()
            .map_err(ReviewError::Effect)
    }
}

/// A publisher that counts its releases and takes a moment over each, as X
/// does, so two approvals can overlap.
#[derive(Default)]
pub(crate) struct CountingRelease {
    pub(crate) releases: std::sync::atomic::AtomicUsize,
}

#[async_trait]
impl Release for CountingRelease {
    async fn release(&self, _hold: &Hold) -> Result<String, ReviewError> {
        tokio::task::yield_now().await;
        let n = self.releases.fetch_add(1, Ordering::SeqCst) + 1;
        Ok(format!("published on X as {n}"))
    }
}
