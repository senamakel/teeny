use std::sync::Arc;

use chrono::TimeDelta;
use teeny_store::posts::{InMemoryPostLedger, NewOwnPost, PostLedger};

use super::*;
use crate::morning::test_support::{at, ctx_at};

fn gates(rota: Option<(Rota, u32)>) -> Gates {
    Gates {
        job: "moon",
        kind: Some("moon"),
        rota,
        force: false,
    }
}

#[tokio::test]
async fn keeps_a_job_to_its_day_on_the_rota() {
    let rota = Rota::parse(3, "2026-09-14").unwrap();
    let (ctx, clock) = ctx_at(at("2026-09-15T13:00:00Z"), Arc::default());
    let declined = admit(&ctx, gates(Some((rota, 0)))).await.unwrap_err();
    assert!(declined.contains("not its day on the rota"));
    clock.set(at("2026-09-17T13:00:00Z"));
    assert!(admit(&ctx, gates(Some((rota, 0)))).await.is_ok());
}

#[tokio::test]
async fn declines_to_run_twice_inside_twenty_hours_and_runs_again_after() {
    let (ctx, clock) = ctx_at(at("2026-09-14T13:00:00Z"), Arc::default());
    assert!(admit(&ctx, gates(None)).await.is_ok());
    clock.advance(TimeDelta::hours(1));
    assert!(admit(&ctx, gates(None)).await.unwrap_err().contains("already ran recently"));
    clock.advance(TimeDelta::hours(20));
    assert!(admit(&ctx, gates(None)).await.is_ok());
}

#[tokio::test]
async fn asks_the_cadence_and_stands_down_when_too_soon_unless_forced() {
    let ledger = Arc::new(InMemoryPostLedger::default());
    ledger
        .record(&NewOwnPost {
            post_id: "1".into(),
            kind: "morning".into(),
            posted_at: "2026-09-14T12:00:00.000Z".into(),
            text: None,
        })
        .await
        .unwrap();
    let (ctx, _) = ctx_at(at("2026-09-14T13:00:00Z"), ledger);
    let declined = admit(&ctx, gates(None)).await.unwrap_err();
    assert!(declined.contains("too soon after the last post"));
    // Declining does not mark the guard.
    assert!(ctx.guard("moon").check(ctx.now()).is_ok());
    let forced = Gates { force: true, ..gates(None) };
    assert!(admit(&ctx, forced).await.is_ok());
}

#[test]
fn rolls_a_value_in_the_unit_interval() {
    let (ctx, _) = ctx_at(at("2026-09-14T13:00:00.123Z"), Arc::default());
    let value = roll(&ctx);
    assert!((0.0..1.0).contains(&value));
}
