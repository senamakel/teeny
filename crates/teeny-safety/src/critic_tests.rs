//! Port of the arithmetic half of `legacy/test/critic.test.ts`, plus the
//! critic's call. The `x_critique` tool and `POST /critique` are ported with
//! the agent tools and the axum routes.

use std::sync::Arc;

use serde_json::{Value, json};

use super::*;
use crate::model::ScriptedModel;

fn predicted(some: &[(Action, f64)]) -> Predicted {
    let mut predicted = Predicted::default();
    for (action, value) in some {
        predicted.set(*action, *value);
    }
    predicted
}

fn answer(some: &[(Action, f64)], community_reply: f64) -> CritiqueAnswer {
    CritiqueAnswer {
        predicted: predicted(some),
        community_reply,
        summary: "a read".into(),
        ..CritiqueAnswer::default()
    }
}

mod the_ranker_arithmetic {
    use super::*;
    use Action::*;

    #[test]
    fn is_the_param_rs_weighted_sum_a_reply_is_ten_likes_a_copied_link_forty_one_report_erases_forty_seven_replies()
     {
        assert_eq!(Reply.weight() / Like.weight(), 10.0);
        assert_eq!(ShareViaCopyLink.weight() / Like.weight(), 40.0);
        assert_eq!((-Report.weight() / Reply.weight()).floor(), 46.0);
        let scored = score_actions(
            &predicted(&[
                (Reply, 0.3),
                (Like, 0.5),
                (ShareViaCopyLink, 0.05),
                (Report, 0.01),
            ]),
            0.1,
        );
        // 0.3*5 + 0.5*0.5 + 0.05*20 + 0.1*15 = 4.25; report 0.01*-234 = -2.34
        assert_eq!(
            scored,
            Scored {
                score: 1.91,
                upside: 4.25,
                cost: -2.34
            }
        );
    }

    #[test]
    fn calls_a_draft_worthy_over_the_line_flat_under_it_and_risky_when_the_downside_is_the_story() {
        assert_eq!(verdict_of(WORTHY_SCORE, 0.0), CritiqueVerdict::Worthy);
        assert_eq!(verdict_of(WORTHY_SCORE - 0.01, 0.0), CritiqueVerdict::Flat);
        assert_eq!(verdict_of(30.0, RISKY_COST), CritiqueVerdict::Risky);
        let read = |some: &[(Action, f64)], community: f64| {
            let scored = score_actions(&predicted(some), community);
            verdict_of(scored.score, scored.cost)
        };
        // A likes-only post scores a sixth: flat. One that a third would answer: worthy.
        assert_eq!(read(&[(Like, 0.3)], 0.0), CritiqueVerdict::Flat);
        assert_eq!(
            read(&[(Reply, 0.3), (Quote, 0.05), (Like, 0.2)], 0.05),
            CritiqueVerdict::Worthy
        );
        // One in fifty reporting sinks anything; so does one in ten tapping "not interested".
        assert_eq!(
            read(
                &[(Reply, 0.5), (ShareViaCopyLink, 0.2), (Report, 0.02)],
                0.3
            ),
            CritiqueVerdict::Risky
        );
        assert_eq!(
            read(&[(Reply, 0.2), (NotInterested, 0.1)], 0.1),
            CritiqueVerdict::Risky
        );
        assert_eq!(
            read(&[(Reply, 0.3), (NotInterested, 0.05)], 0.1),
            CritiqueVerdict::Flat
        );
    }

    #[test]
    fn turns_the_model_answer_into_a_critique_and_renders_it_verdict_first_fixes_last() {
        let critique = to_critique(CritiqueAnswer {
            first_reply: "gm".into(),
            fixes: vec!["end on the question".into(), "cut the closing line".into()],
            summary: "a greeting plus a fact".into(),
            ..answer(&[(Reply, 0.02), (Like, 0.2)], 0.0)
        });
        assert_eq!(critique.verdict, CritiqueVerdict::Flat);
        assert_eq!(critique.predicted.get(Reply), 0.02);
        let text = render_critique(&critique);
        assert!(
            text.starts_with("Verdict: FLAT (ranker score 0.2"),
            "{text}"
        );
        assert!(text.contains("First reply a stranger types: \"gm\""));
        assert!(text.contains("1. end on the question\n2. cut the closing line"));
        assert!(render_critique(&to_critique(answer(&[], 0.0))).contains("nobody has one"));
    }

    #[test]
    fn trims_long_fields_rather_than_refusing_them() {
        let critique = to_critique(CritiqueAnswer {
            first_reply: "x".repeat(400),
            fixes: vec!["a".into(), "b".into(), "c".into(), "d".into()],
            summary: "s".repeat(300),
            ..answer(&[], 0.0)
        });
        assert_eq!(critique.first_reply.len(), 280);
        assert_eq!(critique.fixes, ["a", "b", "c"]);
        assert_eq!(critique.summary.len(), 200);
    }
}

fn model_answer() -> Value {
    let mut answer = json!({
        "firstReply": "mine is a for-loop",
        "disagreement": "nobody",
        "shareReason": "",
        "riskReason": "",
        "communityReply": 0.1,
        "fixes": ["ask what they run locally"],
        "summary": "worthy",
    });
    for action in ACTIONS {
        answer[action.as_str()] = json!(0.0);
    }
    answer["reply"] = json!(0.3);
    answer["quote"] = json!(0.1);
    answer["like"] = json!(0.3);
    answer
}

mod the_critic {
    use super::*;

    fn input() -> CritiqueInput {
        CritiqueInput {
            kind: CritiqueKind::Caption,
            text: "gm. a lab shipped a model.".into(),
            media: Some("Tiny at a desk".into()),
            context: Some("Story: a model".into()),
        }
    }

    #[tokio::test]
    async fn hands_the_draft_the_media_line_and_the_context_to_the_model() {
        let model = Arc::new(ScriptedModel::new([Ok(model_answer())]));
        let critique = AlgoCritic::new(model.clone())
            .critique(&input())
            .await
            .unwrap();
        assert_eq!(critique.verdict, CritiqueVerdict::Worthy);
        let request = &model.requests()[0];
        assert_eq!(
            (request.name, request.temperature, request.max_tokens),
            ("ranker_read", 0.1, Some(2_000))
        );
        assert!(request.messages[0].content.contains("  report: -234"));
        assert_eq!(
            request.messages[1].content,
            "Kind: caption.\nContext (what it answers, quotes, or is about):\nStory: a model\n\nAttached media shows: Tiny at a desk\n\nThe draft:\ngm. a lab shipped a model."
        );
    }

    #[tokio::test]
    async fn asks_once_more_when_the_first_answer_was_cut_short() {
        let half = json!({ "firstReply": "gm" });
        let model = Arc::new(ScriptedModel::new([Ok(half), Ok(model_answer())]));
        let critique = AlgoCritic::new(model.clone())
            .critique(&input())
            .await
            .unwrap();
        assert_eq!(critique.summary, "worthy");
        let retry = &model.requests()[1];
        assert_eq!(retry.messages.len(), 3);
        assert!(retry.messages[2].content.contains("was cut short"));
    }

    #[tokio::test]
    async fn fails_with_the_first_failure_after_two_tries_so_callers_fail_open() {
        let model = Arc::new(ScriptedModel::new([
            Err("502".into()),
            Ok(json!({ "reply": 2.0 })),
        ]));
        let error = AlgoCritic::new(model).critique(&input()).await.unwrap_err();
        assert!(error.to_string().contains("502"), "{error}");
        let model = Arc::new(ScriptedModel::new([Ok(json!({})), Err("down".into())]));
        let error = AlgoCritic::new(model).critique(&input()).await.unwrap_err();
        assert!(error.to_string().contains("unreadable shape"), "{error}");
    }

    #[tokio::test]
    async fn refuses_a_probability_outside_zero_to_one() {
        let mut out_of_range = model_answer();
        out_of_range["reply"] = json!(1.5);
        let model = Arc::new(ScriptedModel::new([
            Ok(out_of_range.clone()),
            Ok(out_of_range),
        ]));
        assert!(AlgoCritic::new(model).critique(&input()).await.is_err());
    }
}
