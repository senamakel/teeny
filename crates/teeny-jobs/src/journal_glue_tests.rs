use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;
use teeny_store::journal::{Outcome, Reflection, ReflectionInput, Tone, Trigger};
use teeny_store::{InMemoryJournal, Journal, StoreError};

use super::*;
use crate::runner::ToolCallRecord;

fn call(name: &str, result: &str) -> ToolCallRecord {
    ToolCallRecord {
        name: name.into(),
        arguments: json!({}),
        result: result.into(),
        is_error: false,
    }
}

// legacy/test/journal.test.ts: "cuts a memory block down to a few plain lines"
#[test]
fn cuts_a_memory_block_down_to_a_few_plain_lines() {
    let block = [
        "ABOUT @fren:".to_owned(),
        "- asked about the chart twice".into(),
        "* likes rust".into(),
        String::new(),
        "GENERAL CONTEXT:".into(),
        format!("> {}", "x".repeat(300)),
        "fifth line that is over the cap".into(),
    ]
    .join("\n");
    let lines = summarise_recall(&block, 4);
    assert_eq!(lines.len(), 4);
    assert_eq!(lines[0], "asked about the chart twice");
    assert_eq!(lines[2].chars().count(), 180);
    assert!(lines[2].ends_with('…'));
}

// "keeps the prose from a CortexDB block and drops the records"
#[test]
fn keeps_the_prose_from_a_cortex_block_and_drops_the_records() {
    let block = [
        "GENERAL CONTEXT:",
        "[2026-09-02 07:47 UTC] {\"redacted\":true}",
        "[2026-09-02 07:48 UTC] {\"RecordAsserted\":{\"record\":{\"id\":\"01a0\"}}}",
        "[2026-09-02 07:49 UTC] they asked about the chart and got a real answer",
        "- likes rust",
    ]
    .join("\n");
    assert_eq!(
        summarise_recall(&block, 4),
        vec![
            "they asked about the chart and got a real answer".to_owned(),
            "likes rust".into()
        ]
    );
    assert!(summarise_recall("[x] {\"redacted\":true}", 4).is_empty());
}

// "flattens a tool result to one line"
#[test]
fn flattens_a_tool_result_to_one_line() {
    assert_eq!(summarise_result(&"a".repeat(200), 160).chars().count(), 160);
    assert_eq!(summarise_result("", 160), "");
    assert_eq!(
        summarise_result(
            "GENERAL CONTEXT:\n[2026-09-02 07:47 UTC] {\"redacted\":true}\n[2026-09-13 18:23 UTC] [assistant] glad it landed",
            160
        ),
        "[assistant] glad it landed"
    );
    assert_eq!(
        summarise_result("id: 1\nauthor: @x\ntext: hi", 160),
        "id: 1 · author: @x · text: hi"
    );
}

#[test]
fn tool_steps_keep_the_name_and_a_one_line_result() {
    let steps = tool_steps(&[call("token_overview", "price\n and   volume")]);
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].name, "token_overview");
    assert_eq!(steps[0].result, "price · and volume");
}

// legacy/src/agent/graph.ts producedMediaId: the id must be quoted in a
// tool result of this turn, not merely look like one.
#[test]
fn a_media_id_counts_only_when_a_tool_of_the_turn_returned_it() {
    let calls = [call("generate_image", "{\"mediaId\":\"img_abc_1\"}")];
    assert!(produced_media_id(&calls, "img_abc_1"));
    assert!(!produced_media_id(&calls, "img_abc_2"));
    assert!(!produced_media_id(&[], "img_abc_1"));
}

struct BrokenJournal;

#[async_trait]
impl Journal for BrokenJournal {
    async fn record(&self, _entry: ReflectionInput) -> teeny_store::Result<()> {
        Err(StoreError::Io(std::io::Error::other("mongo is down")))
    }
    async fn recent(&self, _limit: usize) -> teeny_store::Result<Vec<Reflection>> {
        Ok(Vec::new())
    }
}

fn entry() -> ReflectionInput {
    ReflectionInput {
        id: Some("mention:1".into()),
        at: None,
        trigger: Trigger::Mention,
        incoming: "gm".into(),
        tone: Tone::Neutral,
        recalled: Vec::new(),
        tools: Vec::new(),
        thinking: String::new(),
        said: String::new(),
        declined: Vec::new(),
        outcome: Outcome::Skipped,
        outcome_note: None,
    }
}

#[tokio::test]
async fn a_journal_write_is_kept_and_a_broken_journal_is_only_logged() {
    let journal = Arc::new(InMemoryJournal::default());
    record_quietly(journal.as_ref(), entry()).await;
    assert_eq!(journal.recent(5).await.unwrap().len(), 1);
    record_quietly(&BrokenJournal, entry()).await;
}
