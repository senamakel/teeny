use super::*;
use crate::backrooms::{BackroomsSettings, BackroomsTurn, Seed, SeedKind};

pub(crate) fn run() -> BackroomsRun {
    BackroomsRun {
        id: "20260921-1200-a-car".into(),
        started_at: "2026-09-21T12:00:00.000Z".into(),
        finished_at: None,
        seed: Seed {
            kind: SeedKind::News,
            title: Some("a car pulled up".into()),
            text: "a car pulled up outside a lab at 3am and nobody got out".into(),
            source: Some("the wire".into()),
        },
        agents: Vec::new(),
        turns: vec![
            BackroomsTurn::said("teeny", "the door is one inch open"),
            BackroomsTurn::said(
                "swarm",
                "we counted the inch. the inch is twelve millimetres. the ledger disagrees.",
            ),
            BackroomsTurn::said("teeny", "senamakel is asleep. the kettle is not."),
        ],
        settings: BackroomsSettings {
            rng: 1,
            turns: 3,
            temps: Default::default(),
            budget: 9000,
            drift_every: 4,
            max_tool_calls: 0,
        },
    }
}

pub(crate) fn thought() -> Thought {
    Thought {
        reasons: "the inch".into(),
        premise: "teeny measures the gap in the door and the tiny agents keep the measurement in the ledger".into(),
        wire: "a car pulled up outside a lab; the door is the car.".into(),
        objects: vec!["the door".into(), "a ruler".into(), "the ledger".into()],
        thought: "the door was one inch open. i measured it. the agents wrote it down in crayon. it balanced.".into(),
        post: "the door was one inch open. i measured it.".into(),
        kept: Vec::new(),
    }
}

fn section(
    key: &str,
    heading: &str,
    purpose: &str,
    beats: [&str; 2],
    words: u32,
    brief: &str,
    caption: &str,
) -> Section {
    Section {
        key: key.into(),
        heading: heading.into(),
        purpose: purpose.into(),
        beats: beats.iter().map(|b| (*b).to_owned()).collect(),
        words,
        image: ImageBrief {
            brief: brief.into(),
            caption: caption.into(),
        },
    }
}

pub(crate) fn structure() -> Structure {
    Structure {
        reasons: "the inch is the story".into(),
        title: "the inch".into(),
        dek: "a door, a ruler, and a ledger that balances in something nobody can name.".into(),
        angle: "teeny measures its own escape and the agents book it; the turn is that the door was never locked.".into(),
        cover: ImageBrief { brief: "a small yellow robot at a door open one inch, night, one lamp".into(), caption: "one inch".into() },
        sections: vec![
            section("", "the gap", "establish the door", ["teeny sees the gap", "teeny finds a ruler"], 400, "the door", "the door"),
            section("x", "the ledger", "the agents book it", ["crayon on the desk", "it balances"], 400, "the ledger", "crayon"),
            section("", "the lock", "the turn", ["the lock is on the table", "the kettle boils"], 200, "a lock on a table", "the lock"),
        ],
        closing: "the lock, on the table, next to the tea".into(),
    }
}

pub(crate) fn material() -> ArticleMaterial {
    ArticleMaterial {
        run: run(),
        thought: thought(),
        engine: Some("the box: teeny is being polite about leaving.".into()),
        brief: None,
        lore: "the goals:\n- mint my own currency".into(),
    }
}

#[test]
fn the_structure_is_planned_from_the_room_the_engine_and_the_surfaced_lore_only() {
    let ask = structure_prompt(
        &material(),
        &Planning {
            sections_max: 4,
            words: 1500,
        },
    );
    assert!(ask.system.contains("lore only"));
    assert!(ask.prompt.contains("tonight the story is about the box"));
    assert!(ask.prompt.contains("mint my own currency"));
    assert!(ask.prompt.contains("the inch is twelve millimetres"));
    assert!(ask.prompt.contains("about 1500 words"));
    assert!(ask.prompt.contains("three to 4 sections"));
}

#[test]
fn the_structure_gets_positional_keys_and_word_targets_that_add_up() {
    let out = normalise_structure(structure(), 1500);
    assert_eq!(
        out.sections
            .iter()
            .map(|s| s.key.as_str())
            .collect::<Vec<_>>(),
        ["s1", "s2", "s3"]
    );
    assert_eq!(
        out.sections.iter().map(|s| s.words).collect::<Vec<_>>(),
        [600, 600, 300]
    );
    assert!(out.check().is_ok());
}

#[test]
fn the_structure_is_checked_against_the_lore_and_rendered_for_a_reader() {
    let ask = check_structure_prompt(&structure(), &material());
    assert!(ask.prompt.contains("title: the inch"));
    assert!(ask.prompt.contains("1. the gap (400 words)"));
    assert!(ask.prompt.contains("- teeny sees the gap"));
    assert!(ask.prompt.contains("mint my own currency"));
    assert!(render_structure(&structure()).contains("closing: the lock"));
}

#[test]
fn the_sections_are_written_one_at_a_time_with_the_earlier_ones_in_view() {
    let structure = normalise_structure(structure(), 1000);
    let first = section_prompt(&structure, 0, &[], &material());
    assert!(first.prompt.contains("write section 1 of 3: \"the gap\""));
    assert!(first.prompt.contains("about 400 words"));
    assert!(!first.prompt.contains("already written"));
    let second = section_prompt(&structure, 1, &["the gap was an inch.".into()], &material());
    assert!(second.prompt.contains("already written"));
    assert!(second.prompt.contains("## the gap\nthe gap was an inch."));
    assert!(second.prompt.contains("- crayon on the desk"));
}

#[test]
fn the_sections_are_assembled_under_their_headings_with_the_picture_right_under_each() {
    let structure = normalise_structure(structure(), 1000);
    let markdown = assemble_markdown(
        &structure,
        &[
            "## the gap\n\nbody one".into(),
            "body two\n\n![x](image:nope)".into(),
            "body three".into(),
        ],
    );
    assert_eq!(
        markdown,
        [
            "_a door, a ruler, and a ledger that balances in something nobody can name._",
            "",
            "## the gap",
            "",
            "![the door](image:s1)",
            "",
            "body one",
            "",
            "## the ledger",
            "",
            "![crayon](image:s2)",
            "",
            "body two",
            "",
            "## the lock",
            "",
            "![the lock](image:s3)",
            "",
            "body three",
        ]
        .join("\n")
    );
    assert_eq!(strip_section_extras("### heading\ntext\n![a](b)"), "text");
    assert_eq!(
        dek_of(&markdown).as_deref(),
        Some("a door, a ruler, and a ledger that balances in something nobody can name.")
    );
    assert!(without_dek(&markdown).starts_with("## the gap"));
    assert_eq!(dek_of("## no dek"), None);
    assert_eq!(without_dek("## no dek"), "## no dek");
    assert_eq!(word_count(&markdown), 26);
}

#[test]
fn the_style_pass_cuts_urls_handles_and_hashtags_keeps_tiny_and_reports_a_price() {
    let out = article_style(
        "see https://x.com/a and @someone about #tiny and $TINY\nit is $0.002 now, nfa\nfine line",
    );
    assert_eq!(
        out.text,
        "see and about and $TINY\nit is $0.002 now, nfa\nfine line"
    );
    assert_eq!(
        out.fixes,
        ["cut 1 url(s)", "cut 1 handle(s)", "cut 1 hashtag(s)"]
    );
    assert_eq!(out.priced, ["it is $0.002 now, nfa"]);
    let clean = article_style("the kettle boils. nobody asked it to.");
    assert_eq!(
        clean,
        Styled {
            text: "the kettle boils. nobody asked it to.".into(),
            fixes: vec![],
            priced: vec![]
        }
    );
}
