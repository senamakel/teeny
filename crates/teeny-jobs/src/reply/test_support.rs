//! Fakes for the conversation side's tests: X reads, a memory that records
//! the order of its calls, and the drafter's JSON answers.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use serde_json::{Value, json};
use teeny_core::types::{PostId, UserId};
use teeny_memory::{Experience, Memory, MemoryError, Person};
use teeny_x::cache::XReads;
use teeny_x::model::{OwnPostMetrics, SearchOptions, Trend, TrendOptions, XPost, XUser};
use teeny_x::{XError, XResult};

/// X reads from fixed answers, counting what was asked.
#[derive(Default)]
pub struct FakeX {
    pub root: Option<XPost>,
    pub conversation: Vec<XPost>,
    pub fail: bool,
    pub metrics: Vec<OwnPostMetrics>,
    pub conversation_calls: AtomicUsize,
    pub metric_ids: Mutex<Vec<String>>,
}

fn down() -> XError {
    XError::Missing("rate limited".into())
}

#[async_trait]
impl XReads for FakeX {
    async fn search_posts(&self, _query: &str, _options: &SearchOptions) -> XResult<Vec<XPost>> {
        Ok(Vec::new())
    }
    async fn get_post(&self, _id: &PostId) -> XResult<Option<XPost>> {
        if self.fail {
            return Err(down());
        }
        Ok(self.root.clone())
    }
    async fn get_conversation(
        &self,
        _conversation_id: &PostId,
        _max_results: Option<u32>,
    ) -> XResult<Vec<XPost>> {
        self.conversation_calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(down());
        }
        Ok(self.conversation.clone())
    }
    async fn trends(&self, _options: TrendOptions) -> XResult<Vec<Trend>> {
        Ok(Vec::new())
    }
    async fn post_metrics(&self, ids: &[PostId]) -> XResult<Vec<OwnPostMetrics>> {
        self.metric_ids
            .lock()
            .unwrap()
            .extend(ids.iter().map(ToString::to_string));
        Ok(self.metrics.clone())
    }
    async fn follows_account(&self, _user_id: &UserId) -> Option<bool> {
        None
    }
    async fn user_by_username(&self, _username: &str) -> XResult<Option<XUser>> {
        Ok(None)
    }
}

/// A memory that answers recalls from a script (then `default`), records
/// every call in order, and can be broken outright.
#[derive(Default)]
pub struct ProbeMemory {
    pub answers: Mutex<VecDeque<Result<String, String>>>,
    pub default: String,
    pub broken: bool,
    /// Capturing a reply never finishes, as a process dying after the post
    /// would leave it.
    pub stall_reply_capture: bool,
    /// Capturing a reply fails, as a store that timed out after the post would.
    pub fail_reply_capture: bool,
    /// `recall` and `capture:<kind>` (`mention`, `reply`, `message`), in order.
    pub events: Mutex<Vec<String>>,
    pub queries: Mutex<Vec<(String, Option<Person>)>>,
    pub captured: Mutex<Vec<Experience>>,
}

impl ProbeMemory {
    pub fn answering(answers: &[&str]) -> Self {
        Self {
            answers: Mutex::new(answers.iter().map(|a| Ok((*a).to_owned())).collect()),
            ..Self::default()
        }
    }

    pub fn broken() -> Self {
        Self {
            broken: true,
            ..Self::default()
        }
    }

    pub fn events(&self) -> Vec<String> {
        self.events.lock().unwrap().clone()
    }

    pub fn captured_kinds(&self) -> Vec<String> {
        self.events()
            .into_iter()
            .filter_map(|event| event.strip_prefix("capture:").map(str::to_owned))
            .collect()
    }

    pub fn queries(&self) -> Vec<(String, Option<Person>)> {
        self.queries.lock().unwrap().clone()
    }
}

#[async_trait]
impl Memory for ProbeMemory {
    async fn capture(&self, experience: Experience) -> teeny_memory::Result<()> {
        let kind = experience
            .idempotency_key
            .split(':')
            .nth(1)
            .unwrap_or("")
            .to_owned();
        self.events.lock().unwrap().push(format!("capture:{kind}"));
        if self.stall_reply_capture && kind == "reply" {
            std::future::pending::<()>().await;
        }
        if self.broken || (self.fail_reply_capture && kind == "reply") {
            return Err(MemoryError::InvalidInput("401 MISSING_TOKEN".into()));
        }
        self.captured.lock().unwrap().push(experience);
        Ok(())
    }

    async fn recall(&self, query: &str, person: Option<&Person>) -> teeny_memory::Result<String> {
        self.events.lock().unwrap().push("recall".into());
        self.queries
            .lock()
            .unwrap()
            .push((query.to_owned(), person.cloned()));
        if self.broken {
            return Err(MemoryError::InvalidInput("401 MISSING_TOKEN".into()));
        }
        match self.answers.lock().unwrap().pop_front() {
            Some(Ok(answer)) => Ok(answer),
            Some(Err(error)) => Err(MemoryError::InvalidInput(error)),
            None => Ok(self.default.clone()),
        }
    }
}

/// The reply drafter's JSON answer: `reply` plus whatever `extra` adds.
pub fn reply_answer(text: &str, extra: Value) -> String {
    let mut answer = json!({ "reply": text, "incomingTone": "friendly", "thinking": "" });
    if let (Some(answer), Some(extra)) = (answer.as_object_mut(), extra.as_object()) {
        for (key, value) in extra {
            answer.insert(key.clone(), value.clone());
        }
    }
    answer.to_string()
}

/// A post by `author` (`None`: no author known).
pub fn post(id: &str, text: &str, author: Option<&str>) -> XPost {
    XPost {
        author_id: author.map(UserId::from),
        ..XPost::new(id, text)
    }
}

/// GIFs from a script: the channel's answer, then everyone's.
pub struct ScriptedGifs {
    pub channel: Vec<teeny_media::giphy::Gif>,
    pub wider: Vec<teeny_media::giphy::Gif>,
    pub fail: bool,
    pub searches: Mutex<Vec<(String, bool)>>,
}

pub fn gif(id: &str) -> teeny_media::giphy::Gif {
    teeny_media::giphy::Gif {
        id: id.into(),
        title: id.into(),
        page_url: format!("https://giphy.com/gifs/{id}"),
        media_url: format!("https://media.giphy.com/{id}.gif"),
        mime_type: teeny_media::types::MimeType::Gif,
    }
}

impl ScriptedGifs {
    pub fn new(channel: Vec<teeny_media::giphy::Gif>, wider: Vec<teeny_media::giphy::Gif>) -> Self {
        Self {
            channel,
            wider,
            fail: false,
            searches: Mutex::default(),
        }
    }
}

#[async_trait]
impl teeny_media::giphy::GifSource for ScriptedGifs {
    async fn search(
        &self,
        query: &str,
        channel_only: bool,
    ) -> teeny_media::Result<Vec<teeny_media::giphy::Gif>> {
        self.searches
            .lock()
            .unwrap()
            .push((query.to_owned(), channel_only));
        if self.fail {
            return Err(teeny_media::Error::Failed("giphy is down".into()));
        }
        Ok(if channel_only {
            self.channel.clone()
        } else {
            self.wider.clone()
        })
    }
    async fn get(&self, _id: &str) -> teeny_media::Result<Option<teeny_media::giphy::Gif>> {
        Ok(None)
    }
    async fn download(
        &self,
        _gif: &teeny_media::giphy::Gif,
    ) -> teeny_media::Result<(Vec<u8>, teeny_media::types::MimeType)> {
        Ok((vec![1, 2, 3], teeny_media::types::MimeType::Gif))
    }
}

/// A judge that answers every text the same way and records what it read.
pub struct FixedJudge {
    pub verdict: teeny_safety::judge::JudgeVerdict,
    pub seen: Mutex<Vec<(String, teeny_safety::screen::Direction)>>,
}

impl FixedJudge {
    pub fn clean() -> Self {
        Self::of(teeny_safety::judge::Judgement::Clean, None, "")
    }

    pub fn of(
        verdict: teeny_safety::judge::Judgement,
        category: Option<&str>,
        reason: &str,
    ) -> Self {
        Self {
            verdict: teeny_safety::judge::JudgeVerdict {
                verdict,
                category: category.map(str::to_owned),
                reason: reason.into(),
            },
            seen: Mutex::default(),
        }
    }
}

#[async_trait]
impl teeny_safety::judge::Judge for FixedJudge {
    async fn judge(
        &self,
        text: &str,
        direction: teeny_safety::screen::Direction,
    ) -> teeny_safety::judge::JudgeVerdict {
        self.seen.lock().unwrap().push((text.to_owned(), direction));
        self.verdict.clone()
    }
}

/// A queue that pauses after every read, so two consumers can both read
/// the same pending mention before either acts on it.
pub struct YieldingQueue(
    pub std::sync::Arc<dyn teeny_store::MentionQueue<teeny_agent::deps::QueuedXPost>>,
);

#[async_trait]
impl teeny_store::MentionQueue<teeny_agent::deps::QueuedXPost> for YieldingQueue {
    async fn cache(&self, posts: &[teeny_agent::deps::QueuedXPost]) -> teeny_store::Result<u64> {
        self.0.cache(posts).await
    }
    async fn take_pending(
        &self,
        limit: usize,
    ) -> teeny_store::Result<Vec<teeny_store::QueuedMention<teeny_agent::deps::QueuedXPost>>> {
        let rows = self.0.take_pending(limit).await;
        tokio::task::yield_now().await;
        rows
    }
    async fn claim(
        &self,
        post_id: &str,
    ) -> teeny_store::Result<Option<teeny_store::QueuedMention<teeny_agent::deps::QueuedXPost>>>
    {
        self.0.claim(post_id).await
    }
    async fn record_reply_id(&self, post_id: &str, reply_id: &str) -> teeny_store::Result<()> {
        self.0.record_reply_id(post_id, reply_id).await
    }
    async fn mark_replied(
        &self,
        post_id: &str,
        reply: &teeny_store::Reply,
    ) -> teeny_store::Result<()> {
        self.0.mark_replied(post_id, reply).await
    }
    async fn mark_failed(&self, post_id: &str, error: &str) -> teeny_store::Result<()> {
        self.0.mark_failed(post_id, error).await
    }
    async fn mark_skipped(&self, post_id: &str, reason: &str) -> teeny_store::Result<()> {
        self.0.mark_skipped(post_id, reason).await
    }
    async fn mark_liked(&self, post_id: &str) -> teeny_store::Result<()> {
        self.0.mark_liked(post_id).await
    }
    async fn count_replied(&self, filter: &teeny_store::RepliedFilter) -> teeny_store::Result<u64> {
        self.0.count_replied(filter).await
    }
    async fn requeue_approved(&self, post_id: &str) -> teeny_store::Result<bool> {
        self.0.requeue_approved(post_id).await
    }
    async fn requeue_fresh(&self, post_id: &str) -> teeny_store::Result<bool> {
        self.0.requeue_fresh(post_id).await
    }
    async fn status_of(
        &self,
        post_id: &str,
    ) -> teeny_store::Result<Option<teeny_store::MentionStatus>> {
        self.0.status_of(post_id).await
    }
    async fn counts(&self) -> teeny_store::Result<teeny_store::StatusCounts> {
        self.0.counts().await
    }
}
