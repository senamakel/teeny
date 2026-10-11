//! Port of `legacy/test/slot.test.ts`.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

use chrono::{DateTime, Utc};

use super::*;
use crate::clock::{ManualClock, at};

/// A job that answers each run from a script (posted, not posted, or an
/// error) and posts once the script runs out.
#[derive(Default)]
struct Counter {
    runs: AtomicU32,
    script: Mutex<VecDeque<Result<bool, &'static str>>>,
}

#[async_trait]
impl SlotRun for Counter {
    async fn run(&self) -> Result<bool, SlotRunError> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        match self.script.lock().unwrap().pop_front() {
            None => Ok(true),
            Some(Ok(posted)) => Ok(posted),
            Some(Err(error)) => Err(error.into()),
        }
    }
}

impl Counter {
    fn scripted(script: impl IntoIterator<Item = Result<bool, &'static str>>) -> Self {
        Self {
            runs: AtomicU32::new(0),
            script: Mutex::new(script.into_iter().collect()),
        }
    }

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

fn posted(job: &str) -> SlotAttempt {
    SlotAttempt {
        job: job.into(),
        result: Attempt::Posted,
    }
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
            attempts: vec![posted("article")]
        }
    );
    assert_eq!(article.runs(), 1);
    assert_eq!(lore.runs(), 0);
    clock.set(evening("2026-09-25")); // Friday, slot 4
    slot.run().await;
    assert_eq!(serial.runs(), 1);
    clock.set(evening("2026-09-27")); // Sunday, slot 6
    assert_eq!(
        slot.run().await,
        SlotOutcome::Ran {
            day: 6,
            attempts: vec![posted("lore")]
        }
    );
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

    let failing_owner = Arc::new(Counter::scripted([Err("ladder down")]));
    let failing = SlotJob::new(
        "evening",
        rota(),
        Arc::new(ManualClock::new(evening("2026-09-23"))),
    )
    .owner("article", [2], failing_owner.clone());
    let outcome = failing.run().await;
    assert_eq!(
        outcome,
        SlotOutcome::Ran {
            day: 2,
            attempts: vec![SlotAttempt {
                job: "article".into(),
                result: Attempt::Failed("ladder down".into())
            }]
        }
    );
    assert_eq!(outcome.failure(), Some(("article", "ladder down")));
    assert!(!outcome.posted());
    assert_eq!(failing_owner.runs(), 1);
}

#[tokio::test]
async fn an_owner_that_does_not_post_hands_the_slot_to_the_fallback_whether_it_skipped_or_threw() {
    let quote = Arc::new(Counter::scripted([Ok(false), Ok(false)]));
    let article = Arc::new(Counter::scripted([
        Ok(false),
        Err("x refused the article"),
        Ok(true),
    ]));
    let slot = SlotJob::new(
        "evening",
        rota(),
        Arc::new(ManualClock::new(evening("2026-09-23"))),
    )
    .owner("article", [2], article.clone())
    .fallback("quote", quote.clone());

    assert_eq!(
        slot.run().await,
        SlotOutcome::Ran {
            day: 2,
            attempts: vec![
                SlotAttempt {
                    job: "article".into(),
                    result: Attempt::NotPosted
                },
                SlotAttempt {
                    job: "quote".into(),
                    result: Attempt::NotPosted
                },
            ]
        }
    );
    assert_eq!(quote.runs(), 1);

    let threw = slot.run().await;
    assert_eq!(quote.runs(), 2);
    // The owner's failure is on the record, but the slot's last word is the
    // fallback's, so the run is not an error.
    assert_eq!(threw.failure(), None);
    match &threw {
        SlotOutcome::Ran { attempts, .. } => assert_eq!(
            attempts[0].result,
            Attempt::Failed("x refused the article".into())
        ),
        other => panic!("{other:?}"),
    }

    let landed = slot.run().await;
    assert!(landed.posted());
    assert_eq!(article.runs(), 3);
    assert_eq!(quote.runs(), 2);
}

#[tokio::test]
async fn an_owner_that_does_not_post_with_no_fallback_leaves_the_slot_quiet() {
    let article = Arc::new(Counter::scripted([Ok(false)]));
    let slot = SlotJob::new(
        "evening",
        rota(),
        Arc::new(ManualClock::new(evening("2026-09-23"))),
    )
    .owner("article", [2], article.clone());
    let outcome = slot.run().await;
    assert!(!outcome.posted());
    assert_eq!(outcome.failure(), None);
    assert_eq!(article.runs(), 1);
}

#[tokio::test]
async fn an_owner_may_borrow_what_it_runs_with() {
    // The jobs' owners close over the run's context rather than owning it.
    let posts = AtomicU32::new(0);
    struct Borrowing<'a>(&'a AtomicU32);
    #[async_trait]
    impl SlotRun for Borrowing<'_> {
        async fn run(&self) -> Result<bool, SlotRunError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(true)
        }
    }
    let slot = SlotJob::new(
        "afternoon",
        Rota::parse(3, "2026-09-14").unwrap(),
        Arc::new(ManualClock::new(evening("2026-09-14"))),
    )
    .owner("chart", [0], Arc::new(Borrowing(&posts)));
    assert!(slot.run().await.posted());
    assert_eq!(posts.load(Ordering::SeqCst), 1);
}

#[test]
fn the_default_calendar_is_three_days_with_the_article_once_a_cycle() {
    let config = teeny_core::Config::from_vars([
        ("X_BOT_USERNAME", "teenydroid"),
        ("X_API_KEY", "k"),
        ("X_API_SECRET", "s"),
        ("X_ACCESS_TOKEN", "t"),
        ("X_ACCESS_TOKEN_SECRET", "ts"),
        ("CORTEX_TOKEN", "c"),
        ("LLM_API_KEY", "l"),
        ("LLM_MODEL", "reasoning"),
    ])
    .unwrap();
    let rota = Rota::from_config(&config.rota).unwrap();
    let clock = Arc::new(ManualClock::new(evening("2026-09-14")));
    let run = Arc::new(Counter::default());
    let afternoon = SlotJob::new("afternoon", rota, clock.clone())
        .owner("chart", [config.rota.chart_slot], run.clone())
        .owner("moon", [config.rota.moon_slot], run.clone())
        .fallback("lore", run.clone());
    let evening = SlotJob::new("evening", rota, clock)
        .owner(
            "article",
            config.rota.article_slots.iter().copied(),
            run.clone(),
        )
        .owner(
            "narrative",
            config.rota.narrative_slots.iter().copied(),
            run.clone(),
        )
        .fallback("quote", run);
    assert_eq!(afternoon.describe(), "0:chart 1:moon 2:lore");
    assert_eq!(evening.describe(), "0:quote 1:article 2:narrative");
}
