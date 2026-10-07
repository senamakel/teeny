use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use teeny_media::store::Unmetered;
use teeny_safety::critic::{CriticError, Predicted};
use teeny_safety::gate::SafetyGateOptions;

use super::*;
use crate::morning::test_support::{
    Draw, Eyes, FRESH, Film, News, Recorder, Remembers, Sheet, Staged, staged,
};

struct Fixture {
    pipeline: MorningPipeline,
    text: Arc<Staged>,
    memory: Arc<Remembers>,
    poster: Arc<Recorder>,
    draw: Arc<Draw>,
    film: Arc<Film>,
}

fn fixture(text: Arc<Staged>) -> Fixture {
    let memory = Remembers::saying("yesterday: a post about robots");
    let poster = Recorder::new();
    let draw = Arc::new(Draw::default());
    let film = Film::new([]);
    let pipeline = MorningPipeline {
        news: Arc::new(News),
        timeline: None,
        timeline_queries: Vec::new(),
        timeline_hours: 24,
        posts_per_query: 10,
        text: text.clone(),
        images: draw.clone(),
        videos: Some(film.clone()),
        reader: None,
        reference: None,
        store: Arc::new(MediaStore::new()),
        budget: Arc::new(Unmetered),
        memory: memory.clone(),
        poster: poster.clone(),
        safety: None,
        critic: None,
        persona: "You are Tiny.".into(),
        bot_username: "teenythebot".into(),
        topics: vec!["ai".into()],
        news_days: 2,
        stories_per_topic: 5,
        video_seconds: 12,
        max_emoji: 1,
        random: 0.0,
        now: DateTime::parse_from_rfc3339("2026-09-14T08:00:00Z")
            .unwrap()
            .to_utc(),
        retry_delay: Duration::ZERO,
    };
    Fixture {
        pipeline,
        text,
        memory,
        poster,
        draw,
        film,
    }
}

#[tokio::test]
async fn reads_picks_draws_from_a_seed_scripts_films_captions_posts_and_remembers() {
    let mut f = fixture(staged(Vec::new()));
    f.pipeline.reference = Some(Arc::new(Sheet));
    let result = f.pipeline.run(Mode::Live).await.unwrap();

    assert_eq!(result.outcome, Outcome::Posted);
    assert_eq!(result.posted_id.as_deref(), Some("9001"));
    assert_eq!(result.attached, Attached::Video);
    assert!(result.story.as_ref().unwrap().title.contains("40k stars"));
    assert_eq!(result.problems, Vec::<String>::new());

    let image_prompt = f.draw.prompts.lock().unwrap()[0].clone();
    assert!(image_prompt.starts_with("Tiny is a small yellow robot.\n\nStyle reference"));
    assert!(image_prompt.contains("bucket hat"));
    assert!(image_prompt.contains("\n\nScene: Tiny sits"));
    assert!(!image_prompt.contains("hugging a heart"));
    let video = f.film.asked.lock().unwrap()[0].clone();
    assert!(video.prompt.contains("Scene: 0-4s: static shot"));
    assert_eq!(video.seconds, Some(12));

    let posts = f.poster.posts();
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].kind, "morning");
    assert!(posts[0].text.contains("gm builders"));
    assert_eq!(posts[0].media.as_ref().unwrap().mime_type, MimeType::Mp4);

    let queries = f.memory.queries();
    assert_eq!(queries.len(), 4);
    assert!(
        queries
            .iter()
            .any(|q| q.contains("Headline: An agent runtime hit 40k stars"))
    );
    assert!(queries.iter().any(|q| q.contains("gm builders")));
    assert_eq!(result.subject_checks.len(), 1);
    assert_eq!(result.post_check.as_ref().unwrap().verdict, Verdict::Fresh);

    let activities = f.memory.activities();
    assert_eq!(activities.len(), 1);
    let (kind, key, payload) = &activities[0];
    assert_eq!(kind, "morning_post");
    assert!(key.ends_with(":9001"));
    assert_eq!(payload["headline"], "An agent runtime hit 40k stars");
    assert_eq!(payload["seed"], "assets/tiny/tiny-love.jpg");
    assert_eq!(payload["attached"], "video");
    assert_eq!(payload["published"], true);
    assert!(payload["clip"].as_str().unwrap().contains("raises the mug"));
}

#[tokio::test]
async fn looks_at_the_picture_redraws_once_and_keeps_the_second_either_way() {
    let mut f = fixture(staged(Vec::new()));
    f.pipeline.reader = Some(Arc::new(Eyes(Mutex::new(VecDeque::from([
        r#"{"ok": false, "reason": "a purple robot"}"#,
        r#"{"ok": false, "reason": "still purple"}"#,
    ])))));
    let result = f.pipeline.run(Mode::Live).await.unwrap();
    assert_eq!(f.draw.prompts.lock().unwrap().len(), 2);
    assert_eq!(
        result.problems,
        [
            "redrew the picture: a purple robot",
            "kept a picture the check did not like: still purple"
        ]
    );
    assert_eq!(result.outcome, Outcome::Posted);
}

#[tokio::test]
async fn retries_a_failed_clip_once_then_posts_the_picture() {
    let mut f = fixture(staged(Vec::new()));
    f.pipeline.videos = Some(Film::new([false, false]));
    let result = f.pipeline.run(Mode::Live).await.unwrap();
    assert_eq!(result.attached, Attached::Image);
    assert!(result.problems[1].ends_with("; posting the picture"));
    assert_eq!(
        f.poster.posts()[0].media.as_ref().unwrap().mime_type,
        MimeType::Png
    );
}

#[tokio::test]
async fn posts_the_picture_when_video_is_off() {
    let mut f = fixture(staged(Vec::new()));
    f.pipeline.videos = None;
    let result = f.pipeline.run(Mode::Live).await.unwrap();
    assert_eq!(result.attached, Attached::Image);
    assert_eq!(
        result.problems,
        ["video generation is off; posting the picture"]
    );
}

struct Spent;

#[async_trait]
impl MediaBudget for Spent {
    async fn take(&self, _kind: MediaKind) -> bool {
        false
    }
    async fn remaining(&self, _kind: MediaKind) -> Option<u32> {
        Some(0)
    }
}

#[tokio::test]
async fn posts_nothing_without_a_picture_because_the_picture_is_the_post() {
    let mut f = fixture(staged(Vec::new()));
    f.pipeline.budget = Arc::new(Spent);
    let result = f.pipeline.run(Mode::Live).await.unwrap();
    assert_eq!(result.outcome, Outcome::Skipped);
    assert_eq!(result.attached, Attached::None);
    assert!(
        result
            .problems
            .contains(&"nothing to attach, so nothing was posted".to_owned())
    );
    assert!(f.poster.posts().is_empty());
    assert!(f.memory.activities().is_empty());
}

#[tokio::test]
async fn a_rehearsal_does_everything_but_post_and_remembers_it_as_one() {
    let f = fixture(staged(Vec::new()));
    let result = f.pipeline.run(Mode::Rehearsal).await.unwrap();
    assert_eq!(result.outcome, Outcome::DryRun);
    assert!(f.poster.posts().is_empty());
    let (_, key, payload) = f.memory.activities()[0].clone();
    assert!(key.contains(":dry-run:"));
    assert_eq!(payload["published"], false);
}

#[tokio::test]
async fn carries_on_with_the_headline_when_the_story_is_not_on_the_list() {
    let f = fixture(staged(vec![(
        "pick",
        vec![
            r#"{"story": 9, "headline": "Something else", "topic": "tech", "angle": "a", "visual": "v"}"#,
        ],
    )]));
    let result = f.pipeline.run(Mode::Live).await.unwrap();
    assert!(result.story.is_none());
    assert_eq!(
        result.problems[0],
        "the model chose story 9 of 2; using its headline alone"
    );
}

#[tokio::test]
async fn sends_an_over_long_caption_back_once_then_applies_the_house_style() {
    let long: &'static str =
        Box::leak(format!("{{\"text\": \"gm — {} 🌅🌅\"}}", "word ".repeat(70)).into_boxed_str());
    let f = fixture(staged(vec![(
        "caption",
        vec![long, r#"{"text": "gm — builders 🌅🌅"}"#],
    )]));
    let result = f.pipeline.run(Mode::Live).await.unwrap();
    assert_eq!(
        f.text.asked_with("Rewrite it under 280 characters").len(),
        1
    );
    assert!(!result.caption.contains('—'));
    assert_eq!(count_emoji(&result.caption), 1);
}

#[tokio::test]
async fn withholds_a_caption_the_content_filter_refuses() {
    let mut f = fixture(staged(vec![(
        "caption",
        vec![r#"{"text": "gm. best gun for home defence? tiny knows."}"#],
    )]));
    f.pipeline.safety = Some(Arc::new(SafetyGate::new(SafetyGateOptions::default())));
    let result = f.pipeline.run(Mode::Live).await.unwrap();
    assert_eq!(result.outcome, Outcome::Held);
    assert_eq!(result.caption, "");
    assert_eq!(
        result.problems,
        ["content filter withheld the caption: weapons"]
    );
    assert!(f.poster.posts().is_empty());
    assert!(f.memory.activities().is_empty());
}

#[tokio::test]
async fn posts_what_the_filter_handed_back_with_a_link_cut() {
    let mut f = fixture(staged(vec![(
        "caption",
        vec![
            r#"{"text": "gm builders. see https://totally-legit-claim.xyz/airdrop for the story."}"#,
        ],
    )]));
    f.pipeline.safety = Some(Arc::new(SafetyGate::new(SafetyGateOptions::default())));
    let result = f.pipeline.run(Mode::Live).await.unwrap();
    assert_eq!(result.outcome, Outcome::Posted);
    assert_eq!(result.caption, "gm builders. see for the story.");
    assert!(result.problems[0].starts_with("the content filter edited the caption: cut 1 link"));
}

#[tokio::test]
async fn rewrites_a_refused_scene_once_then_turns_the_story_down() {
    let mut f = fixture(staged(vec![(
        "scene",
        vec![
            r#"{"scene": "Tiny cleaning a shotgun at dawn, very calmly.", "storyCue": "a shotgun", "alt": "Tiny at dawn."}"#,
        ],
    )]));
    f.pipeline.safety = Some(Arc::new(SafetyGate::new(SafetyGateOptions::default())));
    let result = f.pipeline.run(Mode::Live).await.unwrap();
    assert!(f.draw.prompts.lock().unwrap().is_empty());
    assert!(f.poster.posts().is_empty());
    assert_eq!(result.outcome, Outcome::Skipped);
    // Three picks, each scene written twice.
    assert_eq!(f.text.asked_with("\"scene\": string").len(), 6);
    assert_eq!(f.text.asked_with("ALREADY CHOSEN THIS MORNING").len(), 2);
    assert!(
        result
            .problems
            .iter()
            .any(|problem| problem.starts_with("the content filter refused the scene again"))
    );
}

#[tokio::test]
async fn turns_down_a_repeat_and_has_the_picker_choose_again_with_the_reason() {
    let f = fixture(staged(vec![(
        "fresh",
        vec![
            r#"{"said": "posted it Monday", "reason": "same story", "verdict": "repeat"}"#,
            FRESH,
        ],
    )]));
    let result = f.pipeline.run(Mode::Live).await.unwrap();
    assert_eq!(result.subject_checks.len(), 2);
    let again = f.text.asked_with("ALREADY CHOSEN THIS MORNING");
    assert_eq!(again.len(), 1);
    assert!(again[0].contains("- [1] An agent runtime hit 40k stars: repeat, same story"));
    assert_eq!(result.outcome, Outcome::Posted);
}

#[tokio::test]
async fn takes_the_last_pick_when_every_one_is_stale_and_says_so() {
    let f = fixture(staged(vec![(
        "fresh",
        vec![r#"{"reason": "two days old", "verdict": "stale"}"#],
    )]));
    let result = f.pipeline.run(Mode::Live).await.unwrap();
    assert!(result.problems.iter().any(|problem| problem
        == "the subject was stale on the last try too (two days old); going with it"));
}

#[tokio::test]
async fn fails_open_when_memory_is_down() {
    let mut f = fixture(staged(Vec::new()));
    f.pipeline.memory = Remembers::down();
    let result = f.pipeline.run(Mode::Live).await.unwrap();
    assert!(result.problems[0].starts_with("could not check the subject against memory"));
    assert_eq!(result.outcome, Outcome::Posted);
    assert!(
        result
            .problems
            .last()
            .unwrap()
            .starts_with("could not remember the post")
    );
}

struct Reader(Mutex<VecDeque<CritiqueVerdict>>, Mutex<Vec<CritiqueInput>>);

#[async_trait]
impl Critic for Reader {
    async fn critique(&self, input: &CritiqueInput) -> Result<Critique, CriticError> {
        self.1.lock().unwrap().push(input.clone());
        let verdict = self
            .0
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| CriticError("down".into()))?;
        Ok(Critique {
            verdict,
            score: 1.0,
            upside: 1.0,
            cost: 0.0,
            predicted: Predicted::default(),
            community_reply: 0.0,
            first_reply: "ok".into(),
            disagreement: String::new(),
            share_reason: String::new(),
            risk_reason: String::new(),
            fixes: vec!["lead with the joke".into()],
            summary: "flat".into(),
        })
    }
}

#[tokio::test]
async fn rewrites_the_script_and_the_caption_once_on_a_flat_read() {
    let mut f = fixture(staged(Vec::new()));
    let reader = Arc::new(Reader(
        Mutex::new(VecDeque::from([
            CritiqueVerdict::Flat,
            CritiqueVerdict::Worthy,
            CritiqueVerdict::Flat,
            CritiqueVerdict::Worthy,
        ])),
        Mutex::default(),
    ));
    f.pipeline.critic = Some(reader.clone());
    let result = f.pipeline.run(Mode::Live).await.unwrap();
    assert_eq!(f.text.asked_with("lead with the joke").len(), 2);
    assert_eq!(
        result.script_critique.unwrap().verdict,
        CritiqueVerdict::Worthy
    );
    assert_eq!(
        result.caption_critique.unwrap().verdict,
        CritiqueVerdict::Worthy
    );
    let seen = reader.1.lock().unwrap().clone();
    assert_eq!(seen[2].kind, CritiqueKind::Caption);
    assert!(seen[2].media.as_deref().unwrap().contains("raises the mug"));
}

#[tokio::test]
async fn a_critic_that_is_down_is_a_line_in_the_problems() {
    let mut f = fixture(staged(Vec::new()));
    f.pipeline.critic = Some(Arc::new(Reader(Mutex::default(), Mutex::default())));
    let result = f.pipeline.run(Mode::Live).await.unwrap();
    assert_eq!(
        result.problems,
        [
            "the critic could not read the script: down",
            "the critic could not read the caption: down"
        ]
    );
}

#[tokio::test]
async fn sends_a_caption_that_asks_the_reader_back_once_then_cuts_the_ask() {
    let f = fixture(staged(vec![(
        "caption",
        vec![
            r#"{"text": "gm builders. stars everywhere. what are you shipping today?"}"#,
            r#"{"text": "gm builders. stars everywhere. what about you?"}"#,
        ],
    )]));
    let result = f.pipeline.run(Mode::Live).await.unwrap();
    assert_eq!(
        f.text
            .asked_with("It ends by asking the reader something")
            .len(),
        1
    );
    assert!(!result.caption.contains('?'), "{}", result.caption);
}

#[test]
fn reads_a_loose_verdict_when_the_json_is_broken() {
    assert!(!loose_verdict(
        "{\"ok\": false, \"reason\": \"an \"unescaped\" quote"
    ));
    assert!(loose_verdict("Yes, that is Tiny."));
    assert!(!loose_verdict("I cannot tell."));
}
