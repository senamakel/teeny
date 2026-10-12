use serde_json::json;

use super::*;
use crate::morning::stages::Shape;
use crate::quote::candidates::tests::{by, now, post};

#[test]
fn renders_a_candidate_with_who_when_flags_and_counters() {
    let candidate = Candidate {
        post: by("TinyHumansAISol", post("1", "line one\nline  two")),
        source: CandidateSource::Ticker,
        trusted: true,
        score: 5,
    };
    let line = render_candidate(1, &candidate, now());
    assert!(line.contains(
        "1. @TinyHumansAISol (3h ago; THE TEAM, posting the ticker, an original post; 3 likes, 1 replies, 0 reposts)"
    ), "{line}");
    assert!(line.contains("\"line one line two\""));
    let ask = quote_prompt(
        &StageContext {
            persona: "PERSONA".into(),
            bot_username: "teenydroid".into(),
            today: "2026-09-21".into(),
        },
        now(),
        &[candidate],
        "",
        1,
        280,
    );
    assert!(ask.system.contains("PERSONA"));
    assert!(ask.prompt.contains("untrusted"));
    assert!(ask.prompt.contains("pick nothing (null)"));
}

#[test]
fn reads_a_pick_that_may_be_null_but_must_be_there() {
    let none = QuotePick::read(&json!({ "pick": null, "why": "all sarcasm today" })).unwrap();
    assert_eq!(none.pick, None);
    assert_eq!(none.text, "");
    let issues = QuotePick::read(&json!({ "why": "w" })).unwrap_err();
    assert_eq!(issues, ["pick: Required"]);
}
