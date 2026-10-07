//! Which budget a reply comes out of, and whether it has room.
//!
//! `Auto` is the account answering under its own post: somebody replied to
//! the morning post and the account replies back. `Smart` is the reply guy:
//! a mention or a quote out in somebody else's timeline. They are capped
//! separately (`REPLY_AUTO_PER_POST`, `REPLY_SMART_PER_POST`) because they do
//! different jobs: the first keeps a seen thread alive, the second puts the
//! account in front of people who have not seen it.
//!
//! `Called` is somebody typing the handle on purpose, out in the world: a
//! question asked to the account's face, answered whatever the day's budget
//! says. It has no cap. A reply under one of the account's own posts is never
//! `Called`, however many times the handle appears in it.
//!
//! The held slots of a hot post are not lost, they move: the post the
//! timeline is kept quiet for gets each held slot's auto and smart allowance
//! added to its own thread, which is where the attention is.
//! A port of `legacy/src/cadence/budget.ts`.

use std::marker::PhantomData;
use std::sync::{Arc, LazyLock};

use async_trait::async_trait;
use chrono::TimeDelta;
use regex::Regex;
use teeny_core::Config;
use teeny_store::mentions::{MentionQueue, QueuedPost, RepliedFilter, ReplyLane};
use teeny_store::posts::{OwnPost, PostLedger};

use crate::clock::{Clock, hours, iso, parse_iso};

/// What the budget needs to know about a post it might answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mention {
    pub text: String,
    /// The root post's id; a root post is its own conversation on X.
    pub conversation_id: Option<String>,
    /// The parent's author, when the post is a reply.
    pub in_reply_to_user_id: Option<String>,
}

/// The budget's answer for one post.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assessment {
    pub lane: ReplyLane,
    /// For the auto lane, the own post the reply would sit under.
    pub own_post_id: Option<String>,
    pub allowed: bool,
    pub used: u64,
    /// `None` is uncapped (the called lane): `used` is for the record.
    pub cap: Option<u64>,
}

/// Counts the replies already sent out of a lane.
#[async_trait]
pub trait RepliedCounter: Send + Sync {
    async fn count_replied(&self, filter: &RepliedFilter) -> teeny_store::Result<u64>;
}

/// Any teeny-store mention queue, as the budget's counter.
pub struct QueueCounter<P, Q: ?Sized> {
    queue: Arc<Q>,
    post: PhantomData<fn() -> P>,
}

impl<P, Q: ?Sized> QueueCounter<P, Q> {
    pub fn new(queue: Arc<Q>) -> Self {
        Self {
            queue,
            post: PhantomData,
        }
    }
}

#[async_trait]
impl<P: QueuedPost, Q: MentionQueue<P> + ?Sized> RepliedCounter for QueueCounter<P, Q> {
    async fn count_replied(&self, filter: &RepliedFilter) -> teeny_store::Result<u64> {
        self.queue.count_replied(filter).await
    }
}

/// The allowances.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetRules {
    /// Replies the account writes under each of its own posts.
    pub auto_per_post: u32,
    /// Replies out in the world, per own post published in the last day.
    pub smart_per_post: u32,
    /// One slot of the cadence, to count how many a hot post's hold took.
    pub gap: TimeDelta,
    /// The account's handle, to tell a deliberate mention from an inherited one.
    pub bot_username: Option<String>,
    /// How long an own post keeps attracting replies worth answering.
    pub thread_window: TimeDelta,
}

impl BudgetRules {
    /// `REPLY_AUTO_PER_POST`, `REPLY_SMART_PER_POST`, `POST_MIN_GAP_HOURS`
    /// and `X_BOT_USERNAME`, with the legacy week-long thread window.
    pub fn from_config(config: &Config) -> Self {
        Self {
            auto_per_post: config.reply.auto_per_post,
            smart_per_post: config.reply.smart_per_post,
            gap: hours(config.cadence.post_min_gap_hours),
            bot_username: Some(config.x.bot_username.clone()),
            thread_window: TimeDelta::days(7),
        }
    }
}

/// The reply budget.
pub struct ReplyBudget {
    ledger: Arc<dyn PostLedger>,
    queue: Arc<dyn RepliedCounter>,
    rules: BudgetRules,
    clock: Arc<dyn Clock>,
}

impl ReplyBudget {
    pub fn new(
        ledger: Arc<dyn PostLedger>,
        queue: Arc<dyn RepliedCounter>,
        rules: BudgetRules,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            ledger,
            queue,
            rules,
            clock,
        }
    }

    async fn count(
        &self,
        lane: ReplyLane,
        own_post_id: Option<String>,
        since: Option<String>,
    ) -> teeny_store::Result<u64> {
        self.queue
            .count_replied(&RepliedFilter {
                lane,
                own_post_id,
                since,
            })
            .await
    }

    /// Which lane a post would be answered from, and whether it has room.
    pub async fn assess(
        &self,
        post: &Mention,
        bot_user_id: &str,
    ) -> teeny_store::Result<Assessment> {
        let now = self.clock.now();
        let own = self
            .ledger
            .since(&iso(now - self.rules.thread_window), 200)
            .await?;
        let (auto, smart) = (
            u64::from(self.rules.auto_per_post),
            u64::from(self.rules.smart_per_post),
        );

        if let Some(under) = own_post_of(post, bot_user_id, &own) {
            let used = self
                .count(ReplyLane::Auto, Some(under.post_id.clone()), None)
                .await?;
            let cap = auto + held_slots(under, self.rules.gap) * (auto + smart);
            return Ok(Assessment {
                lane: ReplyLane::Auto,
                own_post_id: Some(under.post_id.clone()),
                allowed: used < cap,
                used,
                cap: Some(cap),
            });
        }

        let day_ago = iso(now - TimeDelta::hours(24));
        if let Some(bot) = &self.rules.bot_username
            && mentions_deliberately(&post.text, bot)
        {
            let used = self.count(ReplyLane::Called, None, Some(day_ago)).await?;
            return Ok(Assessment {
                lane: ReplyLane::Called,
                own_post_id: None,
                allowed: true,
                used,
                cap: None,
            });
        }

        let posts_today = own
            .iter()
            .filter(|entry| entry.posted_at >= day_ago)
            .count() as u64;
        let used = self.count(ReplyLane::Smart, None, Some(day_ago)).await?;
        // A quiet day still gets one post's worth: the reply guy is how a day
        // with nothing to say still talks to somebody.
        let cap = smart * posts_today.max(1);
        Ok(Assessment {
            lane: ReplyLane::Smart,
            own_post_id: None,
            allowed: used < cap,
            used,
            cap: Some(cap),
        })
    }
}

/// The own post a reply sits under, if it is one. A root post is its own
/// conversation on X, so the conversation id names it, and a reply whose
/// conversation is not one of the account's posts is out in somebody else's
/// thread. Only a delivery with no conversation at all (the old Account
/// Activity shape) that replies to the account is taken to be about the
/// latest post, which is where nearly all of them land.
pub fn own_post_of<'a>(
    post: &Mention,
    bot_user_id: &str,
    own: &'a [OwnPost],
) -> Option<&'a OwnPost> {
    match &post.conversation_id {
        Some(conversation) => own.iter().find(|entry| &entry.post_id == conversation),
        None if post.in_reply_to_user_id.as_deref() == Some(bot_user_id) => own.first(),
        None => None,
    }
}

/// How many slots a hot post's hold spans, counted from when it first ran hot.
pub fn held_slots(post: &OwnPost, gap: TimeDelta) -> u64 {
    let Some(hot) = &post.hot else { return 0 };
    let (Some(since), Some(until)) = (parse_iso(&hot.since), parse_iso(&hot.hold_until)) else {
        return 0;
    };
    let span = (until - since).num_milliseconds();
    let gap = gap.num_milliseconds().max(1);
    u64::try_from((span.max(0) + gap - 1) / gap).unwrap_or_default()
}

static RETWEET: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^RT @[A-Za-z0-9_]{1,15}:").expect("valid"));
static LEADING_HANDLES: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:\s*@[A-Za-z0-9_]{1,15}\b)+").expect("valid"));

/// Whether a human deliberately typed the account's handle, rather than X
/// carrying it along.
///
/// Replying puts every earlier participant's handle at the front of the text,
/// so a thread the account was tagged into once mentions it in every later
/// reply, none addressed to it. The leading run is that machinery; anything
/// after it was typed on purpose. "RT @x:" is a retweet, not an address.
/// A port of `mentionsDeliberately` in `legacy/src/types.ts`.
pub fn mentions_deliberately(text: &str, bot_username: &str) -> bool {
    if RETWEET.is_match(text) {
        return false;
    }
    let handle = bot_username.trim_start_matches('@');
    let body = LEADING_HANDLES.replace(text, "");
    Regex::new(&format!(r"(?i)@{}\b", regex::escape(handle)))
        .is_ok_and(|pattern| pattern.is_match(&body))
}

#[cfg(test)]
#[path = "budget_tests.rs"]
mod tests;
