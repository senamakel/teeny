use chrono::{TimeZone, Utc};

use super::*;
use crate::backrooms::distill::Thought;
use crate::backrooms::evaluate::stats;
use crate::backrooms::testing::{run, scratch};

#[test]
fn writes_the_run_as_json_and_as_a_transcript() {
    let dir = scratch("backrooms-store");
    let run = run();
    let extras = SavedExtras {
        stats: Some(stats(&run)),
        thought: Some(Thought {
            reasons: String::new(),
            premise: "a robot and a door.".into(),
            wire: String::new(),
            objects: vec!["door".into()],
            thought: "the door is open.".into(),
            post: "the door is open.".into(),
            kept: Vec::new(),
        }),
        ..SavedExtras::default()
    };
    let saved = save(&dir, &run, &extras).unwrap();
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&saved.json).unwrap()).unwrap();
    assert_eq!(json["id"], run.id);
    assert_eq!(json["stats"]["turns"], 4);
    assert_eq!(json["turns"][1]["who"], "swarm");
    assert_eq!(json["startedAt"], run.started_at);
    let markdown = std::fs::read_to_string(&saved.markdown).unwrap();
    assert!(markdown.contains("# backrooms 20260921-1200-a-car"));
    assert!(markdown.contains("### 2. swarm\n\n> looked up · x_search_posts · \"inch\"\n\nwe counted"));
    assert!(markdown.contains("### 3. teeny · mood → giddy"));
    assert!(markdown.contains(
        "## the thought\n\n*a robot and a door.*\n\nthe door is open.\n\n**as a post**\n\n> the door is open."
    ));
    assert!(markdown.find("## the thought") < markdown.find("## transcript"));
    assert!(markdown.contains("## stats"));
    assert!(!markdown.contains("## judge"));
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn names_a_run_by_the_minute_and_the_slug() {
    let at = Utc.with_ymd_and_hms(2026, 9, 21, 13, 35, 12).unwrap();
    assert_eq!(run_id(at, "A car pulled up!! outside"), "20260921-1335-a-car-pulled-up-outside");
    assert_eq!(run_id(at, "!!"), "20260921-1335");
}
