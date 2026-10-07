//! Port of `legacy/test/slot.test.ts`.

use std::sync::atomic::{AtomicU32, Ordering};

use chrono::{DateTime, Utc};

use super::*;
use crate::clock::{ManualClock, at};

#[derive(Default)]
struct Counter {
    runs: AtomicU32,
    fail: bool,
}

#[async_trait]
impl SlotRun for Counter {
    async fn run(&self) -> Result<(), SlotRunError> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            Err("ladder down".into())
        } else {
            Ok(())
        }
    }
}

impl Counter {
    fn runs(&self) -> u32 {
        self.runs.load(Ordering::SeqCst)
    }
}

fn rota() -> Rota {
    Rota::parse(7, "2026-09-14").unwrap()
}

fn evening(date: &str) -> DateTime<Utc> {
    at(&format!("{date}T19:00:00Z"))
}

#[tokio::test]
async fn runs_the_days_owner_and_the_fallback_on_days_nobody_has() {
    let article = Arc::new(Counter::default());
    let serial = Arc::new(Counter::default());
    let lore = Arc::new(Counter::default());
    let clock = Arc::new(ManualClock::new(evening("2026-09-23"))); // Wednesday, slot 2
    let slot = SlotJob::new("evening", rota(), clock.clone())
        .owner("narrative", [1, 4], serial.clone())
        .owner("article", [2, 5], article.clone())
        .fallback("lore", lore.clone());
    assert_eq!(
        slot.run().await,
        SlotOutcome::Ran {
            day: 2,
            job: "article".into()
        }
    );
    assert_eq!(article.runs(), 1);
    clock.set(evening("2026-09-25")); // Friday, slot 4
    slot.run().await;
    assert_eq!(serial.runs(), 1);
    clock.set(evening("2026-09-27")); // Sunday, slot 6
    slot.run().await;
    assert_eq!(lore.runs(), 1);
    assert_eq!(
        slot.describe(),
        "0:lore 1:narrative 2:article 3:lore 4:narrative 5:article 6:lore"
    );
}

#[tokio::test]
async fn is_quiet_with_no_owner_and_no_fallback_and_survives_an_owner_failing() {
    let quiet = SlotJob::new(
        "evening",
        rota(),
        Arc::new(ManualClock::new(evening("2026-09-22"))),
    )
    .owner("article", [2], Arc::new(Counter::default()));
    assert_eq!(quiet.run().await, SlotOutcome::Quiet { day: 1 });
    assert_eq!(quiet.describe(), "0:- 1:- 2:article 3:- 4:- 5:- 6:-");

    let failing_owner = Arc::new(Counter {
        fail: true,
        ..Counter::default()
    });
    let failing = SlotJob::new(
        "evening",
        rota(),
        Arc::new(ManualClock::new(evening("2026-09-23"))),
    )
    .owner("article", [2], failing_owner.clone());
    assert_eq!(
        failing.run().await,
        SlotOutcome::Failed {
            day: 2,
            job: "article".into(),
            error: "ladder down".into()
        }
    );
    assert_eq!(failing_owner.runs(), 1);
}
