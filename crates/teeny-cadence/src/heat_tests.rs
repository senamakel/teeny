//! Port of "the heat job" in `legacy/test/cadence.test.ts`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use teeny_store::posts::{InMemoryPostLedger, PostLedger};

use super::*;
use crate::clock::{ManualClock, iso};
use crate::test_support::{HOUR, config, own, post, sample, t0};

#[test]
fn reads_a_velocity_from_two_readings_and_a_baseline_from_the_other_posts() {
    let one = post("1", "chart", t0(), vec![sample(t0() + HOUR, 100)]);
    assert_eq!(
        velocity_of(&one, &sample(t0() + HOUR * 2, 700)),
        Some(600.0)
    );
    assert_eq!(velocity_of(&one, &sample(t0() + HOUR, 700)), None);
    let others = [
        post("2", "morning", t0(), vec![sample(t0() + HOUR, 50)]),
        post("3", "morning", t0(), vec![sample(t0() + HOUR, 150)]),
        post("4", "morning", t0(), vec![]),
    ];
    assert_eq!(baseline_of(others.iter()), Some(100.0));
    assert_eq!(baseline_of(others[..1].iter()), None);
}

/// Answers every id with 100 impressions, except post 1, which reads `one`.
struct Metrics {
    one: Mutex<i64>,
}

#[async_trait]
impl MetricsSource for Metrics {
    async fn post_metrics(&self, ids: &[String]) -> Result<Vec<PostMetrics>, HeatError> {
        let one = *self.one.lock().unwrap();
        Ok(ids
            .iter()
            .map(|id| PostMetrics {
                id: id.clone(),
                impressions: if id == "1" { one } else { 100 },
                likes: 0,
                replies: 0,
                reposts: 0,
                quotes: 0,
            })
            .collect())
    }
}

#[tokio::test]
async fn holds_one_slot_for_a_hot_post_and_extends_it_while_it_stays_hot_up_to_the_cap() {
    let ledger = Arc::new(InMemoryPostLedger::default());
    ledger.record(&own("1", "chart", t0())).await.unwrap();
    ledger
        .record(&own("2", "morning", t0() - HOUR * 20))
        .await
        .unwrap();
    ledger
        .sample("2", &sample(t0() - HOUR * 19, 100))
        .await
        .unwrap();
    ledger
        .record(&own("3", "moon", t0() - HOUR * 22))
        .await
        .unwrap();
    ledger
        .sample("3", &sample(t0() - HOUR * 21, 120))
        .await
        .unwrap();
    let metrics = Arc::new(Metrics { one: Mutex::new(0) });
    let clock = Arc::new(ManualClock::new(t0()));
    let rules = HeatRules {
        lookback: HOUR * 24,
        gap: HOUR * 8,
        impressions_per_hour: 500.0,
        baseline_multiple: 3.0,
        max_hold_slots: 2,
    };
    let heat = HeatJob::new(ledger.clone(), metrics.clone(), rules, clock.clone());
    let minutes = TimeDelta::minutes;
    let reading = async |at: DateTime<Utc>, impressions: i64| {
        clock.set(at);
        *metrics.one.lock().unwrap() = impressions;
        let readings: HashMap<String, HeatReading> = heat
            .run()
            .await
            .unwrap()
            .into_iter()
            .map(|r| (r.post_id.clone(), r))
            .collect();
        readings["1"].clone()
    };

    // A first reading of nothing: not hot.
    assert!(!reading(t0() + minutes(20), 50).await.hot);

    // 2000 impressions in twenty minutes: 6000/h, past 500/h and past 3x the 110/h baseline.
    let hot = reading(t0() + minutes(40), 2050).await;
    assert!(hot.hot);
    assert_eq!((hot.velocity, hot.baseline), (Some(6000.0), Some(110.0)));
    assert_eq!(
        hot.hold_until.as_deref(),
        Some(iso(t0() + minutes(40) + HOUR * 8).as_str())
    );
    assert_eq!(
        ledger.get("1").await.unwrap().unwrap().hot.unwrap().since,
        iso(t0() + minutes(40))
    );

    // Still hot an hour on: the hold moves out with it.
    let later = reading(t0() + minutes(100), 4050).await;
    assert_eq!(later.hold_until, Some(iso(t0() + minutes(100) + HOUR * 8)));

    // Nine hours on and still hot: capped at two slots from when it first ran hot.
    let capped = reading(t0() + minutes(40) + HOUR * 9, 50_000).await;
    assert_eq!(capped.hold_until, Some(iso(t0() + minutes(40) + HOUR * 16)));

    // Cooled off: the hold stands where it was and is not extended further.
    let cooled = reading(t0() + minutes(40) + HOUR * 10, 50_010).await;
    assert!(!cooled.hot);
    let hold = ledger.get("1").await.unwrap().unwrap().hot.unwrap();
    assert_eq!(hold.hold_until, iso(t0() + minutes(40) + HOUR * 16));
}

#[tokio::test]
async fn reads_nothing_when_nothing_was_posted() {
    let heat = HeatJob::new(
        Arc::new(InMemoryPostLedger::default()),
        Arc::new(Metrics { one: Mutex::new(0) }),
        HeatRules::from_config(&config().cadence),
        Arc::new(ManualClock::new(t0())),
    );
    assert!(heat.run().await.unwrap().is_empty());
}

#[test]
fn reads_its_thresholds_from_the_configuration() {
    assert_eq!(
        HeatRules::from_config(&config().cadence),
        HeatRules {
            lookback: HOUR * 24,
            gap: HOUR * 7,
            impressions_per_hour: 500.0,
            baseline_multiple: 3.0,
            max_hold_slots: 2,
        }
    );
}
