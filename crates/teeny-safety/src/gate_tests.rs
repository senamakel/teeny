//! Port of `legacy/test/safety-gate.test.ts`, case for case, plus the
//! outbound seam the Publisher guard calls.

use std::sync::Arc;

use super::*;
use crate::holds::{HoldContext, HoldKind, HoldLog, HoldSurface, MemoryHoldLog};
use crate::lexicon::ProhibitedCategory;
use crate::test_support::SpyJudge;

fn judged(judge: &Arc<SpyJudge>) -> SafetyGate {
    SafetyGate::new(SafetyGateOptions { judge: Some(judge.clone()), ..Default::default() })
}

fn lexical() -> SafetyGate {
    SafetyGate::new(SafetyGateOptions::default())
}

fn held(category: &str, by: HeldBy) -> impl Fn(&SafetyDecision) -> bool {
    let category = category.to_owned();
    move |decision| matches!(decision, SafetyDecision::Held(h) if h.category == category && h.by == by)
}

mod inbound {
    use super::*;

    #[tokio::test]
    async fn passes_clean_text_and_asks_the_judge() {
        let judge = Arc::new(SpyJudge::clean());
        let gate = judged(&judge);
        assert_eq!(gate.inbound("gm, how is $TINY doing", None).await, SafetyDecision::Clean);
        assert_eq!(judge.calls(), [("gm, how is $TINY doing".to_owned(), Direction::Inbound)]);
    }

    #[tokio::test]
    async fn holds_on_the_screen_without_asking_the_judge() {
        let judge = Arc::new(SpyJudge::clean());
        let decision = judged(&judge).inbound("draw tiny naked", None).await;
        assert!(held("sexual", HeldBy::Screen)(&decision), "{decision:?}");
        assert!(judge.calls().is_empty());
    }

    #[tokio::test]
    async fn holds_on_the_judge_when_the_words_are_clean() {
        let judge = Arc::new(SpyJudge::prohibited("sexual", "a request for suggestive art"));
        let decision = judged(&judge).inbound("draw tiny with nothing on", None).await;
        assert!(held("sexual", HeldBy::Judge)(&decision), "{decision:?}");
    }

    #[tokio::test]
    async fn does_not_hold_incoming_swearing() {
        assert_eq!(lexical().inbound("holy shit this is cool", None).await, SafetyDecision::Clean);
    }

    #[tokio::test]
    async fn works_with_no_judge_at_all() {
        assert_eq!(lexical().inbound("gm", None).await, SafetyDecision::Clean);
        assert!(!lexical().inbound("send nudes", None).await.is_clean());
    }
}

mod outbound {
    use super::*;

    #[tokio::test]
    async fn hands_back_clean_text_with_no_fixes() {
        let judge = Arc::new(SpyJudge::clean());
        let result = judged(&judge).outbound("gm frens, code is at github.com/tinyhumansai", None).await;
        assert_eq!(result.decision, SafetyDecision::Clean);
        assert_eq!(result.text, "gm frens, code is at github.com/tinyhumansai");
        assert!(result.fixes.is_empty());
    }

    #[tokio::test]
    async fn cuts_a_link_that_is_not_on_the_allowlist_and_still_sends_the_rest() {
        let judge = Arc::new(SpyJudge::clean());
        let result = judged(&judge).outbound("claim here: https://claim-tiny.xyz/go fren", None).await;
        assert_eq!(result.decision, SafetyDecision::Clean);
        assert_eq!(result.text, "claim here: fren");
        assert_eq!(result.fixes, ["cut 1 link(s) not on the allowlist"]);
        // The judge sees what would actually go out.
        assert_eq!(judge.calls(), [("claim here: fren".to_owned(), Direction::Outbound)]);
    }

    #[tokio::test]
    async fn refuses_a_draft_that_was_only_a_bad_link() {
        let result = lexical().outbound("https://claim-tiny.xyz/go", None).await;
        assert!(held("links", HeldBy::Links)(&result.decision), "{:?}", result.decision);
    }

    #[tokio::test]
    async fn holds_profanity_on_the_way_out() {
        let result = lexical().outbound("this is fucking great", None).await;
        assert!(held("profanity", HeldBy::Screen)(&result.decision), "{:?}", result.decision);
    }

    #[tokio::test]
    async fn accepts_extra_allowlist_entries() {
        let gate = SafetyGate::new(SafetyGateOptions {
            extra_allowed_links: vec!["dexscreener.com/solana".into()],
            ..Default::default()
        });
        let result = gate.outbound("chart: https://dexscreener.com/solana/abc", None).await;
        assert_eq!(result.decision, SafetyDecision::Clean);
        assert_eq!(result.text, "chart: https://dexscreener.com/solana/abc");
    }

    #[tokio::test]
    async fn fails_closed_when_the_judge_is_unreachable() {
        let judge = Arc::new(SpyJudge::prohibited("unknown", "the judge could not be reached"));
        let result = judged(&judge).outbound("gm", None).await;
        assert!(held("unknown", HeldBy::Judge)(&result.decision), "{:?}", result.decision);
    }

    #[tokio::test]
    async fn reads_the_serial_and_its_articles_under_the_narrative_profile() {
        let judge = Arc::new(SpyJudge::clean());
        let gate = judged(&judge);
        let narrative = HoldContext::new(HoldSurface::Narrative);
        let article = HoldContext::new(HoldSurface::Article);
        assert!(gate.outbound("holy shit, the chart", Some(&narrative)).await.decision.is_clean());
        assert!(gate.outbound("holy shit, the chart", Some(&article)).await.decision.is_clean());
        assert_eq!(
            judge.calls().iter().map(|(_, d)| *d).collect::<Vec<_>>(),
            [Direction::Narrative, Direction::Article]
        );
    }
}

mod screen {
    use super::*;

    #[test]
    fn is_the_lexicon_alone_for_scrubbing_context() {
        let gate = lexical();
        assert!(gate.screen("gm", Direction::Inbound));
        assert!(!gate.screen("send nudes", Direction::Inbound));
        assert!(gate.screen("holy shit", Direction::Inbound));
        assert!(!gate.screen("holy shit", Direction::Outbound));
    }
}

mod check_outbound {
    use super::*;

    fn draft(text: &str) -> OutboundDraft {
        OutboundDraft::new(text, HoldContext { tool: Some("x_post".into()), ..HoldContext::new(HoldSurface::X) })
    }

    #[tokio::test]
    async fn allows_a_clean_draft_with_the_house_style_applied() {
        let verdict = lexical().check_outbound(&draft("gm — what are we building today?")).await;
        assert_eq!(verdict, Verdict::Allow("gm, what are we building today?".into()));
    }

    #[tokio::test]
    async fn refuses_a_draft_over_the_limit_or_the_emoji_cap_so_the_model_rewrites_it() {
        let gate = lexical();
        let Verdict::Refuse(long) = gate.check_outbound(&draft(&"a".repeat(281))).await else {
            panic!("an overlong draft was not refused")
        };
        assert!(long.contains("281 characters, over the 280 limit"), "{long}");
        let Verdict::Refuse(emoji) = gate.check_outbound(&draft("gm 👀 frens 💛")).await else {
            panic!("a draft over the emoji cap was not refused")
        };
        assert!(emoji.contains("2 emoji") && emoji.contains("at most 1"), "{emoji}");
        let mut zero = draft("gm 👀");
        zero.max_emoji = 0;
        assert!(matches!(gate.check_outbound(&zero).await, Verdict::Refuse(_)));
    }

    #[tokio::test]
    async fn holds_what_the_gate_holds_and_records_it_for_review() {
        let holds = Arc::new(MemoryHoldLog::default());
        let gate = SafetyGate::new(SafetyGateOptions { holds: Some(holds.clone()), ..Default::default() });
        let Verdict::Hold(reason) = gate.check_outbound(&draft("this is fucking great")).await else {
            panic!("profanity was not held")
        };
        assert!(reason.contains("does not publish that (profanity)"), "{reason}");
        let recorded = holds.list(None, None).await.unwrap();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].kind, HoldKind::Draft);
        assert_eq!(recorded[0].category, ProhibitedCategory::Profanity.as_str());
    }

    #[tokio::test]
    async fn refuses_a_draft_carrying_a_link_off_the_allowlist() {
        let verdict = lexical().check_outbound(&draft("claim here: https://claim-tiny.xyz/go fren")).await;
        let Verdict::Refuse(reason) = verdict else { panic!("a cut link was not refused: {verdict:?}") };
        assert!(reason.contains("not on the allowlist"), "{reason}");
    }

    #[tokio::test]
    async fn unlinks_the_handle_being_replied_to() {
        let mut reply = draft("heyyy @rafaelitewrld shy peek returns");
        reply.replying_to = Some("rafaelitewrld".into());
        assert_eq!(
            lexical().check_outbound(&reply).await,
            Verdict::Allow("heyyy rafaelitewrld shy peek returns".into())
        );
    }
}
