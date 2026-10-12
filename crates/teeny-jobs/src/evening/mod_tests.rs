use std::sync::Arc;

use chrono::{TimeZone, Utc};
use futures::future::BoxFuture;
use teeny_agent::deps::{Deps, fake_vars};
use teeny_cadence::clock::ManualClock;
use teeny_cadence::slot::{Attempt, SlotAttempt};
use teeny_store::InMemoryJournal;
use teeny_store::journal::Outcome;

use super::*;
use crate::runner::ScriptedRunner;

fn ctx(extra: &[(&'static str, &'static str)]) -> JobCtx {
    let mut vars = fake_vars();
    vars.push(("POST_ROTA_DAYS", "3"));
    vars.push(("POST_ROTA_ANCHOR", "2026-09-14"));
    vars.extend_from_slice(extra);
    let config = teeny_core::Config::from_vars(vars).unwrap();
    JobCtx::new(
        Arc::new(Deps::fake()),
        Arc::new(ScriptedRunner::new()),
        Arc::new(config),
        Arc::new(InMemoryJournal::default()),
    )
}

fn on(ctx: JobCtx, day: u32) -> JobCtx {
    ctx.with_clock(Arc::new(ManualClock::new(
        Utc.with_ymd_and_hms(2026, 9, 14 + day, 23, 0, 0).unwrap(),
    )))
}

#[test]
fn runs_the_article_and_the_serial_on_their_days_and_the_quote_on_the_rest() {
    let all = ctx(&[
        ("NARRATIVE_ENABLED", "true"),
        ("ARTICLE_ENABLED", "true"),
        ("QUOTE_ENABLED", "true"),
    ]);
    assert_eq!(describe(&all).unwrap(), "0:quote 1:article 2:narrative");
    let article_only = ctx(&[("ARTICLE_ENABLED", "true")]);
    assert_eq!(describe(&article_only).unwrap(), "0:- 1:article 2:-");
    // A job switched off hands its days to the quote.
    let no_serial = ctx(&[("ARTICLE_ENABLED", "true"), ("QUOTE_ENABLED", "true")]);
    assert_eq!(owner_of(&no_serial, 2), Some(EveningJob::Quote));
    // The lore post is the afternoon's fallback now, never the evening's.
    let lore = ctx(&[("LORE_ENABLED", "true")]);
    assert_eq!(describe(&lore).unwrap(), "0:- 1:- 2:-");
}

#[tokio::test]
async fn is_quiet_with_no_owner() {
    let quiet = on(ctx(&[("ARTICLE_ENABLED", "true")]), 0);
    assert_eq!(run(&quiet).await.unwrap(), SlotOutcome::Quiet { day: 0 });
}

#[tokio::test]
async fn survives_an_owner_failing() {
    // The serial cannot be built without its lore files on this box; the slot reports it and carries on.
    let failing = on(
        ctx(&[
            ("NARRATIVE_ENABLED", "true"),
            ("NARRATIVE_LORE_PATH", "/nonexistent/LORE.md"),
        ]),
        2,
    );
    let outcome = run(&failing).await.unwrap();
    assert_eq!(outcome.failure().map(|(job, _)| job), Some("narrative"));
}

fn rehearsed(_: &JobCtx) -> BoxFuture<'_, anyhow::Result<JobReport>> {
    Box::pin(async { Ok(JobReport::ran("article", Outcome::DryRun)) })
}

fn quoted(_: &JobCtx) -> BoxFuture<'_, anyhow::Result<JobReport>> {
    Box::pin(async { Ok(JobReport::ran("quote", Outcome::Posted)) })
}

#[tokio::test]
async fn an_article_that_does_not_post_hands_the_evening_to_the_quote() {
    let jobs = EveningJobs {
        narrative: rehearsed,
        article: rehearsed,
        quote: quoted,
    };
    let article_day = on(
        ctx(&[("ARTICLE_ENABLED", "true"), ("QUOTE_ENABLED", "true")]),
        1,
    );
    assert_eq!(
        run_with(&article_day, &jobs).await.unwrap(),
        SlotOutcome::Ran {
            day: 1,
            attempts: vec![
                SlotAttempt {
                    job: "article".into(),
                    result: Attempt::NotPosted
                },
                SlotAttempt {
                    job: "quote".into(),
                    result: Attempt::Posted
                },
            ]
        }
    );
}
