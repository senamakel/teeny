//! The mention poll: reads X and writes to the queue, nothing else. A port
//! of `legacy/src/jobs/ingest-job.ts`.
//!
//! This half has the deadline: X's recent search reaches back seven days
//! and every post retrieved is billed, so it runs often, asks only for what
//! arrived since the last poll, and hands the result to the queue. Whether a
//! reply is ever written is the reply job's business.
//!
//! The webhook and the activity stream feed the same queue through
//! [`QueueSink`], and the queue keeps one row per post id, so a mention that
//! arrives on a transport and in the poll is queued once.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, TimeDelta, Utc};
use teeny_agent::deps::QueuedXPost;
use teeny_cadence::clock::{iso, parse_iso};
use teeny_core::types::{PostId, UserId};
use teeny_store::journal::Outcome;
use teeny_store::{BotState, MentionQueue};
use teeny_x::intake::{AccountId, IntakeError, MentionSink};
use teeny_x::mention::is_worth_answering;
use teeny_x::model::{MentionWindow, WindowStart, XPost};
use teeny_x::{XApi, XResult};

use crate::ctx::{JobCtx, JobReport};

/// The job's name in reports.
pub const JOB: &str = "ingest";

/// The overlap with the previous poll, so a post indexed late is not missed.
const OVERLAP: TimeDelta = TimeDelta::minutes(1);
/// How far back X's recent search reaches.
const SEVEN_DAYS: TimeDelta = TimeDelta::days(7);

/// The account id [`crate::ctx::Conversation::fake`] answers with.
pub const FAKE_ACCOUNT_ID: &str = "2094854245468000256";

/// The mention search. [`XApi`] asks X; tests script it.
#[async_trait]
pub trait MentionSource: Send + Sync {
    async fn find_mentions(&self, window: &MentionWindow) -> XResult<Vec<XPost>>;
}

#[async_trait]
impl MentionSource for XApi {
    async fn find_mentions(&self, window: &MentionWindow) -> XResult<Vec<XPost>> {
        XApi::find_mentions(self, window).await
    }
}

/// Answers every poll with the same posts and records each window asked.
#[derive(Default)]
pub struct ScriptedMentions {
    found: Vec<XPost>,
    windows: Mutex<Vec<MentionWindow>>,
}

impl ScriptedMentions {
    pub fn new(found: Vec<XPost>) -> Self {
        Self {
            found,
            windows: Mutex::default(),
        }
    }

    /// Every window asked for, in order.
    pub fn windows(&self) -> Vec<MentionWindow> {
        self.windows
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }
}

#[async_trait]
impl MentionSource for ScriptedMentions {
    async fn find_mentions(&self, window: &MentionWindow) -> XResult<Vec<XPost>> {
        self.windows
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .push(window.clone());
        Ok(self.found.clone())
    }
}

/// An account whose id is known in advance (or known to be unknown).
pub struct FixedAccount(Option<UserId>);

impl FixedAccount {
    pub fn new(id: &str) -> Self {
        Self(Some(UserId::from(id)))
    }

    /// An account that cannot say who it is.
    pub fn unknown() -> Self {
        Self(None)
    }
}

#[async_trait]
impl AccountId for FixedAccount {
    async fn bot_user_id(&self) -> Result<Option<UserId>, IntakeError> {
        Ok(self.0.clone())
    }
}

/// The mention queue as the webhook / stream intake's sink, so every
/// transport and the poll share one queue and its per-post dedupe.
pub struct QueueSink(pub Arc<dyn MentionQueue<QueuedXPost>>);

#[async_trait]
impl MentionSink for QueueSink {
    async fn cache(&self, mentions: Vec<XPost>) -> Result<usize, IntakeError> {
        let posts: Vec<QueuedXPost> = mentions.into_iter().map(QueuedXPost).collect();
        Ok(self.0.cache(&posts).await? as usize)
    }
}

/// X v2 rejects `since_id` with `start_time`, so this picks one: the cursor
/// when there is one, a window from the last poll otherwise, which stops a
/// quiet account re-reading seven days a cycle.
fn window(
    state: &BotState,
    now: DateTime<Utc>,
    lookback_minutes: u32,
    max_results: u32,
) -> MentionWindow {
    let max_results = Some(max_results);
    if let Some(since_id) = &state.since_id {
        return MentionWindow {
            max_results,
            start: WindowStart::SinceId(PostId::from(since_id.as_str())),
        };
    }
    let from = match state.last_polled_at.as_deref().and_then(parse_iso) {
        Some(previous) => previous - OVERLAP,
        None => now - TimeDelta::minutes(i64::from(lookback_minutes)),
    };
    let floor = now - SEVEN_DAYS + OVERLAP;
    MentionWindow {
        max_results,
        start: WindowStart::StartTime(iso(from.max(floor))),
    }
}

/// Post ids are numbers too wide for an `f64`; compared as integers, or as
/// text when one does not parse.
fn newer(candidate: &str, than: &str) -> bool {
    match (candidate.parse::<u128>(), than.parse::<u128>()) {
        (Ok(a), Ok(b)) => a > b,
        _ => candidate > than,
    }
}

/// One poll: read what arrived, queue what is close enough to answer, move
/// the cursor past everything read.
pub async fn run(ctx: &JobCtx) -> anyhow::Result<JobReport> {
    let conversation = ctx.conversation()?;
    let state = conversation.cursor.load().await?;
    let now = ctx.now();
    let window = window(
        &state,
        now,
        ctx.config.x.mention_lookback_minutes,
        ctx.config.x.search_max_results,
    );
    let polled_at = iso(now);
    let retrieved = conversation.mentions.find_mentions(&window).await?;

    // Direct replies to the account, plus anything further away where
    // somebody typed the handle. What is dropped is the thread the account
    // was tagged into once and then carried along in every reply after.
    let bot_user_id = conversation
        .account
        .bot_user_id()
        .await
        .map_err(|error| anyhow::anyhow!("could not identify the account: {error}"))?
        .filter(|id| !id.as_str().is_empty());
    let bot_username = ctx.config.x.bot_username.trim_start_matches('@');
    let shallow: Vec<QueuedXPost> = retrieved
        .iter()
        .filter(|post| {
            !(bot_user_id.is_some() && post.author_id == bot_user_id)
                && is_worth_answering(post, bot_user_id.as_ref(), bot_username)
        })
        .cloned()
        .map(QueuedXPost)
        .collect();
    let cached = conversation.queue.cache(&shallow).await?;

    // The cursor advances past everything read, not past everything
    // answered: the queue owns the reply work from here.
    let newest = retrieved
        .iter()
        .fold(state.since_id.clone(), |highest, post| match highest {
            Some(highest) if !newer(post.id.as_str(), &highest) => Some(highest),
            _ => Some(post.id.to_string()),
        });
    conversation
        .cursor
        .save(&BotState {
            last_polled_at: Some(polled_at),
            since_id: newest,
            ..state
        })
        .await?;

    let note = format!(
        "retrieved {}, too deep {}, cached {cached}",
        retrieved.len(),
        retrieved.len() - shallow.len()
    );
    tracing::info!(
        retrieved = retrieved.len(),
        too_deep = retrieved.len() - shallow.len(),
        cached,
        ?window,
        "ingested X mentions"
    );
    Ok(JobReport {
        note: Some(note),
        ..JobReport::ran(JOB, Outcome::Skipped)
    })
}

#[cfg(test)]
#[path = "ingest_tests.rs"]
mod tests;
