//! Shared by the cadence tests.

use chrono::{DateTime, TimeDelta, Utc};
use teeny_core::Config;
use teeny_store::posts::{MetricSample, NewOwnPost, OwnPost};

use crate::clock::{at, iso};

pub(crate) const HOUR: TimeDelta = TimeDelta::hours(1);

/// 2026-09-14T07:00:00Z, the morning the legacy tests start from.
pub(crate) fn t0() -> DateTime<Utc> {
    at("2026-09-14T07:00:00.000Z")
}

pub(crate) fn own(post_id: &str, kind: &str, posted_at: DateTime<Utc>) -> NewOwnPost {
    NewOwnPost {
        post_id: post_id.into(),
        kind: kind.into(),
        posted_at: iso(posted_at),
        text: None,
    }
}

pub(crate) fn post(
    post_id: &str,
    kind: &str,
    posted_at: DateTime<Utc>,
    samples: Vec<MetricSample>,
) -> OwnPost {
    OwnPost {
        post_id: post_id.into(),
        kind: kind.into(),
        posted_at: iso(posted_at),
        text: None,
        samples,
        hot: None,
    }
}

pub(crate) fn sample(at: DateTime<Utc>, impressions: i64) -> MetricSample {
    MetricSample {
        at: iso(at),
        impressions,
        likes: 0,
        replies: 0,
        reposts: 0,
        quotes: 0,
    }
}

/// The configuration with only the required values set.
pub(crate) fn config() -> Config {
    Config::from_vars([
        ("X_BOT_USERNAME", "teenydroid"),
        ("X_API_KEY", "k"),
        ("X_API_SECRET", "s"),
        ("X_ACCESS_TOKEN", "t"),
        ("X_ACCESS_TOKEN_SECRET", "ts"),
        ("CORTEX_TOKEN", "c"),
        ("LLM_API_KEY", "l"),
        ("LLM_MODEL", "reasoning"),
    ])
    .expect("a valid configuration")
}
