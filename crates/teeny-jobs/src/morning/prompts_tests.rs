use super::*;
use crate::morning::stages::{Pick, Scene};

fn context(persona: &str) -> StageContext {
    StageContext {
        persona: persona.into(),
        bot_username: "TinyMascot".into(),
        today: "2026-09-14".into(),
    }
}

fn pick() -> Pick {
    Pick {
        shortlist: Vec::new(),
        story: 1,
        headline: "A model launched".into(),
        topic: "ai".into(),
        hook: String::new(),
        angle: "local-first has no status page".into(),
        visual: "a board".into(),
        fallback: None,
    }
}

#[test]
fn writes_the_script_as_tiny_with_the_persona_and_its_skills_in_front() {
    let scene = Scene {
        concepts: vec!["a dead status board".into()],
        scene: "A status board over a stage.".into(),
        story_cue: "the board".into(),
        alt: "a board".into(),
    };
    let ask = script_prompt(
        &context("SOUL\n<skill name=\"video-script\">beats</skill>"),
        &pick(),
        &scene,
        10,
    );
    assert!(
        ask.system
            .starts_with("SOUL\n<skill name=\"video-script\">beats</skill>\n")
    );
    assert!(ask.system.contains("filming"));
    assert!(ask.prompt.contains("10-second script"));
    assert!(ask.prompt.contains("local-first has no status page"));
}

#[test]
fn reads_the_skills_into_the_persona_as_the_legacy_did() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../skills");
    let rendered = with_skills("SOUL", dir);
    assert!(rendered.starts_with("SOUL\n\nThe following skill is active."));
    assert!(rendered.contains("<skill name=\"video-script\">"));
    assert_eq!(with_skills("SOUL", "/nowhere/at/all"), "SOUL");
}

#[test]
fn tells_the_picker_what_was_turned_down_and_why() {
    let ask = pick_prompt(
        &context("You are Tiny."),
        &PickInput {
            stories: "[1] A story".into(),
            rejected: vec![Rejected {
                story: 1,
                headline: "A story".into(),
                verdict: "repeat".into(),
                reason: "posted on Monday".into(),
            }],
            ..PickInput::default()
        },
    );
    assert!(
        ask.system
            .contains("@TinyMascot's good-morning post is about today, 2026-09-14.")
    );
    assert!(ask.prompt.contains("(the timeline could not be read)"));
    assert!(
        ask.prompt
            .contains("- [1] A story: repeat, posted on Monday")
    );
    assert!(
        ask.prompt
            .ends_with("\"angle\": string, \"visual\": string, \"fallback\"?: string}")
    );
}

#[test]
fn puts_a_refused_scene_in_front_of_the_writer() {
    let refused = RefusedScene {
        category: "violence".into(),
        scene: "the old scene".into(),
    };
    let ask = scene_prompt(&pick(), "", Some("gold light"), Some(&refused));
    assert!(ask.prompt.contains("refused your last scene as violence"));
    assert!(ask.prompt.contains("the old scene"));
    assert!(
        ask.prompt
            .contains("(could not be read; work from the headline)")
    );
    assert!(
        ask.prompt
            .contains("palette and mood only; its scene is irrelevant here:\ngold light")
    );
}

#[test]
fn the_caption_does_not_narrate_the_attachment_and_avoids_a_repeat() {
    let avoid = Avoid {
        said: "gm, stars".into(),
        reason: "same joke".into(),
    };
    let ask = caption_prompt(
        &context("P"),
        &pick(),
        Attached::Video,
        "",
        1,
        280,
        Some(&avoid),
    );
    assert!(
        ask.prompt
            .contains("A short clip of Tiny in the story is attached. Do NOT describe it")
    );
    assert!(ask.prompt.contains("YOU HAVE ALREADY SAID THIS"));
    assert!(ask.prompt.contains("(same joke)"));
    assert!(ask.prompt.contains("Under 280 characters in total"));
    assert_eq!(
        ask.system,
        "P\n\nYou are writing @TinyMascot's good-morning post for 2026-09-14."
    );
}

#[test]
fn checks_freshness_of_a_subject_and_of_a_post_differently() {
    let subject = freshness_prompt(&context("P"), FreshnessKind::Subject, "Headline: h", "");
    assert!(
        subject
            .prompt
            .starts_with("THE SUBJECT CHOSEN FOR TODAY'S POST:\nHeadline: h")
    );
    assert!(subject.prompt.contains("(nothing recalled)"));
    assert!(subject.prompt.contains("- stale: the moment has passed."));
    let post = freshness_prompt(&context("P"), FreshnessKind::Post, "gm", "said gm");
    assert!(post.prompt.contains("- stale: never for a post"));
}
