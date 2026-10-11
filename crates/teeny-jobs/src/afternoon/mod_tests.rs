use std::sync::Arc;

use chrono::TimeDelta;
use futures::future::BoxFuture;
use teeny_cadence::clock::ManualClock;
use teeny_cadence::slot::{Attempt, SlotAttempt};
use teeny_core::Config;
use teeny_store::journal::Outcome;

use super::*;
use crate::morning::test_support::{at, ctx_at};
use crate::runner::ScriptedRunner;

fn config(vars: &[(&'static str, &'static str)]) -> Config {
    let mut all = teeny_agent::deps::fake_vars();
    all.extend(vars.iter().copied());
    Config::from_vars(all).unwrap()
}

/// A context at `now` (UTC) on `config`, and its clock.
fn ctx_on(config: Config, now: &str) -> (JobCtx, Arc<ManualClock>) {
    let (probe, clock) = ctx_at(at(now), Arc::default());
    let ctx = JobCtx::new(
        probe.deps.clone(),
        Arc::new(ScriptedRunner::new()),
        Arc::new(config),
        probe.journal.clone(),
    )
    .with_clock(clock.clone());
    (ctx, clock)
}

fn ctx(config: Config, now: &str) -> JobCtx {
    ctx_on(config, now).0
}

fn all_on() -> Config {
    config(&[
        ("POST_ROTA_DAYS", "3"),
        ("POST_ROTA_ANCHOR", "2026-09-14"),
        ("MOON_ENABLED", "true"),
        ("CHART_ENABLED", "true"),
        ("LORE_ENABLED", "true"),
    ])
}

#[test]
fn gives_the_chart_day_0_the_moon_day_1_and_the_lore_the_rest() {
    let ctx = ctx(all_on(), "2026-09-14T15:00:00Z");
    assert_eq!(describe(&ctx).unwrap(), "0:chart 1:moon 2:lore");
    let day = |date: &str| at(&format!("{date}T15:00:00Z"));
    let config = &ctx.config;
    assert_eq!(
        owner(config, day("2026-09-14")).unwrap(),
        Some(Afternoon::Chart)
    );
    assert_eq!(
        owner(config, day("2026-09-15")).unwrap(),
        Some(Afternoon::Moon)
    );
    assert_eq!(
        owner(config, day("2026-09-16")).unwrap(),
        Some(Afternoon::Lore)
    );
    assert_eq!(
        owner(config, day("2026-09-17")).unwrap(),
        Some(Afternoon::Chart)
    );
}

#[test]
fn hands_a_switched_off_owners_days_to_the_lore_and_is_quiet_without_it() {
    let moon_day = at("2026-09-15T15:00:00Z");
    let lore_only = config(&[("MOON_ENABLED", "false"), ("LORE_ENABLED", "true")]);
    assert_eq!(owner(&lore_only, moon_day).unwrap(), Some(Afternoon::Lore));
    assert_eq!(
        describe(&ctx(lore_only, "2026-09-15T15:00:00Z")).unwrap(),
        "0:lore 1:lore 2:lore"
    );
    let nothing = config(&[("MOON_ENABLED", "false"), ("LORE_ENABLED", "false")]);
    assert_eq!(owner(&nothing, moon_day).unwrap(), None);
    assert_eq!(
        describe(&ctx(nothing, "2026-09-15T15:00:00Z")).unwrap(),
        "0:- 1:- 2:-"
    );
}

#[tokio::test]
async fn a_quiet_day_runs_nothing() {
    let quiet = ctx(
        config(&[
            ("MOON_ENABLED", "false"),
            ("CHART_ENABLED", "false"),
            ("LORE_ENABLED", "false"),
        ]),
        "2026-09-14T15:00:00Z",
    );
    assert_eq!(run(&quiet).await.unwrap(), SlotOutcome::Quiet { day: 0 });
}

fn held(_: &JobCtx) -> BoxFuture<'_, anyhow::Result<JobReport>> {
    Box::pin(async { Ok(JobReport::ran("chart", Outcome::Held)) })
}

fn broke(_: &JobCtx) -> BoxFuture<'_, anyhow::Result<JobReport>> {
    Box::pin(async { Err(anyhow::anyhow!("gecko down")) })
}

fn posted(_: &JobCtx) -> BoxFuture<'_, anyhow::Result<JobReport>> {
    Box::pin(async { Ok(JobReport::ran("lore", Outcome::Posted)) })
}

#[tokio::test]
async fn an_owner_that_does_not_post_hands_the_afternoon_to_the_lore() {
    let jobs = AfternoonJobs {
        chart: held,
        moon: broke,
        lore: posted,
    };
    let (chart_day, clock) = ctx_on(all_on(), "2026-09-14T15:00:00Z");
    assert_eq!(
        run_with(&chart_day, &jobs).await.unwrap(),
        SlotOutcome::Ran {
            day: 0,
            attempts: vec![
                SlotAttempt {
                    job: "chart".into(),
                    result: Attempt::NotPosted
                },
                SlotAttempt {
                    job: "lore".into(),
                    result: Attempt::Posted
                },
            ]
        }
    );
    clock.advance(TimeDelta::days(1));
    let moon_day = run_with(&chart_day, &jobs).await.unwrap();
    assert!(moon_day.posted());
    assert_eq!(moon_day.failure(), None);
    match moon_day {
        SlotOutcome::Ran { day: 1, attempts } => {
            assert_eq!(attempts[0].result, Attempt::Failed("gecko down".into()));
        }
        other => panic!("{other:?}"),
    }
}
