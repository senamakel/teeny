//! Test doubles shared by the gate and review tests.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;

use crate::holds::{
    Decision, Hold, HoldCounts, HoldInput, HoldLog, HoldLogError, HoldStatus, MemoryHoldLog,
};
use crate::judge::{Judge, JudgeVerdict, Judgement};
use crate::review::{Release, Requeue, ReviewError};
use crate::screen::Direction;

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
    pub(crate) inner: MemoryHoldLog,
    pub(crate) fail_record: AtomicBool,
    pub(crate) fail_allowed_links: AtomicBool,
}

fn down() -> HoldLogError {
    HoldLogError::new("mongo down")
}

#[async_trait]
impl HoldLog for FlakyLog {
    async fn record(&self, entry: HoldInput) -> Result<Hold, HoldLogError> {
        if self.fail_record.load(Ordering::SeqCst) {
            return Err(down());
        }
        self.inner.record(entry).await
    }
    async fn list(
        &self,
        status: Option<HoldStatus>,
        limit: Option<usize>,
    ) -> Result<Vec<Hold>, HoldLogError> {
        self.inner.list(status, limit).await
    }
    async fn get(&self, id: &str) -> Result<Option<Hold>, HoldLogError> {
        self.inner.get(id).await
    }
    async fn decide(
        &self,
        id: &str,
        decision: Decision,
        note: Option<String>,
        effect: Option<String>,
    ) -> Result<Option<Hold>, HoldLogError> {
        self.inner.decide(id, decision, note, effect).await
    }
    async fn counts(&self) -> Result<HoldCounts, HoldLogError> {
        self.inner.counts().await
    }
    async fn allowed_links(&self) -> Result<Vec<String>, HoldLogError> {
        if self.fail_allowed_links.load(Ordering::SeqCst) {
            return Err(down());
        }
        self.inner.allowed_links().await
    }
    async fn allow_link(&self, entry: &str, hold_id: &str) -> Result<(), HoldLogError> {
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
