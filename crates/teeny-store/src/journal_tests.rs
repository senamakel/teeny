use serde_json::json;

use super::*;

fn blank() -> ReflectionInput {
    ReflectionInput {
        id: None,
        at: None,
        trigger: Trigger::Mention,
        incoming: String::new(),
        tone: Tone::Neutral,
        recalled: vec![],
        tools: vec![],
        thinking: String::new(),
        said: String::new(),
        declined: vec![],
        outcome: Outcome::Posted,
        outcome_note: None,
    }
}

fn at(id: &str, at: &str) -> ReflectionInput {
    ReflectionInput {
        id: Some(id.into()),
        at: Some(at.into()),
        ..blank()
    }
}

/// The journal's contract, run against every implementation.
async fn contract(journal: &dyn Journal) {
    journal
        .record(at("a", "2026-09-13T01:00:00.000Z"))
        .await
        .unwrap();
    journal
        .record(at("b", "2026-09-13T02:00:00.000Z"))
        .await
        .unwrap();
    // A retried turn cannot journal itself twice.
    journal
        .record(ReflectionInput {
            said: "again".into(),
            ..at("a", "2026-09-13T03:00:00.000Z")
        })
        .await
        .unwrap();

    let recent = journal.recent(10).await.unwrap();
    assert_eq!(
        recent.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        ["b", "a"]
    );
    assert_eq!(recent[1].said, "");
    assert_eq!(journal.recent(1).await.unwrap().len(), 1);

    // An entry without an id or a time gets both.
    journal
        .record(ReflectionInput {
            outcome: Outcome::DryRun,
            ..blank()
        })
        .await
        .unwrap();
    let newest = &journal.recent(1).await.unwrap()[0];
    assert!(newest.id.starts_with("r-"), "{}", newest.id);
    assert!(newest.at.as_str() > "2026-09-13T02:00:00.000Z");
    assert_eq!(newest.outcome, Outcome::DryRun);
}

#[tokio::test]
async fn keeps_newest_first_and_never_records_the_same_turn_twice() {
    contract(&InMemoryJournal::default()).await;
}

#[tokio::test]
async fn the_in_memory_journal_keeps_only_its_capacity() {
    let journal = InMemoryJournal::with_capacity(2);
    for (id, time) in [("a", "01"), ("b", "02"), ("c", "03")] {
        journal
            .record(at(id, &format!("2026-09-13T{time}:00:00.000Z")))
            .await
            .unwrap();
    }
    let ids: Vec<_> = journal
        .recent(10)
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.id)
        .collect();
    assert_eq!(ids, ["c", "b"]);
}

#[test]
fn blanks_every_handle_before_anything_is_shown() {
    assert_eq!(
        redact_handles("@someone hey @Tiny_Bot, @a1 look"),
        "@… hey @…, @… look"
    );
    // Longer than X allows is not a handle; a bare @ is not one either.
    assert_eq!(
        redact_handles("@abcdefghijklmnop and @ alone"),
        "@abcdefghijklmnop and @ alone"
    );
    assert_eq!(redact_handles("mail me@x.com"), "mail me@….com");

    let redacted = redact_reflection(&Reflection {
        id: "x".into(),
        at: "now".into(),
        trigger: Trigger::Mention,
        incoming: "@fren gm".into(),
        tone: Tone::Neutral,
        recalled: vec!["spoke with @fren".into()],
        tools: vec![ToolCall {
            name: "recall_memory".into(),
            result: "@fren asked before".into(),
        }],
        thinking: "I know @fren".into(),
        said: "gm @fren".into(),
        declined: vec!["tagging @other".into()],
        outcome: Outcome::Posted,
        outcome_note: None,
    });
    let shown = serde_json::to_string(&redacted).unwrap();
    assert!(!shown.contains("@f") && !shown.contains("@o"), "{shown}");
}

#[test]
fn cuts_a_memory_block_down_to_a_few_plain_lines() {
    let block = [
        "ABOUT @fren:",
        "- asked about the chart twice",
        "* likes rust",
        "",
        "GENERAL CONTEXT:",
        &format!("> {}", "x".repeat(300)),
        "fifth line that is over the cap",
    ]
    .join("\n");
    let lines = summarise_recall(&block, 4);
    assert_eq!(lines.len(), 4);
    assert_eq!(lines[0], "asked about the chart twice");
    assert_eq!(lines[2].chars().count(), 180);
    assert!(lines[2].ends_with('…'));
}

#[test]
fn keeps_the_prose_from_a_cortexdb_block_and_drops_the_records() {
    let block = [
        "GENERAL CONTEXT:",
        r#"[2026-09-02 07:47 UTC] {"redacted":true}"#,
        r#"[2026-09-02 07:48 UTC] {"RecordAsserted":{"record":{"id":"01a0"}}}"#,
        "[2026-09-02 07:49 UTC] they asked about the chart and got a real answer",
        "- likes rust",
    ]
    .join("\n");
    assert_eq!(
        summarise_recall(&block, 4),
        [
            "they asked about the chart and got a real answer",
            "likes rust"
        ]
    );
    assert!(summarise_recall(r#"[x] {"redacted":true}"#, 4).is_empty());
}

#[test]
fn flattens_a_tool_result_to_one_line() {
    // serde_json orders keys; the TypeScript test had them as written.
    assert_eq!(
        summarise_result(Some(&json!({ "note": "a\n  b", "price": 1 })), 160),
        r#"{"note":"a\n b","price":1}"#
    );
    assert_eq!(
        summarise_result(Some(&json!("a".repeat(200))), 160)
            .chars()
            .count(),
        160
    );
    assert_eq!(summarise_result(None, 160), "");
    assert_eq!(
        summarise_text(
            "GENERAL CONTEXT:\n[2026-09-02 07:47 UTC] {\"redacted\":true}\n[2026-09-13 18:23 UTC] [assistant] glad it landed",
            160
        ),
        "[assistant] glad it landed"
    );
    assert_eq!(
        summarise_text("id: 1\nauthor: @x\ntext: hi", 160),
        "id: 1 · author: @x · text: hi"
    );
}

#[test]
fn a_reflection_is_spelled_as_the_typescript_service_stored_it() {
    let reflection = Reflection {
        id: "r".into(),
        at: "2026-09-13T01:00:00.000Z".into(),
        trigger: Trigger::Chart,
        incoming: "in".into(),
        tone: Tone::Hostile,
        recalled: vec![],
        tools: vec![ToolCall {
            name: "t".into(),
            result: "r".into(),
        }],
        thinking: "th".into(),
        said: "s".into(),
        declined: vec![],
        outcome: Outcome::DryRun,
        outcome_note: Some("why".into()),
    };
    assert_eq!(
        serde_json::to_value(&reflection).unwrap(),
        json!({
            "id": "r", "at": "2026-09-13T01:00:00.000Z", "trigger": "chart", "incoming": "in",
            "tone": "hostile", "recalled": [], "tools": [{ "name": "t", "result": "r" }],
            "thinking": "th", "said": "s", "declined": [], "outcome": "dry-run", "outcomeNote": "why",
        })
    );
}

#[tokio::test]
#[ignore = "needs a Mongo at TEENY_TEST_MONGO_URI"]
async fn the_mongo_journal_keeps_the_contract() {
    let Some(db) = crate::mongo::testing::database().await else {
        return;
    };
    let name = crate::mongo::testing::collection("reflections");
    contract(&MongoJournal::with_collection(&db, &name)).await;
    db.collection::<mongodb::bson::Document>(&name)
        .drop()
        .await
        .unwrap();
}
