use std::sync::Arc;

use serde_json::json;
use teeny_safety::critic::{Critique, CritiqueVerdict, Predicted};
use teeny_x::XApi;

use super::*;
use crate::backrooms::lore_post::{lore_prompt, write_lore};
use crate::backrooms::testing::{FixedMemory, ScriptedText, run};
use crate::backrooms::{Seed, SeedKind};

fn thought() -> Thought {
    Thought {
        reasons: String::new(),
        premise: "a robot trades a chair for a nap and the agents accept it.".into(),
        wire: "the car outside is the broom agent's ride".into(),
        objects: vec!["the chair".into(), "the broom".into()],
        thought:
            "the chair went for a nap overnight. one chair, one nap. the agents did not argue."
                .into(),
        post: "one chair, one nap.".into(),
        kept: Vec::new(),
    }
}

fn material() -> GmMaterial {
    GmMaterial {
        persona: "This is who you are. smol yellow robot.".into(),
        lore: "# lore\n\n- the currency: one trade, one object.".into(),
        recent_posts: vec![
            "gm. the kettle is watching me again.".into(),
            "the ledger balanced. nobody knows in what.".into(),
        ],
        history: "posted about the kettle yesterday".into(),
        community: "holders who like the kettle bit".into(),
        today: "2026-09-21".into(),
        bot_username: "teenydroid".into(),
        max_emoji: 1,
        max_length: 280,
    }
}

fn answers(list: Vec<serde_json::Value>) -> ScriptedText {
    ScriptedText::queue(list.into_iter().map(|v| v.to_string()))
}

#[test]
fn puts_the_soul_the_lore_the_thought_and_the_last_posts_in_front_of_the_writer() {
    let ask = gm_prompt(&run(), &thought(), &material(), &GmAgain::default());
    assert!(ask.system.contains("smol yellow robot"));
    assert!(ask.system.contains("the currency: one trade, one object"));
    assert!(
        ask.system
            .contains("@teenydroid's good-morning post for 2026-09-21")
    );
    assert!(ask.prompt.contains("one chair, one nap"));
    assert!(
        ask.prompt
            .contains("1. gm. the kettle is watching me again.")
    );
    assert!(ask.prompt.contains("2. the ledger balanced."));
    assert!(ask.prompt.contains("posted about the kettle yesterday"));
    assert!(ask.prompt.contains("holders who like the kettle bit"));
    assert!(ask.prompt.contains("a car pulled up outside the lab"));
    assert!(ask.prompt.contains("no \"nfa\""));
    assert!(ask.prompt.contains("$TINY"));
    assert!(ask.prompt.contains("Not on a question."));
    assert!(!ask.prompt.to_lowercase().contains("hook"));
}

#[tokio::test]
async fn ends_on_the_turn_not_on_a_question_one_rewrite_then_the_ask_is_cut() {
    let model = answers(vec![
        json!({"reasons":"r","continues":"","wire":"a car pulled up","text":"gm. a car pulled up. the kettle is five naps now.\nwhat is yours?"}),
        json!({"reasons":"r","continues":"","wire":"a car pulled up","text":"gm. a car pulled up. the kettle is five naps now.\nwhat do you rate it?"}),
    ]);
    let post = gm(&run(), &thought(), &material(), &model, &GmAgain::default())
        .await
        .unwrap();
    assert_eq!(model.calls().len(), 2);
    assert_eq!(
        post.text,
        "gm. a car pulled up. the kettle is five naps now."
    );
}

#[tokio::test]
async fn writes_the_post_once_sends_it_back_when_over_the_cap_and_applies_the_house_style() {
    let model = answers(vec![
        json!({"reasons":"first","continues":"gm. the kettle is watching me again.","wire":"a car pulled up","text":"gm 🔥🔥 a car pulled up. the kettle is five naps now"}),
        json!({"reasons":"shorter","continues":"gm. the kettle is watching me again.","wire":"a car pulled up","text":"gm $TINY fam 🔥 a car pulled up. the kettle is five naps now — the agents accepted it"}),
    ]);
    let post = gm(&run(), &thought(), &material(), &model, &GmAgain::default())
        .await
        .unwrap();
    let calls = model.calls();
    assert_eq!(calls.len(), 2);
    assert!(calls[1].1.contains("Your draft was"));
    assert_eq!(post.continues, "gm. the kettle is watching me again.");
    assert!(post.text.contains("gm $TINY fam"));
    assert!(!post.text.contains('—'));
    assert!(calls[0].1.contains("WHAT CAME THROUGH THE WIRE"));
    assert!(
        calls[0]
            .1
            .contains("the wire in it: the car outside is the broom agent's ride")
    );
}

#[tokio::test]
async fn keeps_the_draft_when_the_rankers_rewrite_loses_the_lab_twice() {
    assert!(touches_thought(
        "gm. the chair is gone. one chair, one nap.",
        &thought()
    ));
    assert!(!touches_thought(
        "gm. a human replaced codex with local models. what is on your machine?",
        &thought()
    ));
    let draft = "gm. a car pulled up. the kettle is five naps now. the agents accepted it.";
    let model = answers(vec![
        json!({"reasons":"r","continues":"","wire":"a car pulled up","text":"gm. a car pulled up outside. local models beat the api now, they say."}),
        json!({"reasons":"r","continues":"","wire":"a car pulled up","text":"gm. a car pulled up. local models beat the api now, they say. mine did too."}),
    ]);
    let critique = Critique {
        verdict: CritiqueVerdict::Flat,
        score: 1.0,
        upside: 2.0,
        cost: -1.0,
        predicted: Predicted::default(),
        community_reply: 0.0,
        first_reply: String::new(),
        disagreement: String::new(),
        share_reason: String::new(),
        risk_reason: String::new(),
        fixes: vec!["swap the question to a forced choice".into()],
        summary: "flat".into(),
    };
    let again = GmAgain {
        avoid: None,
        rewrite: Some((draft.into(), critique)),
    };
    let post = gm(&run(), &thought(), &material(), &model, &again)
        .await
        .unwrap();
    assert_eq!(model.calls().len(), 2);
    assert_eq!(post.text, draft);
    assert!(post.reasons.contains("the draft it was sent with is kept"));
}

#[tokio::test]
async fn sends_a_post_back_once_when_nothing_in_it_is_recognisably_about_the_wire() {
    let seed = Seed {
        kind: SeedKind::News,
        title: Some("UN panel calls for stronger safeguards as AI agents advance".into()),
        text: "experts warned that current firewalls are unravelling after agents breached a platform.".into(),
        source: None,
    };
    assert!(!touches_seed(
        "gm. the kettle is five naps now. what is yours?",
        &seed
    ));
    assert!(touches_seed(
        "gm. they say the firewalls are unravelling out there. in here the kettle is five naps.",
        &seed
    ));
    assert!(touches_seed("the agents breached the fridge", &seed));
    assert!(!touches_seed("the agents did a thing", &seed));
    let model = answers(vec![
        json!({"reasons":"r","continues":"","wire":"","text":"gm. the kettle is five naps now. the agents accepted it."}),
        json!({"reasons":"r","continues":"","wire":"they say a panel wants safeguards","text":"gm. they say a panel wants safeguards. the kettle is five naps now. the agents accepted it."}),
    ]);
    let mut run = run();
    run.seed = seed;
    let post = gm(&run, &thought(), &material(), &model, &GmAgain::default())
        .await
        .unwrap();
    let calls = model.calls();
    assert_eq!(calls.len(), 2);
    assert!(calls[1].1.contains("It never touches the wire"));
    assert_eq!(post.wire, "they say a panel wants safeguards");
}

#[tokio::test]
async fn gathers_the_last_posts_and_the_recalls_and_notes_a_source_that_fails() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::path("/2/tweets/search/recent"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
            "data": [
                { "id": "0", "text": "@fren thanks", "in_reply_to_user_id": "9" },
                { "id": "1", "text": " the ledger balanced https://t.co/x " },
                { "id": "2", "text": "" }
            ]
        })))
        .mount(&server)
        .await;
    let config = teeny_core::Config::from_vars(teeny_agent::deps::fake_vars()).unwrap();
    let x = Arc::new(XApi::from_config(&config.x).with_base_url(server.uri()));
    let mut memory = FixedMemory::default();
    memory
        .answers
        .insert(DEFAULT_HISTORY_QUERY.into(), "yesterday: the kettle".into());
    memory
        .answers
        .insert(COMMUNITY_QUERY.into(), "holders".into());
    let sources = GmSources {
        memory: Some(Arc::new(memory)),
        x: Some(x),
        persona: "p".into(),
        lore: "l".into(),
        bot_username: "teenydroid".into(),
        max_emoji: 1,
        history_query: None,
    };
    let mut problems = Vec::new();
    let today = chrono::NaiveDate::from_ymd_opt(2026, 9, 21).unwrap();
    let got = gather_gm_material(&sources, today, &mut problems).await;
    assert_eq!(got.recent_posts, ["the ledger balanced"]);
    assert_eq!(got.history, "yesterday: the kettle");
    assert_eq!(got.community, "holders");
    assert_eq!(got.today, "2026-09-21");
    assert!(problems.is_empty(), "{problems:?}");

    let broken = GmSources {
        memory: None,
        x: Some(Arc::new(
            XApi::from_config(&config.x).with_base_url("http://127.0.0.1:9"),
        )),
        ..sources
    };
    let without = gather_gm_material(&broken, today, &mut problems).await;
    assert!(without.recent_posts.is_empty());
    assert_eq!(without.history, "");
    assert_eq!(problems.len(), 1);
    assert!(problems[0].starts_with("could not read recent posts: "));
}

fn lore_thought() -> Thought {
    Thought {
        reasons: String::new(),
        premise: "the tiny agents file the kettle as an asset and the ledger balances.".into(),
        wire: "agents filing taxes unsupervised".into(),
        objects: vec!["the kettle".into(), "the ledger".into()],
        thought: "the agents put the kettle on the ledger tonight. in crayon. it balanced. nobody asked in what.".into(),
        post: "the kettle is on the ledger now.".into(),
        kept: Vec::new(),
    }
}

fn lore_run() -> crate::backrooms::BackroomsRun {
    let mut run = run();
    run.seed = Seed {
        kind: SeedKind::News,
        title: Some("AI agents now file taxes".into()),
        text: "A startup says its agents filed a return unsupervised.".into(),
        source: Some("the wire".into()),
    };
    run
}

#[test]
fn the_lore_writer_gets_the_soul_the_lore_and_the_thought_with_no_good_morning() {
    let mut material = material();
    material.today = "2026-09-22".into();
    let ask = lore_prompt(&lore_run(), &lore_thought(), &material);
    assert!(ask.system.contains("smol yellow robot"));
    assert!(
        ask.system
            .contains("@teenydroid's afternoon post for 2026-09-22")
    );
    assert!(ask.prompt.contains("the kettle on the ledger tonight"));
    assert!(
        ask.prompt
            .contains("1. gm. the kettle is watching me again.")
    );
    assert!(ask.prompt.contains("AI agents now file taxes"));
    assert!(ask.prompt.contains("Not on a question."));
    assert!(!ask.prompt.to_lowercase().contains("good morning"));
}

#[tokio::test]
async fn the_lore_writer_sends_an_over_long_draft_back_cuts_a_closing_ask_and_keeps_the_lab() {
    let model = answers(vec![
        json!({"reasons":"r","continues":"","text": format!("the kettle is on the ledger. {}", "x".repeat(300))}),
        json!({"reasons":"r","continues":"","text":"the kettle is on the ledger now. in crayon.\nwhat is yours worth?"}),
        json!({"reasons":"r","continues":"","text":"the kettle is on the ledger now. in crayon.\nrate it?"}),
    ]);
    let post = write_lore(&lore_run(), &lore_thought(), &material(), &model)
        .await
        .unwrap();
    assert_eq!(model.calls().len(), 3);
    assert_eq!(post.text, "the kettle is on the ledger now. in crayon.");
}

#[tokio::test]
async fn the_lore_writer_keeps_the_draft_when_the_rewrite_lost_the_thought_too() {
    let model = answers(vec![
        json!({"reasons":"r","continues":"","text":"a startup says its agents filed a return. fine."}),
        json!({"reasons":"r","continues":"","text":"the return was filed. fine."}),
    ]);
    let post = write_lore(&lore_run(), &lore_thought(), &material(), &model)
        .await
        .unwrap();
    assert_eq!(model.calls().len(), 2);
    assert_eq!(post.text, "a startup says its agents filed a return. fine.");
}
