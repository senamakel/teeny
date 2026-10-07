use std::sync::Arc;

use chrono::TimeDelta;

use super::*;
use crate::moon::pick::tests::board;
use crate::morning::test_support::{Recorder, Remembers, Staged, at, ctx_at};

fn pipeline(ctx: &JobCtx, board: Vec<teeny_market::leaderboard::ListedCoin>) -> MoonPipeline {
    struct Fixed(Vec<teeny_market::leaderboard::ListedCoin>);
    #[async_trait::async_trait]
    impl pipeline::LeaderboardSource for Fixed {
        async fn category(
            &self,
            _: &str,
            _: usize,
        ) -> Result<Vec<teeny_market::leaderboard::ListedCoin>> {
            Ok(self.0.clone())
        }
        async fn coin(&self, _: &str) -> Result<teeny_market::leaderboard::ListedCoin> {
            anyhow::bail!("gecko down")
        }
        async fn logo(&self, _: &str) -> Option<teeny_media::moon::Logo> {
            None
        }
    }
    let mut pipeline = build(ctx);
    pipeline.source = Arc::new(Fixed(board));
    pipeline.coin_id = "tiny-humans-ai".into();
    pipeline.text = Arc::new(Staged::new(|_| {
        Ok(r#"{"text": "$TINY at $AIC's cap. small robot."}"#.into())
    }));
    pipeline.memory = Remembers::saying("Monday: $DRB at 37x");
    pipeline.poster = Recorder::new();
    pipeline.min_multiple = 2.0;
    pipeline.max_multiple = 100.0;
    pipeline.random = 0.0;
    pipeline
}

#[tokio::test]
async fn runs_on_its_day_and_journals_the_turn() {
    // The fake configuration's rota: three days from its anchor, the moon in slot 0.
    let rota = Rota::from_config(
        &JobCtx::fake(Arc::new(crate::runner::ScriptedRunner::new()))
            .config
            .rota,
    )
    .unwrap();
    let mut day = at("2026-09-14T13:00:00Z");
    while rota.slot(day) != 0 {
        day += TimeDelta::days(1);
    }
    let (ctx, clock) = ctx_at(day, Arc::default());
    let gates = Gates {
        job: JOB,
        kind: Some(JOB),
        rota: Some((rota, 0)),
        force: false,
    };
    assert!(admit(&ctx, gates).await.is_ok());
    let report = run_pipeline(&ctx, &pipeline(&ctx, board())).await.unwrap();
    assert_eq!(report.outcome, Outcome::Posted);
    let entry = ctx.journal.recent(1).await.unwrap().remove(0);
    assert_eq!(entry.id, format!("moon:{}", day.format("%Y-%m-%d")));
    assert_eq!(entry.trigger, Trigger::Moon);
    assert!(entry.thinking.contains("yardstick: $AIC"));
    assert!(entry.thinking.contains("skipped as recently used: DRB"));
    assert_eq!(entry.incoming, "11 coins on the board, $TINY at $500,000");

    clock.advance(TimeDelta::hours(1));
    let again = run(&ctx).await.unwrap();
    assert!(!again.ran);
    clock.advance(TimeDelta::days(1));
    let off_day = run(&ctx).await.unwrap();
    assert!(off_day.note.unwrap().contains("not its day on the rota"));
}

#[tokio::test]
async fn journals_a_failure_and_returns_it() {
    let (ctx, _) = ctx_at(at("2026-09-14T13:00:00Z"), Arc::default());
    let empty = pipeline(&ctx, Vec::new());
    let error = run_pipeline(&ctx, &empty).await.unwrap_err();
    assert!(format!("{error:#}").contains("gecko down"));
    let entry = ctx.journal.recent(1).await.unwrap().remove(0);
    assert_eq!(entry.outcome, Outcome::Failed);
    assert_eq!(entry.outcome_note.as_deref(), Some("gecko down"));
}
