//! The heat job's wiring: the account's own posts from the post ledger, their
//! counters from X, the hold written back (legacy/src/index.ts, `HeatJob`).

use std::sync::Arc;

use chrono::{TimeDelta, TimeZone, Utc};
use teeny_agent::deps::{Deps, fake_vars};
use teeny_cadence::clock::{ManualClock, iso};
use teeny_core::Config;
use teeny_store::posts::{MetricSample, NewOwnPost};
use teeny_store::{InMemoryPostLedger, PostLedger};
use teeny_x::model::OwnPostMetrics;

use super::*;
use crate::ctx::{Conversation, JobCtx};
use crate::reply::test_support::FakeX;
use crate::runner::ScriptedRunner;

async fn subject(
    extra: &[(&str, &str)],
    metrics: Vec<OwnPostMetrics>,
) -> (JobCtx, Arc<InMemoryPostLedger>, Arc<FakeX>) {
    let now = Utc.with_ymd_and_hms(2026, 9, 2, 12, 0, 0).unwrap();
    let posts = Arc::new(InMemoryPostLedger::default());
    posts
        .record(&NewOwnPost {
            post_id: "500".into(),
            kind: "chart".into(),
            posted_at: iso(now - TimeDelta::hours(2)),
            text: None,
        })
        .await
        .unwrap();
    posts
        .sample(
            "500",
            &MetricSample {
                at: iso(now - TimeDelta::hours(1)),
                impressions: 100,
                likes: 0,
                replies: 0,
                reposts: 0,
                quotes: 0,
            },
        )
        .await
        .unwrap();
    let x = Arc::new(FakeX {
        metrics,
        ..FakeX::default()
    });
    let mut deps = Deps::fake();
    deps.x_reads = x.clone();
    let mut vars = fake_vars();
    vars.extend_from_slice(extra);
    let mut ctx = JobCtx::fake(Arc::new(ScriptedRunner::new()))
        .with_clock(Arc::new(ManualClock::new(now)))
        .with_conversation(Conversation {
            posts: posts.clone(),
            ..Conversation::fake()
        });
    ctx.deps = Arc::new(deps);
    ctx.config = Arc::new(Config::from_vars(vars).unwrap());
    (ctx, posts, x)
}

fn counters(impressions: u64) -> OwnPostMetrics {
    OwnPostMetrics {
        id: "500".into(),
        impressions,
        likes: 0,
        replies: 0,
        reposts: 0,
        quotes: 0,
    }
}

#[tokio::test]
async fn reads_every_recent_post_and_holds_the_timeline_for_a_hot_one() {
    let (ctx, posts, x) = subject(&[], vec![counters(10_000)]).await;
    let report = run(&ctx).await.unwrap();

    assert!(report.ran);
    assert_eq!(report.note.as_deref(), Some("read 1 posts, 1 hot"));
    assert_eq!(*x.metric_ids.lock().unwrap(), vec!["500".to_owned()]);
    let post = posts.get("500").await.unwrap().unwrap();
    assert_eq!(post.samples.len(), 2);
    assert!(post.hot.is_some());
}

#[tokio::test]
async fn a_quiet_post_is_sampled_and_not_held() {
    let (ctx, posts, _) = subject(&[], vec![counters(150)]).await;
    let report = run(&ctx).await.unwrap();

    assert_eq!(report.note.as_deref(), Some("read 1 posts, 0 hot"));
    assert!(posts.get("500").await.unwrap().unwrap().hot.is_none());
}

#[tokio::test]
async fn does_nothing_when_heat_is_off() {
    let (ctx, posts, x) = subject(&[("HEAT_ENABLED", "false")], vec![counters(10_000)]).await;
    let report = run(&ctx).await.unwrap();

    assert!(!report.ran);
    assert!(x.metric_ids.lock().unwrap().is_empty());
    assert_eq!(posts.get("500").await.unwrap().unwrap().samples.len(), 1);
}
