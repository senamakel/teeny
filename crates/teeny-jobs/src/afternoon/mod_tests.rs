use std::sync::Arc;

use chrono::TimeDelta;
use teeny_core::Config;

use super::*;
use crate::morning::test_support::{at, ctx_at};
use crate::runner::ScriptedRunner;

fn config(vars: &[(&'static str, &'static str)]) -> Config {
    let mut all = teeny_agent::deps::fake_vars();
    all.extend([("POST_ROTA_DAYS", "7"), ("POST_ROTA_ANCHOR", "2026-09-14")]);
    all.extend(vars.iter().copied());
    Config::from_vars(all).unwrap()
}

#[test]
fn gives_the_moon_its_day_the_chart_the_next_and_lore_the_rest() {
    let config = config(&[
        ("MOON_ENABLED", "true"),
        ("CHART_ENABLED", "true"),
        ("LORE_ENABLED", "true"),
    ]);
    let day = |date: &str| at(&format!("{date}T13:00:00Z"));
    let (moon, chart) = (config.rota.moon_slot, config.rota.chart_slot);
    let rota = Rota::from_config(&config.rota).unwrap();
    let mut seen = Vec::new();
    for offset in 0..7 {
        let now = day("2026-09-14") + TimeDelta::days(offset);
        let slot = rota.slot(now);
        let expected = if slot == moon {
            Some(Afternoon::Moon)
        } else if slot == chart {
            Some(Afternoon::Chart)
        } else {
            Some(Afternoon::Lore)
        };
        assert_eq!(owner(&config, now).unwrap(), expected);
        seen.push(expected);
    }
    assert!(seen.contains(&Some(Afternoon::Lore)));
}

#[test]
fn hands_a_switched_off_owners_days_to_lore_and_is_quiet_without_it() {
    let now = at("2026-09-14T13:00:00Z");
    let moon_day = {
        let config = config(&[]);
        let rota = Rota::from_config(&config.rota).unwrap();
        let mut day = now;
        while rota.slot(day) != config.rota.moon_slot {
            day += TimeDelta::days(1);
        }
        day
    };
    let lore_only = config(&[("MOON_ENABLED", "false"), ("LORE_ENABLED", "true")]);
    assert_eq!(owner(&lore_only, moon_day).unwrap(), Some(Afternoon::Lore));
    let nothing = config(&[("MOON_ENABLED", "false"), ("LORE_ENABLED", "false")]);
    assert_eq!(owner(&nothing, moon_day).unwrap(), None);
}

#[test]
fn describes_the_rota_for_the_startup_log() {
    let config = config(&[
        ("MOON_ENABLED", "true"),
        ("CHART_ENABLED", "true"),
        ("LORE_ENABLED", "true"),
    ]);
    let described = describe(&config).unwrap();
    assert_eq!(described.split(' ').count(), 7);
    assert!(described.contains(&format!("{}:moon", config.rota.moon_slot)));
    assert!(described.contains(&format!("{}:chart", config.rota.chart_slot)));
}

#[tokio::test]
async fn a_quiet_day_runs_nothing() {
    let (ctx, _) = ctx_at(at("2026-09-14T13:00:00Z"), Arc::default());
    let ctx = JobCtx::new(
        ctx.deps.clone(),
        Arc::new(ScriptedRunner::new()),
        Arc::new(config(&[
            ("MOON_ENABLED", "false"),
            ("CHART_ENABLED", "false"),
            ("LORE_ENABLED", "false"),
        ])),
        ctx.journal.clone(),
    )
    .with_clock(ctx.clock.clone());
    let report = run(&ctx).await.unwrap();
    assert!(!report.ran);
    assert!(report.note.unwrap().contains("nobody has today"));
}
