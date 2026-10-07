use std::sync::Arc;

use chrono::TimeDelta;
use teeny_store::posts::{InMemoryPostLedger, NewOwnPost, PostLedger};

use super::*;
use crate::morning::test_support::{FakeReads, Recorder, Remembers, Staged, ctx_at};
use crate::quote::candidates::tests::{by, now, post};
use crate::quote::log::MemoryQuoteLog;

fn pipeline(ctx: &JobCtx, reads: FakeReads) -> QuotePipeline {
    let mut pipeline = build(ctx);
    pipeline.x = Arc::new(reads);
    pipeline.text = Arc::new(Staged::new(|_| {
        Ok(r#"{"pick":1,"why":"a bull with a reason","text":"the words"}"#.into())
    }));
    pipeline.memory = Remembers::saying("h");
    pipeline.log = Arc::new(MemoryQuoteLog::default());
    pipeline.poster = Recorder::new();
    pipeline
}

fn found() -> FakeReads {
    FakeReads::answering([
        Ok(vec![
            by("bull", post("1", "the post about $TINY and the lab")),
            by("bot", post("5", "$TINY")),
            by("bot2", post("6", "$TINY")),
        ]),
        Ok(Vec::new()),
    ])
}

#[test]
fn counts_the_reasons_for_the_journal() {
    let reasons = [
        "already quoted",
        "already quoted",
        "@a was quoted on 2026-09-19",
        "names another token ($X)",
    ]
    .map(String::from);
    assert_eq!(
        summarise(&reasons),
        "already quoted ×2, author quoted lately, names another token"
    );
}

#[tokio::test]
async fn refuses_to_run_twice_inside_the_interval_and_journals_the_turn() {
    let (ctx, clock) = ctx_at(now(), Arc::default());
    assert!(admit(&ctx, gates()).await.is_ok());
    let report = run_pipeline(&ctx, &pipeline(&ctx, found())).await.unwrap();
    assert_eq!(report.outcome, Outcome::Posted);
    assert_eq!(report.drafts, ["the words"]);
    let entry = ctx.journal.recent(1).await.unwrap().remove(0);
    assert_eq!(entry.id, "quote:2026-09-21");
    assert_eq!(entry.trigger, Trigger::Quote);
    assert_eq!(entry.said, "the words");
    assert_eq!(entry.incoming, "@bull: the post about $TINY and the lab");
    assert!(entry.thinking.contains("skipped: says nothing beyond the ticker ×2"), "{}", entry.thinking);

    clock.advance(TimeDelta::hours(1));
    assert!(!run(&ctx).await.unwrap().ran);
}

#[tokio::test]
async fn asks_the_cadence_first_and_stands_down_when_told() {
    let ledger = Arc::new(InMemoryPostLedger::default());
    ledger
        .record(&NewOwnPost {
            post_id: "p".into(),
            kind: "morning".into(),
            posted_at: teeny_store::iso(now() - TimeDelta::minutes(30)),
            text: None,
        })
        .await
        .unwrap();
    let (ctx, _) = ctx_at(now(), ledger);
    let report = run(&ctx).await.unwrap();
    assert!(!report.ran);
    assert!(report.note.unwrap().contains("too soon"));
}

#[tokio::test]
async fn journals_a_failure_and_returns_it() {
    let (ctx, _) = ctx_at(now(), Arc::default());
    let mut broken = pipeline(&ctx, found());
    broken.text = Arc::new(Staged::new(|_| Err("x down".into())));
    assert!(run_pipeline(&ctx, &broken).await.is_err());
    let entry = ctx.journal.recent(1).await.unwrap().remove(0);
    assert_eq!(entry.outcome, Outcome::Failed);
    assert_eq!(entry.outcome_note.as_deref(), Some("x down"));
}
