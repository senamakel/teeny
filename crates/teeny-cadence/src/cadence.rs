//! The content calendar's one rule: the account's own posts are spaced out,
//! and a post that is taking off gets the timeline to itself.
//!
//! Every job that posts to the timeline asks here first, and the Publisher
//! guard routes every own post through [`Cadence::publish_own`], so a post
//! nobody asked about is refused anyway. Two reasons to say no:
//!
//! - the gap: the last own post was less than `POST_MIN_GAP_HOURS` ago (less
//!   `POST_GAP_TOLERANCE_MINUTES`, because the jobs fire on the hour and then
//!   spend minutes drawing before they post). Posts an hour apart are the
//!   repetitive-publishing shape the ranker demotes.
//! - the hold: the heat job saw a post's impressions climbing and held the
//!   slots after it. Posting into that competes with yourself; the reply
//!   budget spends the slot on the hot post's thread instead.
//!
//! Replies are never subject to this; they are conversations, not slots.
//! A port of `legacy/src/cadence/cadence.ts`.

use std::fmt;
use std::sync::Arc;

use chrono::TimeDelta;
use teeny_core::config::CadenceConfig;
use teeny_store::StoreError;
use teeny_store::posts::{NewOwnPost, PostLedger};

use crate::clock::{Clock, hours, iso, parse_iso};

/// One own post at a time, across the process: claim, publish and record
/// run under it, so two jobs firing in the same minute (the cron runs them
/// in parallel) cannot both read the ledger before either is recorded and
/// both pass the gap. Static rather than per [`Cadence`] so a second
/// instance over the same ledger is no way round it.
static POSTING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The cadence's answer to "may I post now?".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Claim {
    Granted,
    /// Too soon after the last own post.
    Gap {
        last_post_id: String,
        last_posted_at: String,
        until: String,
    },
    /// The timeline is held for a post that is taking off.
    Hot {
        hot_post_id: String,
        until: String,
    },
}

impl Claim {
    pub fn is_granted(&self) -> bool {
        matches!(self, Self::Granted)
    }
}

/// The spacing rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CadenceRules {
    /// The least time between two own posts.
    pub min_gap: TimeDelta,
    /// How much short of the gap still counts as the gap.
    pub tolerance: TimeDelta,
}

impl CadenceRules {
    /// `POST_MIN_GAP_HOURS` and `POST_GAP_TOLERANCE_MINUTES`.
    pub fn from_config(config: &CadenceConfig) -> Self {
        Self {
            min_gap: hours(config.post_min_gap_hours),
            tolerance: hours(config.post_gap_tolerance_minutes / 60.0),
        }
    }
}

/// An own post refused by the cadence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CadenceHeld(pub Claim);

impl fmt::Display for CadenceHeld {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            Claim::Gap {
                last_post_id,
                last_posted_at,
                until,
            } => write!(
                f,
                "cadence: too soon after post {last_post_id} ({last_posted_at}); next slot at {until}"
            ),
            Claim::Hot { hot_post_id, until } => {
                write!(
                    f,
                    "cadence: the timeline is held for hot post {hot_post_id} until {until}"
                )
            }
            Claim::Granted => write!(f, "cadence: granted"),
        }
    }
}

impl std::error::Error for CadenceHeld {}

/// Why a paced own post did not go out.
#[derive(Debug)]
pub enum PublishError<E> {
    /// The cadence said no; nothing was published.
    Held(CadenceHeld),
    /// The ledger could not be read before publishing.
    Ledger(StoreError),
    /// Publishing itself failed.
    Publish(E),
}

impl<E: fmt::Display> fmt::Display for PublishError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Held(held) => held.fmt(f),
            Self::Ledger(error) => write!(f, "cadence: {error}"),
            Self::Publish(error) => error.fmt(f),
        }
    }
}

impl<E: fmt::Debug + fmt::Display> std::error::Error for PublishError<E> {}

/// What a publish returns, as far as the ledger cares: the own post's id.
pub trait OwnPostId {
    fn own_post_id(&self) -> &str;
}

impl OwnPostId for String {
    fn own_post_id(&self) -> &str {
        self
    }
}

impl OwnPostId for teeny_core::types::PostId {
    fn own_post_id(&self) -> &str {
        self.as_str()
    }
}

/// The cadence, over the own-post ledger.
pub struct Cadence {
    ledger: Arc<dyn PostLedger>,
    rules: CadenceRules,
    clock: Arc<dyn Clock>,
}

impl Cadence {
    pub fn new(ledger: Arc<dyn PostLedger>, rules: CadenceRules, clock: Arc<dyn Clock>) -> Self {
        Self {
            ledger,
            rules,
            clock,
        }
    }

    /// Whether an own post of this kind may go out now.
    pub async fn claim(&self, kind: &str) -> teeny_store::Result<Claim> {
        let now = self.clock.now();
        if let Some(post) = self.ledger.hold(&iso(now)).await?
            && let Some(hot) = post.hot
        {
            let claim = Claim::Hot {
                hot_post_id: post.post_id,
                until: hot.hold_until,
            };
            tracing::info!(kind, ?claim, "cadence: the timeline is held for a hot post");
            return Ok(claim);
        }
        if let Some(last) = self.ledger.latest().await?
            && let Some(posted) = parse_iso(&last.posted_at)
        {
            let since = now - posted;
            if since + self.rules.tolerance < self.rules.min_gap {
                let claim = Claim::Gap {
                    last_post_id: last.post_id,
                    last_posted_at: last.posted_at,
                    until: iso(posted + self.rules.min_gap),
                };
                tracing::info!(
                    kind,
                    since_ms = since.num_milliseconds(),
                    ?claim,
                    "cadence: too soon after the last post"
                );
                return Ok(claim);
            }
        }
        Ok(Claim::Granted)
    }

    /// Records an own post as published now.
    pub async fn record(
        &self,
        post_id: &str,
        kind: &str,
        text: Option<&str>,
    ) -> teeny_store::Result<()> {
        let post = NewOwnPost {
            post_id: post_id.to_owned(),
            kind: kind.to_owned(),
            posted_at: iso(self.clock.now()),
            text: text.map(str::to_owned),
        };
        self.ledger.record(&post).await
    }

    /// Publishes an own post (or an article, which puts one on the timeline)
    /// with the cadence in front of it: claimed before, recorded after, all
    /// under the process-wide posting lock, so the next own post waits for
    /// this one to be on the ledger before it claims.
    ///
    /// A ledger that cannot record after a successful publish is logged, not
    /// returned: the post is out, and an error here would invite a retry
    /// that posts it twice.
    pub async fn publish_own<T, E, F, Fut>(
        &self,
        kind: &str,
        text: Option<&str>,
        publish: F,
    ) -> Result<T, PublishError<E>>
    where
        T: OwnPostId,
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        let _posting = POSTING.lock().await;
        let claim = self.claim(kind).await.map_err(PublishError::Ledger)?;
        if !claim.is_granted() {
            return Err(PublishError::Held(CadenceHeld(claim)));
        }
        let posted = publish().await.map_err(PublishError::Publish)?;
        if let Err(error) = self.record(posted.own_post_id(), kind, text).await {
            tracing::warn!(%error, post_id = posted.own_post_id(), kind, "cadence: published but could not record the post");
        }
        Ok(posted)
    }
}

#[cfg(test)]
#[path = "cadence_tests.rs"]
mod tests;
