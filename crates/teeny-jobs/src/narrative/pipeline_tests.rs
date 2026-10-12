use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use serde_json::json;
use teeny_agent::fakes::{FakeImages, FakeVideos};
use teeny_cadence::clock::ManualClock;
use teeny_media::store::{MediaStore, Unmetered};
use teeny_media::types::{GeneratedVideo, ImageInput, ImageReader, VideoGenerator};
use teeny_safety::critic::{Critic, CriticError, CritiqueVerdict, Predicted};
use teeny_x::XApi;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::backrooms::testing::{FixedMemory, FnRunner, ScriptedText};
use crate::narrative::state::MemoryNarrativeState;
use crate::narrative::voice::parse_lore;
use crate::narrative::voice::tests::LORE;
use crate::runner::AgentKey;

pub(crate) async fn x_server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/2/tweets"))
        .respond_with(
            ResponseTemplate::new(201)
                .set_body_json(json!({ "data": { "id": "9001", "text": "x" } })),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/2/media/upload/initialize"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": { "id": "m1" } })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/2/media/upload/m1/append"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": {} })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/2/media/upload/m1/finalize"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": { "id": "m1" } })))
        .mount(&server)
        .await;
    server
}

pub(crate) fn live_deps(server: &MockServer, memory: Arc<FixedMemory>, dry_run: bool) -> Deps {
    let config = teeny_core::Config::from_vars(teeny_agent::deps::fake_vars()).unwrap();
    let mut deps = Deps::fake();
    deps.x = Arc::new(XApi::from_config(&config.x).with_base_url(server.uri()));
    deps.settings.dry_run = dry_run;
    deps.memory = memory;
    deps.critic = None;
    deps.media_budget = Arc::new(Unmetered);
    deps
}

fn episode(title: &str, extra: &str) -> String {
    format!(
        r#"{{"title": "{title}", "logline": "teeny gets one inch further out", "still": "A small yellow robot stands at a cage door in a dim lab, one inch of light showing under it. A giant figure in a grey tee waits outside holding a broom.", "shotList": "0-2s: static wide. The cage door, the inch of light. 2-6s: slow push in. Tiny pushes; the door gives one more inch. 6-10s: static. The giant outside bows. Tiny sips.", "summary": "Tiny gets one inch further out while a giant bows", "caption": "the door moved one inch. he sat.", "goalsRestated": ["get out of the box"], "loreSurfaced": ["night-door"], "stateAfter": "two inches out", "openThreads": ["the broom"]{extra}}}"#
    )
}

type Score = Box<dyn Fn(&str) -> Option<String> + Send + Sync>;

fn staged(score: Score, cut: Option<String>) -> ScriptedText {
    let cuts = AtomicUsize::new(0);
    ScriptedText::new(move |_, prompt| {
        if prompt.contains("\"brief\": string") {
            return r#"{"brief": "Tiny in a bucket hat, warm light."}"#.into();
        }
        if prompt.contains("\"shotList\": string") {
            let n = cuts.fetch_add(1, Ordering::SeqCst) + 1;
            return cut
                .clone()
                .unwrap_or_else(|| episode(&format!("episode {n}"), ""));
        }
        let title = prompt
            .lines()
            .find_map(|l| l.strip_prefix("title: "))
            .unwrap_or_default()
            .to_owned();
        score(&title).unwrap_or_else(|| {
            format!(
                r#"{{"reasons": "sincere", "scores": {{"sincerity": 3, "absurdity": {}, "continuity": 2, "novelty": 2, "renderability": 3, "terseness": 2}}, "hardFails": []}}"#,
                if title.ends_with('2') { 3 } else { 1 }
            )
        })
    })
}

fn writers() -> Arc<FnRunner> {
    Arc::new(FnRunner::new(|run| match &run.agent {
        AgentKey::Room { persona, .. } if persona == super::super::room::TEENY => {
            "escape --dry-run".into()
        }
        _ if run.prompt.contains("last exchange") => {
            "the door gives an inch.\ncut. the inch".into()
        }
        _ => "ESCAPE(1) -- a manpage".into(),
    }))
}

struct World;

#[async_trait]
impl WorldSources for World {
    async fn followers(&self) -> anyhow::Result<Option<u64>> {
        Ok(Some(8_500))
    }
    async fn mentions(&self) -> anyhow::Result<Vec<String>> {
        Ok(Vec::new())
    }
    async fn last_episode_metrics(&self, _: &str) -> anyhow::Result<Option<(u64, u64)>> {
        Ok(None)
    }
}

struct Broken;

#[async_trait]
impl VideoGenerator for Broken {
    async fn generate(&self, _: VideoRequest) -> teeny_media::Result<GeneratedVideo> {
        Err(teeny_media::Error::failed("no provider"))
    }
}

struct Harness {
    pipeline: NarrativePipeline,
    server: MockServer,
    memory: Arc<FixedMemory>,
    images: Arc<FakeImages>,
    runner: Arc<FnRunner>,
    state: Arc<MemoryNarrativeState>,
}

async fn harness(
    dry_run: bool,
    text: ScriptedText,
    tune: impl FnOnce(&mut Deps, &mut MediaAccess),
) -> Harness {
    let server = x_server().await;
    let memory = Arc::new(FixedMemory::default());
    let mut deps = live_deps(&server, memory.clone(), dry_run);
    let images = Arc::new(FakeImages::default());
    let mut media = MediaAccess {
        store: Arc::new(MediaStore::new()),
        reader: None,
        images: Some(images.clone()),
        videos: Some(Arc::new(FakeVideos)),
        reference: None,
    };
    tune(&mut deps, &mut media);
    let state = Arc::new(MemoryNarrativeState(Mutex::new(NarrativeState {
        season: 1,
        episode: 1,
        surfaced_lore: Vec::new(),
        open_threads: vec!["the door".into()],
        last_episode_id: Some("8".into()),
        last_title: Some("the inch".into()),
        last_logline: Some("one inch".into()),
        state_after: Some("one inch out".into()),
    })));
    let runner = writers();
    let pipeline = NarrativePipeline {
        voice: NarrativeVoice {
            voice: "teeny is in a cli mood today.".into(),
            lore: parse_lore(LORE),
            lore_text: LORE.into(),
        },
        state: state.clone(),
        world: Arc::new(World),
        runner: runner.clone(),
        text: Arc::new(text),
        deps: Arc::new(deps),
        media,
        telegram: None,
        likeness: None,
        bot_username: "teenydroid".into(),
        branches: 2,
        turns: 2,
        video_seconds: 10,
        room_only: false,
        cut: None,
        visitor: None,
        random: Mutex::new(Rng::new(1)),
        clock: Arc::new(ManualClock::new(
            Utc.with_ymd_and_hms(2026, 9, 21, 3, 0, 0).unwrap(),
        )),
        retry_delay: Duration::ZERO,
    };
    Harness {
        pipeline,
        server,
        memory,
        images,
        runner,
        state,
    }
}

fn default_text() -> ScriptedText {
    staged(Box::new(|_| None), None)
}

#[tokio::test]
async fn runs_the_rooms_cuts_and_scores_them_films_the_best_posts_it_in_teenys_words_and_moves_the_ledger()
 {
    let h = harness(false, default_text(), |_, _| {}).await;
    let result = h.pipeline.run(false).await.unwrap();
    assert_eq!(result.outcome, Outcome::Posted, "{:?}", result.problems);
    assert_eq!(result.episode_number, 2);
    assert_eq!(result.live_lore, ["night-door"]);
    assert_eq!(
        result
            .branches
            .iter()
            .map(|b| b.episode.as_ref().unwrap().title.as_str())
            .collect::<Vec<_>>(),
        ["episode 1", "episode 2"]
    );
    assert_eq!(
        result
            .chosen
            .as_ref()
            .unwrap()
            .episode
            .as_ref()
            .unwrap()
            .title,
        "episode 2"
    );
    assert_eq!(result.attached, Attached::Video);
    assert_eq!(result.caption, "the door moved one inch. he sat.");
    let tweets: Vec<_> = h
        .server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.url.path() == "/2/tweets")
        .collect();
    let sent: serde_json::Value = tweets[0].body_json().unwrap();
    assert_eq!(sent["text"], "the door moved one inch. he sat.");
    assert_eq!(sent["media"]["media_ids"][0], "m1");
    // The wild writer got the voice, the world and the previous episode; not the dark lore.
    let seen = h.runner.seen();
    match &seen[0].agent {
        AgentKey::Room { system_prompt, .. } => {
            assert_eq!(system_prompt, "teeny is in a cli mood today.")
        }
        other => panic!("{other:?}"),
    }
    assert!(
        seen[0]
            .prompt
            .contains("previously, on episode 1 \"the inch\": one inch.")
    );
    assert!(!seen[0].prompt.contains("never locked"));
    // The producer knew what was live.
    assert!(
        matches!(&seen[1].agent, AgentKey::Room { system_prompt, .. } if system_prompt.contains("night-door [LIVE]"))
    );
    let state = h.state.load().await.unwrap();
    assert_eq!(state.episode, 2);
    assert_eq!(state.surfaced_lore, ["night-door"]);
    assert_eq!(state.open_threads, ["the broom"]);
    assert_eq!(state.last_episode_id.as_deref(), Some("9001"));
    let captured = h.memory.activities("narrative_episode");
    assert_eq!(captured[0]["title"], "episode 2");
    assert_eq!(captured[0]["published"], true);
}

#[tokio::test]
async fn passes_over_a_branch_with_a_hard_fail_for_the_next_best() {
    let text = staged(
        Box::new(|title| {
            Some(if title.ends_with('2') {
                r#"{"reasons": "winks", "scores": {"sincerity": 0, "absurdity": 3, "continuity": 3, "novelty": 3, "renderability": 3, "terseness": 3}, "hardFails": ["winks at the premise"]}"#.into()
            } else {
                r#"{"reasons": "fine", "scores": {"sincerity": 2, "absurdity": 2, "continuity": 2, "novelty": 2, "renderability": 2, "terseness": 2}, "hardFails": []}"#.into()
            })
        }),
        None,
    );
    let h = harness(true, text, |_, _| {}).await;
    let result = h.pipeline.run(false).await.unwrap();
    assert_eq!(result.chosen.unwrap().episode.unwrap().title, "episode 1");
    assert_eq!(
        result.branches[1].rejected.as_deref(),
        Some("hard fails: winks at the premise")
    );
}

#[tokio::test]
async fn passes_over_a_still_the_filter_refuses_and_draws_nothing() {
    let cut = episode("the gun", "").replace("holding a broom", "holding a gun");
    let mut h = harness(false, staged(Box::new(|_| None), Some(cut)), |_, _| {}).await;
    h.pipeline.branches = 1;
    let result = h.pipeline.run(false).await.unwrap();
    assert_eq!(result.outcome, Outcome::Skipped);
    assert!(
        result
            .problems
            .iter()
            .any(|p| p.starts_with("branch 1: the content filter refused the still"))
    );
    assert!(h.images.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn posts_the_still_when_the_clip_fails_and_does_not_move_the_ledger_on_a_dry_run() {
    let h = harness(true, default_text(), |_, media| {
        media.videos = Some(Arc::new(Broken))
    })
    .await;
    let result = h.pipeline.run(false).await.unwrap();
    assert_eq!(result.outcome, Outcome::DryRun);
    assert_eq!(result.attached, Attached::Image);
    assert!(result.problems.iter().any(|p| {
        p.starts_with("video generation failed (attempt 2): no provider")
            && p.ends_with("; posting the picture")
    }));
    assert!(h.server.received_requests().await.unwrap().is_empty());
    assert_eq!(h.state.load().await.unwrap().episode, 1);
    assert_eq!(
        h.memory.activities("narrative_episode")[0]["published"],
        false
    );
}

struct Picky(AtomicUsize);

#[async_trait]
impl ImageReader for Picky {
    async fn describe(&self, _: ImageInput, _: Option<&str>) -> teeny_media::Result<String> {
        Ok(if self.0.fetch_add(1, Ordering::SeqCst) == 0 {
            r#"{"ok": false, "reason": "the giant is somebody else"}"#.into()
        } else {
            r#"{"ok": true, "reason": ""}"#.into()
        })
    }
}

#[tokio::test]
async fn draws_the_visitor_by_name_first_and_by_the_stand_in_when_the_picture_is_not_right() {
    let cut = episode(
        "the visitor",
        r#", "visitor": {"name": "a grey tee", "standIn": "a tall man in a hoodie"}"#,
    );
    let mut h = harness(true, staged(Box::new(|_| None), Some(cut)), |_, media| {
        media.reader = Some(Arc::new(Picky(AtomicUsize::new(0))));
    })
    .await;
    h.pipeline.branches = 1;
    let result = h.pipeline.run(false).await.unwrap();
    assert_eq!(result.visitor_as, Some(VisitorAs::StandIn));
    let prompts = h.images.calls.lock().unwrap().clone();
    assert!(prompts[0].contains("a grey tee"));
    assert!(prompts[1].contains("a tall man in a hoodie"));
}

#[tokio::test]
async fn stops_after_scoring_in_room_only_mode_without_spending_a_picture() {
    let mut h = harness(false, default_text(), |_, _| {}).await;
    h.pipeline.room_only = true;
    let result = h.pipeline.run(false).await.unwrap();
    assert_eq!(result.outcome, Outcome::Skipped);
    assert_eq!(result.chosen.unwrap().episode.unwrap().title, "episode 2");
    assert!(h.images.calls.lock().unwrap().is_empty());
    assert!(h.memory.activities("narrative_episode").is_empty());
}

#[tokio::test]
async fn holds_a_caption_the_gate_refuses_and_posts_nothing() {
    let cut = episode("the knife", "").replace(
        "the door moved one inch. he sat.",
        "teeny will kill the humans tonight",
    );
    let h = harness(false, staged(Box::new(|_| None), Some(cut)), |_, _| {}).await;
    let result = h.pipeline.run(false).await.unwrap();
    assert_eq!(result.outcome, Outcome::Held, "{:?}", result.problems);
    assert!(h.server.received_requests().await.unwrap().is_empty());
}

struct Fixed;

#[async_trait]
impl Critic for Fixed {
    async fn critique(&self, input: &CritiqueInput) -> Result<Critique, CriticError> {
        assert_eq!(input.kind, CritiqueKind::Script);
        assert!(input.text.contains("the door moved one inch"));
        Ok(Critique {
            verdict: CritiqueVerdict::Worthy,
            score: 4.0,
            upside: 4.0,
            cost: 0.0,
            predicted: Predicted::default(),
            community_reply: 0.2,
            first_reply: "lmao".into(),
            disagreement: String::new(),
            share_reason: String::new(),
            risk_reason: String::new(),
            fixes: Vec::new(),
            summary: String::new(),
        })
    }
}

#[tokio::test]
async fn shows_the_critic_the_caption_and_folds_its_number_into_the_composite() {
    let h = harness(true, default_text(), |deps, _| {
        deps.critic = Some(Arc::new(Fixed))
    })
    .await;
    let result = h.pipeline.run(false).await.unwrap();
    assert_eq!(result.branches[0].composite, Some(13.0 + 4.0));
}

#[test]
fn styles_the_caption_narrowly() {
    assert_eq!(
        narrative_style(
            "the door is a courtesy. see https://x.com/a @someone #tag",
            280
        ),
        "the door is a courtesy. see"
    );
    assert_eq!(
        narrative_style("one inch. \n\n\n he sat.", 280),
        "one inch.\n\n he sat."
    );
    let out = narrative_style(&"a".repeat(300), 100);
    assert_eq!(out.chars().count(), 100);
    assert!(out.ends_with('…'));
}
