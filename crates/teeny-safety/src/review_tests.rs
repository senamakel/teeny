//! Port of `legacy/test/safety-review.test.ts`: the gate recording its holds,
//! approved links reaching the gate, and `reviewHold`. The HTTP endpoints in
//! that file belong to the server and are ported with the axum routes.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use super::*;
use crate::gate::{SafetyGate, SafetyGateOptions};
use crate::holds::{
    HeldLink, HoldBy, HoldContext, HoldInput, HoldKind, HoldLog, HoldStatus, HoldSurface,
    InMemoryHoldLog,
};
use crate::screen::Direction;
use crate::test_support::{CountingRelease, FlakyLog, ScriptedRelease, SpyQueue};

fn x_context() -> HoldContext {
    HoldContext {
        post_id: Some("10".into()),
        author_username: Some("alice".into()),
        ..HoldContext::new(HoldSurface::X)
    }
}

fn hold() -> HoldInput {
    HoldInput {
        direction: Direction::Inbound,
        kind: HoldKind::Message,
        context: x_context(),
        category: "sexual".into(),
        reason: "contains \"naked\"".into(),
        by: HoldBy::Screen,
        text: "draw yourself naked".into(),
        links: None,
    }
}

fn gate_over(holds: Arc<dyn HoldLog>) -> SafetyGate {
    SafetyGate::new(SafetyGateOptions {
        holds: Some(holds),
        ..Default::default()
    })
}

fn deps(holds: &dyn HoldLog) -> ReviewDependencies<'_> {
    ReviewDependencies {
        holds,
        requeue: None,
        release: None,
        refresh_links: None,
    }
}

mod the_gate_records_what_it_holds {
    use super::*;

    #[tokio::test]
    async fn writes_an_inbound_hold_with_the_text_and_where_it_came_from() {
        let holds = Arc::new(InMemoryHoldLog::default());
        gate_over(holds.clone())
            .inbound("send nudes", Some(&x_context()))
            .await;
        let entry = &holds.list(None, None).await.unwrap()[0];
        assert_eq!(entry.direction, Direction::Inbound);
        assert_eq!(entry.kind, HoldKind::Message);
        assert_eq!(entry.category, "sexual");
        assert_eq!(entry.by, HoldBy::Screen);
        assert_eq!(entry.text, "send nudes");
        assert_eq!(entry.status, HoldStatus::Pending);
        assert_eq!(entry.context, x_context());
    }

    #[tokio::test]
    async fn writes_a_task_hold_for_the_ad_hoc_endpoint() {
        let holds = Arc::new(InMemoryHoldLog::default());
        gate_over(holds.clone())
            .inbound("send nudes", Some(&HoldContext::new(HoldSurface::Task)))
            .await;
        assert_eq!(
            holds.list(None, None).await.unwrap()[0].kind,
            HoldKind::Task
        );
    }

    #[tokio::test]
    async fn writes_a_link_hold_when_a_draft_goes_out_with_links_cut() {
        let holds = Arc::new(InMemoryHoldLog::default());
        let context = HoldContext {
            post_id: Some("10".into()),
            ..HoldContext::new(HoldSurface::X)
        };
        let result = gate_over(holds.clone())
            .outbound("see https://claim-tiny.xyz/go now", Some(&context))
            .await;
        assert!(result.decision.is_clean());
        let entry = &holds.list(None, None).await.unwrap()[0];
        assert_eq!(entry.kind, HoldKind::Link);
        assert_eq!(entry.category, "links");
        assert_eq!(entry.by, HoldBy::Links);
        assert_eq!(entry.text, "see https://claim-tiny.xyz/go now");
        assert_eq!(
            entry.links,
            Some(vec![HeldLink {
                link: "https://claim-tiny.xyz/go".into(),
                reason: "claim-tiny.xyz is not on the link allowlist".into(),
            }])
        );
    }

    #[tokio::test]
    async fn writes_a_draft_hold_for_a_withheld_reply_and_a_prompt_hold_for_a_refused_picture() {
        let holds = Arc::new(InMemoryHoldLog::default());
        let gate = gate_over(holds.clone());
        let telegram = HoldContext {
            chat_id: Some("1".into()),
            message_id: Some("5".into()),
            ..HoldContext::new(HoldSurface::Telegram)
        };
        gate.outbound("this is fucking great", Some(&telegram))
            .await;
        let prompt = HoldContext {
            kind: Some(HoldKind::Prompt),
            tool: Some("generate_image".into()),
            ..HoldContext::new(HoldSurface::X)
        };
        gate.outbound("Tiny naked on a beach", Some(&prompt)).await;
        let kinds: Vec<HoldKind> = holds
            .list(None, None)
            .await
            .unwrap()
            .iter()
            .map(|h| h.kind)
            .collect();
        assert_eq!(kinds, [HoldKind::Prompt, HoldKind::Draft]);
    }

    #[tokio::test]
    async fn records_nothing_without_a_context_and_nothing_when_the_text_is_clean() {
        let holds = Arc::new(InMemoryHoldLog::default());
        let gate = gate_over(holds.clone());
        gate.inbound("send nudes", None).await;
        gate.inbound("gm", Some(&HoldContext::new(HoldSurface::X)))
            .await;
        gate.outbound("gm", Some(&HoldContext::new(HoldSurface::X)))
            .await;
        assert!(holds.list(None, None).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn never_lets_a_broken_log_turn_a_hold_into_anything_else() {
        let holds = Arc::new(FlakyLog::default());
        holds.fail_record.store(true, Ordering::SeqCst);
        let decision = gate_over(holds)
            .inbound("send nudes", Some(&HoldContext::new(HoldSurface::X)))
            .await;
        assert!(!decision.is_clean());
    }
}

mod approved_links_reach_the_gate {
    use super::*;

    #[tokio::test]
    async fn reads_operator_approvals_alongside_the_built_in_list() {
        let holds = Arc::new(InMemoryHoldLog::default());
        holds.allow_link("claim-tiny.xyz", "h1").await.unwrap();
        let result = gate_over(holds)
            .outbound(
                "see https://claim-tiny.xyz/go",
                Some(&HoldContext::new(HoldSurface::X)),
            )
            .await;
        assert_eq!(result.text, "see https://claim-tiny.xyz/go");
        assert!(result.fixes.is_empty());
    }

    #[tokio::test]
    async fn caches_them_and_refreshes_on_demand() {
        let holds = Arc::new(InMemoryHoldLog::default());
        let gate = gate_over(holds.clone());
        let x = HoldContext::new(HoldSurface::X);
        assert_eq!(
            gate.outbound("https://claim-tiny.xyz/go ok", Some(&x))
                .await
                .fixes
                .len(),
            1
        );
        holds.allow_link("claim-tiny.xyz", "h1").await.unwrap();
        // Still cached.
        assert_eq!(
            gate.outbound("https://claim-tiny.xyz/go ok", None)
                .await
                .fixes
                .len(),
            1
        );
        gate.refresh_approved_links();
        assert_eq!(
            gate.outbound("https://claim-tiny.xyz/go ok", None)
                .await
                .fixes
                .len(),
            0
        );
    }

    #[tokio::test]
    async fn keeps_the_last_approvals_when_the_log_cannot_be_read() {
        let holds = Arc::new(FlakyLog::default());
        holds.allow_link("claim-tiny.xyz", "h1").await.unwrap();
        let gate = gate_over(holds.clone());
        gate.outbound("x", None).await;
        holds.fail_allowed_links.store(true, Ordering::SeqCst);
        gate.refresh_approved_links();
        assert_eq!(
            gate.outbound("https://claim-tiny.xyz/go ok", None)
                .await
                .fixes
                .len(),
            0
        );
    }
}

mod review_hold {
    use super::*;

    #[tokio::test]
    async fn rejecting_confirms_the_filter_and_does_nothing_else() {
        let holds = InMemoryHoldLog::default();
        let id = holds.record(hold()).await.unwrap().id;
        let queue = SpyQueue::answering(true);
        let deps = ReviewDependencies {
            requeue: Some(&queue),
            ..deps(&holds)
        };
        let reviewed = review_hold(
            &id,
            HoldDecision::Rejected,
            &deps,
            Some("the filter was right"),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(reviewed.status, HoldStatus::Rejected);
        assert_eq!(reviewed.effect.as_deref(), Some("confirmed the filter"));
        assert_eq!(reviewed.note.as_deref(), Some("the filter was right"));
        assert!(queue.calls().is_empty());
    }

    #[tokio::test]
    async fn approving_a_held_x_mention_sends_it_back_to_the_queue() {
        let holds = InMemoryHoldLog::default();
        let id = holds.record(hold()).await.unwrap().id;
        let queue = SpyQueue::answering(true);
        let deps = ReviewDependencies {
            requeue: Some(&queue),
            ..deps(&holds)
        };
        let reviewed = review_hold(&id, HoldDecision::Approved, &deps, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(queue.calls(), ["10"]);
        assert_eq!(reviewed.effect.as_deref(), Some("requeued to be answered"));
    }

    #[tokio::test]
    async fn approving_a_held_telegram_message_is_recorded_only() {
        let holds = InMemoryHoldLog::default();
        let telegram = HoldContext {
            chat_id: Some("1".into()),
            message_id: Some("5".into()),
            ..HoldContext::new(HoldSurface::Telegram)
        };
        let id = holds
            .record(HoldInput {
                context: telegram,
                ..hold()
            })
            .await
            .unwrap()
            .id;
        let queue = SpyQueue::answering(true);
        let deps = ReviewDependencies {
            requeue: Some(&queue),
            ..deps(&holds)
        };
        let reviewed = review_hold(&id, HoldDecision::Approved, &deps, None)
            .await
            .unwrap()
            .unwrap();
        assert!(
            reviewed
                .effect
                .unwrap()
                .contains("cannot be answered after the fact")
        );
    }

    #[tokio::test]
    async fn approving_a_link_hold_allowlists_its_hosts_and_refreshes_the_gate() {
        let holds = InMemoryHoldLog::default();
        let cut = |link: &str| HeldLink {
            link: link.into(),
            reason: "x".into(),
        };
        let input = HoldInput {
            direction: Direction::Outbound,
            kind: HoldKind::Link,
            category: "links".into(),
            by: HoldBy::Links,
            text: "see https://dexscreener.com/solana/abc and https://dexscreener.com/solana/def"
                .into(),
            links: Some(vec![
                cut("https://dexscreener.com/solana/abc"),
                cut("https://dexscreener.com/solana/def"),
            ]),
            ..hold()
        };
        let id = holds.record(input).await.unwrap().id;
        let refreshed = Mutex::new(0);
        let refresh = || *refreshed.lock().unwrap() += 1;
        let deps = ReviewDependencies {
            refresh_links: Some(&refresh),
            ..deps(&holds)
        };
        let reviewed = review_hold(&id, HoldDecision::Approved, &deps, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            reviewed.effect.as_deref(),
            Some("allowlisted dexscreener.com")
        );
        assert_eq!(holds.allowed_links().await.unwrap(), ["dexscreener.com"]);
        assert_eq!(*refreshed.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn approving_a_draft_releases_it_and_a_failed_release_leaves_it_pending() {
        let holds = InMemoryHoldLog::default();
        let draft = HoldInput {
            direction: Direction::Outbound,
            kind: HoldKind::Draft,
            text: "gm frens".into(),
            ..hold()
        };
        let id = holds.record(draft).await.unwrap().id;
        let release = ScriptedRelease {
            answer: Mutex::new(Err("X is down".into())),
        };
        let deps = ReviewDependencies {
            release: Some(&release),
            ..deps(&holds)
        };
        let failed = review_hold(&id, HoldDecision::Approved, &deps, None)
            .await
            .unwrap_err();
        assert!(failed.to_string().contains("X is down"), "{failed}");
        let back = holds.get(&id).await.unwrap().unwrap();
        assert_eq!(back.status, HoldStatus::Pending);
        assert!(
            back.effect
                .as_deref()
                .unwrap_or_default()
                .contains("X is down"),
            "the failure is recorded on the hold: {back:?}"
        );

        *release.answer.lock().unwrap() = Ok("published on X as 99".into());
        let reviewed = review_hold(&id, HoldDecision::Approved, &deps, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(reviewed.status, HoldStatus::Approved);
        assert_eq!(reviewed.effect.as_deref(), Some("published on X as 99"));
    }

    fn draft_hold() -> HoldInput {
        HoldInput {
            direction: Direction::Outbound,
            kind: HoldKind::Draft,
            text: "gm frens".into(),
            ..hold()
        }
    }

    #[tokio::test]
    async fn two_approvals_at_once_publish_a_draft_once() {
        let holds = InMemoryHoldLog::default();
        let id = holds.record(draft_hold()).await.unwrap().id;
        let release = CountingRelease::default();
        let deps = ReviewDependencies {
            release: Some(&release),
            ..deps(&holds)
        };
        let (first, second) = tokio::join!(
            review_hold(&id, HoldDecision::Approved, &deps, None),
            review_hold(&id, HoldDecision::Approved, &deps, None),
        );
        first.unwrap();
        second.unwrap();
        assert_eq!(release.releases.load(Ordering::SeqCst), 1);
        let settled = holds.get(&id).await.unwrap().unwrap();
        assert_eq!(settled.status, HoldStatus::Approved);
        assert_eq!(settled.effect.as_deref(), Some("published on X as 1"));
    }

    #[tokio::test]
    async fn a_release_whose_decision_could_not_be_written_is_marked_released_with_a_note() {
        let holds = FlakyLog::default();
        let id = holds.record(draft_hold()).await.unwrap().id;
        holds.fail_decides.store(1, Ordering::SeqCst);
        let release = CountingRelease::default();
        let deps = ReviewDependencies {
            release: Some(&release),
            ..deps(&holds)
        };
        let reviewed = review_hold(&id, HoldDecision::Approved, &deps, Some("ok"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(reviewed.status, HoldStatus::Approved);
        assert_eq!(reviewed.effect.as_deref(), Some("published on X as 1"));
        let note = reviewed.note.unwrap();
        assert!(note.contains("ok") && note.contains("mongo down"), "{note}");
    }

    #[tokio::test]
    async fn a_release_whose_decision_cannot_be_written_at_all_is_never_offered_again() {
        let holds = FlakyLog::default();
        let id = holds.record(draft_hold()).await.unwrap().id;
        holds.fail_decides.store(2, Ordering::SeqCst);
        let release = CountingRelease::default();
        let deps = ReviewDependencies {
            release: Some(&release),
            ..deps(&holds)
        };
        let failed = review_hold(&id, HoldDecision::Approved, &deps, None)
            .await
            .unwrap_err();
        assert!(failed.to_string().contains("mongo down"), "{failed}");
        assert_eq!(
            holds.get(&id).await.unwrap().unwrap().status,
            HoldStatus::Releasing,
            "published, so never pending again"
        );
        review_hold(&id, HoldDecision::Approved, &deps, None)
            .await
            .unwrap();
        assert_eq!(release.releases.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn does_not_re_run_an_effect_on_a_hold_already_decided() {
        let holds = InMemoryHoldLog::default();
        let id = holds.record(hold()).await.unwrap().id;
        let queue = SpyQueue::answering(true);
        let deps = ReviewDependencies {
            requeue: Some(&queue),
            ..deps(&holds)
        };
        review_hold(&id, HoldDecision::Approved, &deps, None)
            .await
            .unwrap();
        review_hold(&id, HoldDecision::Approved, &deps, None)
            .await
            .unwrap();
        review_hold(&id, HoldDecision::Rejected, &deps, None)
            .await
            .unwrap();
        assert_eq!(queue.calls().len(), 1);
        assert_eq!(
            holds.get(&id).await.unwrap().unwrap().status,
            HoldStatus::Approved
        );
    }

    #[tokio::test]
    async fn answers_nothing_for_an_unknown_id() {
        let holds = InMemoryHoldLog::default();
        assert!(
            review_hold("nope", HoldDecision::Approved, &deps(&holds), None)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn turns_a_cut_link_into_a_host_entry() {
        assert_eq!(
            allowlist_entry_for("https://Dexscreener.com/solana/abc?x=1").as_deref(),
            Some("dexscreener.com")
        );
        assert_eq!(
            allowlist_entry_for("evil.xyz/claim").as_deref(),
            Some("evil.xyz")
        );
        assert_eq!(allowlist_entry_for("not a url at all ://"), None);
    }
}
