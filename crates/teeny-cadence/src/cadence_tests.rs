//! Port of "the cadence" in `legacy/test/cadence.test.ts`. The legacy test
//! wrapped the X client in a proxy; here the Publisher guard calls
//! [`Cadence::publish_own`] for own posts and leaves replies alone.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use teeny_store::posts::{Hot, InMemoryPostLedger, PostLedger};

use super::*;
use crate::clock::{ManualClock, iso};
use crate::test_support::{HOUR, config, t0};

fn subject() -> (Arc<InMemoryPostLedger>, Cadence, Arc<ManualClock>) {
    let ledger = Arc::new(InMemoryPostLedger::default());
    let clock = Arc::new(ManualClock::new(t0()));
    let rules = CadenceRules {
        min_gap: HOUR * 8,
        tolerance: TimeDelta::minutes(20),
    };
    (
        ledger.clone(),
        Cadence::new(ledger, rules, clock.clone()),
        clock,
    )
}

#[tokio::test]
async fn lets_the_first_post_through_then_refuses_one_inside_the_gap() {
    let (_, cadence, clock) = subject();
    assert_eq!(cadence.claim("morning").await.unwrap(), Claim::Granted);
    cadence.record("1", "morning", None).await.unwrap();
    clock.set(t0() + HOUR * 5);
    assert_eq!(
        cadence.claim("chart").await.unwrap(),
        Claim::Gap {
            last_post_id: "1".into(),
            last_posted_at: iso(t0()),
            until: iso(t0() + HOUR * 8)
        }
    );
}

#[tokio::test]
async fn counts_the_tolerance_as_the_gap_so_a_slot_eight_hours_later_that_ran_late_still_posts() {
    let (_, cadence, clock) = subject();
    cadence.record("1", "morning", None).await.unwrap();
    clock.set(t0() + HOUR * 8 - TimeDelta::minutes(15));
    assert_eq!(cadence.claim("chart").await.unwrap(), Claim::Granted);
    clock.set(t0() + HOUR * 8 - TimeDelta::minutes(25));
    assert!(!cadence.claim("chart").await.unwrap().is_granted());
}

#[tokio::test]
async fn holds_the_timeline_for_a_hot_post_whatever_the_gap_says() {
    let (ledger, cadence, clock) = subject();
    cadence.record("1", "morning", None).await.unwrap();
    let hot = Hot {
        since: iso(t0() + HOUR),
        hold_until: iso(t0() + HOUR * 9),
    };
    ledger.set_hot("1", &hot).await.unwrap();
    clock.set(t0() + HOUR * 17 / 2);
    assert_eq!(
        cadence.claim("chart").await.unwrap(),
        Claim::Hot {
            hot_post_id: "1".into(),
            until: iso(t0() + HOUR * 9)
        }
    );
    clock.set(t0() + HOUR * 19 / 2);
    assert_eq!(cadence.claim("chart").await.unwrap(), Claim::Granted);
}

#[tokio::test]
async fn paces_own_posts_claimed_and_recorded_under_their_kind_and_refused_inside_the_gap() {
    let (ledger, cadence, clock) = subject();
    let published = AtomicU32::new(0);
    let publish = || async {
        published.fetch_add(1, Ordering::SeqCst);
        Ok::<_, String>("77".to_owned())
    };
    assert_eq!(
        cadence
            .publish_own("moon", Some("to the moon"), publish)
            .await
            .unwrap(),
        "77"
    );
    let recorded = ledger.get("77").await.unwrap().unwrap();
    assert_eq!(
        (recorded.kind.as_str(), recorded.text.as_deref()),
        ("moon", Some("to the moon"))
    );
    clock.set(t0() + HOUR);
    let refused = cadence
        .publish_own("post", Some("again"), publish)
        .await
        .unwrap_err();
    assert!(
        matches!(refused, PublishError::Held(CadenceHeld(Claim::Gap { .. }))),
        "{refused}"
    );
    assert!(
        refused
            .to_string()
            .starts_with("cadence: too soon after post 77"),
        "{refused}"
    );
    assert_eq!(published.load(Ordering::SeqCst), 1);
    assert_eq!(ledger.since("", 50).await.unwrap().len(), 1);
}

#[tokio::test]
async fn two_own_posts_in_the_same_minute_cannot_both_pass_the_gap() {
    let (ledger, cadence, _) = subject();
    let published = AtomicU32::new(0);
    // Publishing takes a moment, as X does: without the posting lock the
    // second claim reads the ledger before the first post is recorded.
    let publish = |id: &'static str| {
        let published = &published;
        move || async move {
            tokio::task::yield_now().await;
            published.fetch_add(1, Ordering::SeqCst);
            Ok::<_, String>(id.to_owned())
        }
    };
    let (first, second) = tokio::join!(
        cadence.publish_own("chart", None, publish("1")),
        cadence.publish_own("moon", None, publish("2")),
    );
    let refused = [&first, &second]
        .into_iter()
        .filter(|outcome| {
            matches!(
                outcome,
                Err(PublishError::Held(CadenceHeld(Claim::Gap { .. })))
            )
        })
        .count();
    assert_eq!(refused, 1, "{first:?} {second:?}");
    assert_eq!(published.load(Ordering::SeqCst), 1);
    assert_eq!(ledger.since("", 50).await.unwrap().len(), 1);
}

#[derive(Debug, PartialEq)]
struct Article {
    article_id: String,
    post_id: String,
}

impl OwnPostId for Article {
    fn own_post_id(&self) -> &str {
        &self.post_id
    }
}

#[tokio::test]
async fn paces_an_article_the_same_way_claimed_published_recorded_as_an_own_post() {
    let (ledger, cadence, clock) = subject();
    let publish = || async {
        Ok::<_, String>(Article {
            article_id: "a1".into(),
            post_id: "88".into(),
        })
    };
    let article = cadence
        .publish_own("article", Some("the inch"), publish)
        .await
        .unwrap();
    assert_eq!(article.article_id, "a1");
    assert_eq!(ledger.get("88").await.unwrap().unwrap().kind, "article");
    clock.set(t0() + HOUR);
    assert!(matches!(
        cadence.publish_own("article", None, publish).await,
        Err(PublishError::Held(_))
    ));
}

#[tokio::test]
async fn passes_a_failed_publish_through_and_records_nothing() {
    let (ledger, cadence, _) = subject();
    let failed = cadence
        .publish_own("post", None, || async { Err::<String, _>("X is down") })
        .await;
    assert!(matches!(failed, Err(PublishError::Publish("X is down"))));
    assert!(ledger.latest().await.unwrap().is_none());
}

#[test]
fn reads_the_gap_and_tolerance_from_the_configuration() {
    let rules = CadenceRules::from_config(&config().cadence);
    assert_eq!(
        rules,
        CadenceRules {
            min_gap: HOUR * 7,
            tolerance: TimeDelta::minutes(20)
        }
    );
    let held = CadenceHeld(Claim::Hot {
        hot_post_id: "1".into(),
        until: "2026-09-14T16:00:00.000Z".into(),
    });
    assert_eq!(
        held.to_string(),
        "cadence: the timeline is held for hot post 1 until 2026-09-14T16:00:00.000Z"
    );
}
