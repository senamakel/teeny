use std::sync::Arc;

use chrono::{TimeDelta, TimeZone, Utc};
use teeny_agent::deps::Deps;
use teeny_cadence::clock::{Clock, ManualClock};
use teeny_store::{InMemoryJournal, Journal};

use super::*;
use crate::lore::pipeline::tests::{FakeRooms, Found, deps, pipeline, seed};
use crate::runner::ScriptedRunner;

fn ctx(deps: Arc<Deps>, clock: Arc<ManualClock>) -> (JobCtx, Arc<InMemoryJournal>) {
    let journal = Arc::new(InMemoryJournal::default());
    let config = teeny_core::Config::from_vars(teeny_agent::deps::fake_vars()).unwrap();
    let ctx = JobCtx::new(
        deps,
        Arc::new(ScriptedRunner::new()),
        Arc::new(config),
        journal.clone(),
    )
    .with_clock(clock);
    (ctx, journal)
}

fn seeds() -> Vec<(SeedSource, Arc<dyn SeedFinder>)> {
    vec![(SeedSource::News, Arc::new(Found(Some(seed()), false)))]
}

#[tokio::test]
async fn journals_the_turn_with_the_room_and_the_post() {
    let clock = Arc::new(ManualClock::new(
        Utc.with_ymd_and_hms(2026, 9, 22, 15, 0, 0).unwrap(),
    ));
    let (_server, _memory, deps) = deps(true).await;
    let (ctx, journal) = ctx(deps.clone(), clock);
    let rooms = Arc::new(FakeRooms::new("the words"));
    let report = run_pipeline(&ctx, AFTERNOON, &pipeline(rooms, seeds(), deps), false)
        .await
        .unwrap();
    assert_eq!(report.outcome, Outcome::Posted);
    assert_eq!(report.posted_id.as_deref(), Some("own-1"));
    let entry = &journal.recent(1).await.unwrap()[0];
    assert_eq!(entry.id, "lore:2026-09-22");
    assert_eq!(entry.trigger, Trigger::Lore);
    assert_eq!(entry.said, "the words");
    assert_eq!(entry.incoming, "news (the wire): AI agents now file taxes");
    assert!(entry.thinking.contains("kept the kettle"));
    assert!(entry.tools[0].result.contains("the ledger balances"));
}

#[tokio::test]
async fn refuses_to_run_twice_inside_the_interval_under_each_name() {
    let clock = Arc::new(ManualClock::new(
        Utc.with_ymd_and_hms(2026, 9, 22, 15, 0, 0).unwrap(),
    ));
    let (ctx, _journal) = ctx(Arc::new(Deps::fake()), clock.clone());
    ctx.guard(AFTERNOON).mark(clock.now());
    let declined = run_afternoon(&ctx).await.unwrap();
    assert!(!declined.ran);
    assert!(declined.note.unwrap().contains("already ran recently"));
    clock.advance(TimeDelta::hours(1));
    ctx.guard(EVENING).mark(clock.now());
    assert!(!run_evening(&ctx).await.unwrap().ran);
}

#[tokio::test]
async fn the_rehearsal_returns_the_draft_posts_nothing_and_is_journaled() {
    let clock = Arc::new(ManualClock::new(
        Utc.with_ymd_and_hms(2026, 9, 22, 15, 0, 0).unwrap(),
    ));
    let (server, _memory, deps) = deps(true).await;
    let (ctx, journal) = ctx(deps.clone(), clock);
    let rooms = Arc::new(FakeRooms::new("the kettle is on the ledger now."));
    let report = run_pipeline(&ctx, AFTERNOON, &pipeline(rooms, seeds(), deps), true)
        .await
        .unwrap();
    assert_eq!(report.outcome, Outcome::DryRun);
    assert_eq!(report.drafts, ["the kettle is on the ledger now."]);
    assert!(server.received_requests().await.unwrap().is_empty());
    assert_eq!(
        journal.recent(1).await.unwrap()[0].id,
        "lore:2026-09-22:dry-run"
    );
}
