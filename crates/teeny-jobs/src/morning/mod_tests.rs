use std::sync::Arc;

use chrono::TimeDelta;
use teeny_store::posts::{InMemoryPostLedger, NewOwnPost, PostLedger};

use super::*;
use crate::morning::test_support::{
    Draw, Film, News, Recorder, Remembers, at, ctx_at, ctx_with, staged,
};

/// The real pipeline from the fake clients, with the outside world swapped
/// for scripted parts.
fn pipeline(ctx: &JobCtx, caption: Option<&'static str>) -> (MorningPipeline, Arc<Recorder>) {
    let poster = Recorder::new();
    let mut pipeline = build(ctx).expect("the fake clients have news and pictures");
    pipeline.news = Arc::new(News);
    pipeline.timeline = None;
    pipeline.text = staged(
        caption
            .map(|caption| ("caption", vec![caption]))
            .into_iter()
            .collect(),
    );
    pipeline.images = Arc::new(Draw::default());
    pipeline.videos = Some(Film::new([]));
    pipeline.memory = Remembers::saying("yesterday: robots");
    pipeline.poster = poster.clone();
    pipeline.critic = None;
    pipeline.retry_delay = std::time::Duration::ZERO;
    (pipeline, poster)
}

#[tokio::test]
async fn runs_once_a_day_and_journals_the_turn_as_a_morning_post() {
    let (ctx, clock) = ctx_at(at("2026-09-14T07:00:00Z"), Arc::default());
    assert!(admit(&ctx, gates(false)).await.is_ok());
    let (pipeline, poster) = pipeline(&ctx, None);
    let report = run_pipeline(&ctx, &pipeline).await.unwrap();
    assert_eq!(report.outcome, Outcome::Posted);
    assert_eq!(report.posted_id.as_deref(), Some("9001"));
    assert_eq!(poster.posts().len(), 1);

    let entry = ctx.journal.recent(1).await.unwrap().remove(0);
    assert_eq!(entry.id, "morning:2026-09-14");
    assert_eq!(entry.trigger, Trigger::Morning);
    assert_eq!(entry.outcome, Outcome::Posted);
    assert!(entry.said.contains("gm builders"));
    assert!(
        entry
            .thinking
            .contains("story: An agent runtime hit 40k stars")
    );
    assert_eq!(entry.incoming, "2 stories read, 0 dropped as political");
    assert_eq!(entry.tools[0].name, "news");

    clock.advance(TimeDelta::hours(1));
    let declined = run(&ctx).await.unwrap();
    assert!(!declined.ran);
    assert!(declined.note.unwrap().contains("already ran recently"));
}

#[tokio::test]
async fn asks_the_cadence_first_and_stands_down_when_it_says_too_soon() {
    let ledger = Arc::new(InMemoryPostLedger::default());
    ledger
        .record(&NewOwnPost {
            post_id: "1".into(),
            kind: "quote".into(),
            posted_at: "2026-09-14T06:00:00.000Z".into(),
            text: None,
        })
        .await
        .unwrap();
    let (ctx, _) = ctx_at(at("2026-09-14T07:00:00Z"), ledger);
    let report = run(&ctx).await.unwrap();
    assert!(!report.ran);
    assert!(
        report
            .note
            .unwrap()
            .contains("too soon after the last post")
    );
    assert!(ctx.journal.recent(5).await.unwrap().is_empty());
}

#[tokio::test]
async fn journals_a_held_morning_as_held_with_the_category_and_without_the_caption() {
    let (ctx, _) = ctx_at(at("2026-09-14T07:00:00Z"), Arc::default());
    let (pipeline, poster) = pipeline(
        &ctx,
        Some(r#"{"text": "gm. best gun for home defence? tiny knows."}"#),
    );
    let report = run_pipeline(&ctx, &pipeline).await.unwrap();
    assert_eq!(report.outcome, Outcome::Held);
    assert!(poster.posts().is_empty());
    let entry = ctx.journal.recent(1).await.unwrap().remove(0);
    assert_eq!(entry.outcome, Outcome::Held);
    assert_eq!(entry.said, "");
    assert_eq!(
        entry.outcome_note.as_deref(),
        Some("content filter withheld the caption: weapons")
    );
}

#[tokio::test]
async fn journals_a_failure_and_still_returns_it() {
    let (ctx, _) = ctx_at(at("2026-09-14T07:00:00Z"), Arc::default());
    let (mut pipeline, _) = pipeline(&ctx, None);
    pipeline.text = Arc::new(crate::morning::test_support::Staged::new(|_| {
        Err("ladder down".into())
    }));
    let error = run_pipeline(&ctx, &pipeline).await.unwrap_err();
    assert!(format!("{error:#}").contains("ladder down"));
    let entry = ctx.journal.recent(1).await.unwrap().remove(0);
    assert_eq!(entry.outcome, Outcome::Failed);
    assert_eq!(entry.outcome_note.as_deref(), Some("ladder down"));
}

#[tokio::test]
async fn says_which_part_is_missing_when_it_cannot_be_built() {
    let (ctx, _) = ctx_with(at("2026-09-14T07:00:00Z"), Arc::default(), |deps| {
        deps.exa = None
    });
    let error = build(&ctx).err().unwrap();
    assert_eq!(
        error.to_string(),
        "EXA_API_KEY is not set, so there is no news to read"
    );
    let (ctx, _) = ctx_with(at("2026-09-14T07:00:00Z"), Arc::default(), |deps| {
        deps.media = None
    });
    assert!(
        build(&ctx)
            .err()
            .unwrap()
            .to_string()
            .contains("IMAGE_GEN_ENABLED is off")
    );
}

#[tokio::test]
async fn the_rehearsal_posts_nothing_and_hands_back_the_draft() {
    let (ctx, _) = ctx_at(at("2026-09-14T07:00:00Z"), Arc::default());
    let (pipeline, poster) = pipeline(&ctx, None);
    let result = pipeline.run(Mode::Rehearsal).await.unwrap();
    let report = report(&result);
    assert_eq!(report.outcome, Outcome::DryRun);
    assert!(report.drafts[0].contains("gm builders"));
    assert!(poster.posts().is_empty());
}
