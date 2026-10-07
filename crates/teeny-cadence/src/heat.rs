//! Watches the account's recent posts and notices one taking off.
//!
//! Every run reads the counters of everything posted in the last day and
//! keeps the reading. Two readings give a velocity, impressions an hour, and
//! a post is hot when that velocity is both large (`HOT_IMPRESSIONS_PER_HOUR`)
//! and a multiple (`HOT_BASELINE_MULTIPLE`) of what the account's posts
//! usually do: one number alone misreads a big account's ordinary day or a
//! small account's good hour.
//!
//! A hot post gets a hold on the timeline: one slot from now, extended while
//! it stays hot, up to `HOT_MAX_HOLD_SLOTS` from when it first ran hot. The
//! cadence refuses own posts while the hold runs, and the reply budget hands
//! the held slots' replies to the hot post's thread.
//! A port of `legacy/src/cadence/heat.ts`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use chrono::{DateTime, TimeDelta, Utc};
use teeny_core::config::CadenceConfig;
use teeny_store::StoreError;
use teeny_store::posts::{Hot, MetricSample, OwnPost, PostLedger};

use crate::clock::{Clock, hours, iso, parse_iso};

/// How far back a post is still watched.
pub const LOOKBACK: TimeDelta = TimeDelta::hours(24);

/// One post's public counters, as X reports them to its author.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostMetrics {
    pub id: String,
    pub impressions: i64,
    pub likes: i64,
    pub replies: i64,
    pub reposts: i64,
    pub quotes: i64,
}

/// A heat run that could not finish.
#[derive(Debug, thiserror::Error)]
pub enum HeatError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("heat: could not read metrics: {0}")]
    Metrics(String),
}

/// Where the counters come from: the X client, reading the account's own posts.
#[async_trait]
pub trait MetricsSource: Send + Sync {
    async fn post_metrics(&self, ids: &[String]) -> Result<Vec<PostMetrics>, HeatError>;
}

/// The heat thresholds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HeatRules {
    pub lookback: TimeDelta,
    /// One slot of the cadence: how long a hold lasts per extension.
    pub gap: TimeDelta,
    /// Below this many impressions an hour nothing is hot, whatever the baseline.
    pub impressions_per_hour: f64,
    /// Hot also means this many times the account's usual pace.
    pub baseline_multiple: f64,
    /// How many slots a hold may grow to, from when the post first ran hot.
    pub max_hold_slots: u32,
}

impl HeatRules {
    pub fn from_config(config: &CadenceConfig) -> Self {
        Self {
            lookback: LOOKBACK,
            gap: hours(config.post_min_gap_hours),
            impressions_per_hour: config.hot_impressions_per_hour,
            baseline_multiple: config.hot_baseline_multiple,
            max_hold_slots: config.hot_max_hold_slots,
        }
    }
}

/// What one run read for one post.
#[derive(Debug, Clone, PartialEq)]
pub struct HeatReading {
    pub post_id: String,
    pub impressions: i64,
    pub velocity: Option<f64>,
    pub baseline: Option<f64>,
    pub hot: bool,
    pub hold_until: Option<String>,
}

/// The heat job (`HEAT_CRON`).
pub struct HeatJob {
    ledger: Arc<dyn PostLedger>,
    x: Arc<dyn MetricsSource>,
    rules: HeatRules,
    clock: Arc<dyn Clock>,
    running: AtomicBool,
}

/// Clears the running flag however the run ends.
struct Running<'a>(&'a AtomicBool);

impl Drop for Running<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

impl HeatJob {
    pub fn new(
        ledger: Arc<dyn PostLedger>,
        x: Arc<dyn MetricsSource>,
        rules: HeatRules,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            ledger,
            x,
            rules,
            clock,
            running: AtomicBool::new(false),
        }
    }

    /// One reading of every recent own post. A run that starts while another
    /// is going reads nothing.
    pub async fn run(&self) -> Result<Vec<HeatReading>, HeatError> {
        if self.running.swap(true, Ordering::SeqCst) {
            return Ok(Vec::new());
        }
        let _running = Running(&self.running);
        let now = self.clock.now();
        let now_iso = iso(now);
        let posts = self
            .ledger
            .since(&iso(now - self.rules.lookback), 50)
            .await?;
        if posts.is_empty() {
            return Ok(Vec::new());
        }
        let ids: Vec<String> = posts.iter().map(|post| post.post_id.clone()).collect();
        let metrics = self.x.post_metrics(&ids).await?;
        let mut readings = Vec::new();
        for post in &posts {
            let Some(current) = metrics.iter().find(|entry| entry.id == post.post_id) else {
                continue;
            };
            let sample = MetricSample {
                at: now_iso.clone(),
                impressions: current.impressions,
                likes: current.likes,
                replies: current.replies,
                reposts: current.reposts,
                quotes: current.quotes,
            };
            let velocity = velocity_of(post, &sample);
            self.ledger.sample(&post.post_id, &sample).await?;
            let baseline = baseline_of(posts.iter().filter(|other| other.post_id != post.post_id));
            let hot = velocity.is_some_and(|velocity| {
                velocity >= self.rules.impressions_per_hour
                    && baseline
                        .is_none_or(|baseline| velocity >= baseline * self.rules.baseline_multiple)
            });
            let hold_until = if hot {
                Some(self.hold(post, now, &now_iso).await?)
            } else {
                None
            };
            if let Some(until) = &hold_until {
                tracing::info!(post_id = %post.post_id, ?velocity, ?baseline, impressions = sample.impressions, hold_until = %until, "heat: the post is hot; holding the timeline for it");
            }
            readings.push(HeatReading {
                post_id: post.post_id.clone(),
                impressions: sample.impressions,
                velocity,
                baseline,
                hot,
                hold_until,
            });
        }
        Ok(readings)
    }

    /// Holds (or extends the hold on) the timeline for a hot post: a slot
    /// from now, capped at `max_hold_slots` from when it first ran hot, and
    /// never shorter than the hold it already has.
    async fn hold(
        &self,
        post: &OwnPost,
        now: DateTime<Utc>,
        now_iso: &str,
    ) -> Result<String, HeatError> {
        let since = post
            .hot
            .as_ref()
            .map_or_else(|| now_iso.to_owned(), |hot| hot.since.clone());
        let started = parse_iso(&since).unwrap_or(now);
        let slots = i32::try_from(self.rules.max_hold_slots).unwrap_or(i32::MAX);
        let extended = (now + self.rules.gap).min(started + self.rules.gap * slots);
        let current = post.hot.as_ref().and_then(|hot| parse_iso(&hot.hold_until));
        let hold_until = iso(current.map_or(extended, |current| current.max(extended)));
        if post.hot.as_ref().map(|hot| hot.hold_until.as_str()) != Some(hold_until.as_str()) {
            self.ledger
                .set_hot(
                    &post.post_id,
                    &Hot {
                        since,
                        hold_until: hold_until.clone(),
                    },
                )
                .await?;
        }
        Ok(hold_until)
    }
}

fn hours_between(from: DateTime<Utc>, to: DateTime<Utc>) -> f64 {
    (to - from).num_milliseconds() as f64 / 3_600_000.0
}

/// Impressions an hour between the last kept reading (or the post itself)
/// and this one. `None` when no time has passed.
pub fn velocity_of(post: &OwnPost, sample: &MetricSample) -> Option<f64> {
    let (from_at, from_impressions) = match post.samples.last() {
        Some(previous) => (previous.at.as_str(), previous.impressions),
        None => (post.posted_at.as_str(), 0),
    };
    let hours = hours_between(parse_iso(from_at)?, parse_iso(&sample.at)?);
    if hours <= 0.0 {
        return None;
    }
    Some((sample.impressions - from_impressions).max(0) as f64 / hours)
}

/// What the account's other recent posts did at their best: the median of
/// each post's peak velocity. `None` until two posts have readings, when only
/// the absolute threshold applies.
pub fn baseline_of<'a>(others: impl IntoIterator<Item = &'a OwnPost>) -> Option<f64> {
    let mut peaks: Vec<f64> = others
        .into_iter()
        .filter(|post| !post.samples.is_empty())
        .map(|post| {
            let mut peak = 0.0_f64;
            let mut from = (parse_iso(&post.posted_at), 0);
            for sample in &post.samples {
                let at = parse_iso(&sample.at);
                if let (Some(from_at), Some(to_at)) = (from.0, at) {
                    let hours = hours_between(from_at, to_at);
                    if hours > 0.0 {
                        peak = peak.max((sample.impressions - from.1) as f64 / hours);
                    }
                }
                from = (at, sample.impressions);
            }
            peak
        })
        .collect();
    if peaks.len() < 2 {
        return None;
    }
    peaks.sort_by(f64::total_cmp);
    let middle = peaks.len() / 2;
    Some(if peaks.len().is_multiple_of(2) {
        (peaks[middle - 1] + peaks[middle]) / 2.0
    } else {
        peaks[middle]
    })
}

#[cfg(test)]
#[path = "heat_tests.rs"]
mod tests;
