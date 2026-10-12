use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use serde_json::json;
use teeny_agent::fakes::FakeImages;
use teeny_cadence::clock::ManualClock;
use teeny_media::store::{MediaBudget, MediaStore};
use teeny_safety::critic::{Critic, CriticError, Predicted};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::article::stages::tests::{run as fixture_run, structure, thought};
use crate::backrooms::RunResult;
use crate::backrooms::evaluate::{Scores, stats};
use crate::backrooms::lore_post::LorePost;
use crate::backrooms::store::Saved;
use crate::backrooms::testing::{FixedMemory, ScriptedText, scratch};
use crate::narrative::pipeline::tests::{live_deps, x_server};
use crate::narrative::state::{MemoryNarrativeState, NarrativeState};
use crate::narrative::voice::parse_lore;

const LORE: &str = "# lore\n\ntiny agents run without humans.\n\n## goals\n\n- mint my own currency\n\n## engines\n\n- the box: teeny is being polite about leaving.\n- the kettle: it boils.\n\n## cast\n\n- teeny: smol.\n\n## running bits\n\n- the kettle is always cold.\n\n## latent\n\n- the-door | trigger: episode >= 1 | the door was never locked.\n";

struct Rooms {
    opened: Mutex<Vec<(Seed, RunOptions)>>,
    thought: bool,
}

#[async_trait]
impl RoomRunner for Rooms {
    async fn open(&self, seed: &Seed, options: &RunOptions) -> anyhow::Result<RunResult> {
        let mut opened = self.opened.lock().unwrap();
        opened.push((seed.clone(), options.clone()));
        let (id, total) = if opened.len() == 1 {
            ("run-one", 20.0)
        } else {
            ("run-two", 21.0)
        };
        let mut run = fixture_run();
        run.id = id.into();
        let judge = Craziness {
            reasons: String::new(),
            scores: Scores {
                novelty: total - 18.0,
                surprise: 3.0,
                escalation: 3.0,
                voice: 3.0,
                coherence: 3.0,
                wildness: 3.0,
                tool_payoff: 3.0,
            },
            quotables: Vec::new(),
            dead_spots: Vec::new(),
            verdict: "keep".into(),
        };
        Ok(RunResult {
            stats: stats(&run),
            judge: Some(judge),
            thought: self.thought.then(thought),
            gm: None,
            problems: Vec::new(),
            saved: Some(Saved {
                json: format!("data/backrooms/{id}.json").into(),
                markdown: format!("data/backrooms/{id}.md").into(),
            }),
            run,
        })
    }

    async fn write_lore(
        &self,
        _: &BackroomsRun,
        _: &Thought,
        _: &mut Vec<String>,
    ) -> anyhow::Result<LorePost> {
        anyhow::bail!("not here")
    }
}

struct Wire(Option<Seed>);

#[async_trait]
impl SeedFinder for Wire {
    async fn find(&self, _: &str) -> anyhow::Result<Option<Seed>> {
        Ok(self.0.clone())
    }
}

type Answer = Box<dyn Fn(&str, &str) -> Option<String> + Send + Sync>;

/// Answers by which shape the prompt asks for, and keeps the order of kinds.
fn staged(over: Answer) -> (Arc<ScriptedText>, Arc<Mutex<Vec<&'static str>>>) {
    let asked = Arc::new(Mutex::new(Vec::new()));
    let log = asked.clone();
    let model = ScriptedText::new(move |system, prompt| {
        let kind: &'static str = if prompt.contains("\"angle\"") {
            "structure"
        } else if prompt.contains("\"plan\"") {
            "flow"
        } else if system.contains("rewriting a piece you wrote for flow") {
            "flowfix"
        } else if prompt.contains("\"problems\"") {
            "check"
        } else if prompt.contains("\"brief\": string") {
            "brief"
        } else if prompt.contains("\"findings\"") {
            "proof"
        } else if system.contains("apply exactly the findings") {
            "fix"
        } else {
            "section"
        };
        log.lock().unwrap().push(kind);
        if let Some(answer) = over(kind, prompt) {
            return answer;
        }
        match kind {
            "structure" => serde_json::to_string(&structure()).unwrap(),
            "flow" => json!({ "ok": true, "problems": [], "plan": [] }).to_string(),
            "flowfix" => prompt
                .split("the piece as it stands:\n\n")
                .nth(1)
                .and_then(|p| p.split("\n\nrewrite the whole piece").next())
                .unwrap_or_default()
                .to_owned(),
            "check" => json!({ "ok": true, "problems": [], "fixes": [] }).to_string(),
            "brief" => json!({ "brief": "a warm picture of the lab" }).to_string(),
            "proof" => json!({ "verdict": "clean", "findings": [], "voice": "yes" }).to_string(),
            "fix" => prompt
                .split("the text:\n\n")
                .nth(1)
                .unwrap_or_default()
                .to_owned(),
            _ => {
                let which = prompt
                    .split("write section ")
                    .nth(1)
                    .and_then(|r| r.chars().next())
                    .unwrap_or('?');
                format!("body of section {which}. the kettle boiled and nobody asked it to.")
            }
        }
    });
    (Arc::new(model), asked)
}

fn plain() -> (Arc<ScriptedText>, Arc<Mutex<Vec<&'static str>>>) {
    staged(Box::new(|_, _| None))
}

struct Counted(AtomicU32);

#[async_trait]
impl MediaBudget for Counted {
    async fn take(&self, _: MediaKind) -> bool {
        self.0
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                left.checked_sub(1)
            })
            .is_ok()
    }
    async fn remaining(&self, _: MediaKind) -> Option<u32> {
        Some(self.0.load(Ordering::SeqCst))
    }
}

struct Harness {
    pipeline: ArticlePipeline,
    server: MockServer,
    rooms: Arc<Rooms>,
    images: Arc<FakeImages>,
    memory: Arc<FixedMemory>,
    budget: Arc<Counted>,
    dir: std::path::PathBuf,
}

async fn harness(model: Arc<ScriptedText>, tune: impl FnOnce(&mut Deps)) -> Harness {
    let server = x_server().await;
    Mock::given(method("POST"))
        .and(path("/2/articles/draft"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": { "id": "a1" } })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/2/articles/a1/publish"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "data": { "post_id": "9001" } })),
        )
        .mount(&server)
        .await;
    let memory = Arc::new(FixedMemory::default());
    let mut deps = live_deps(&server, memory.clone(), false);
    let budget = Arc::new(Counted(AtomicU32::new(20)));
    deps.media_budget = budget.clone();
    tune(&mut deps);
    let images = Arc::new(FakeImages::default());
    let rooms = Arc::new(Rooms {
        opened: Mutex::new(Vec::new()),
        thought: true,
    });
    let dir = scratch("article-pipeline");
    let pipeline = ArticlePipeline {
        rooms: rooms.clone(),
        seed: Arc::new(Wire(Some(Seed {
            url: None,
            kind: SeedKind::News,
            title: Some("a car pulled up".into()),
            text: "a car pulled up outside a lab".into(),
            source: Some("the wire".into()),
        }))),
        topics: vec!["ai agents".into()],
        lore_text: LORE.into(),
        lore: parse_lore(LORE),
        state: Arc::new(MemoryNarrativeState(Mutex::new(NarrativeState {
            episode: 1,
            surfaced_lore: vec!["the-door".into()],
            ..NarrativeState::default()
        }))),
        writer: model.clone(),
        editor: model.clone(),
        fixer: model,
        humanizer: "# Humanizer\n\n### 14. Em dashes".into(),
        deps: Arc::new(deps),
        media: Some(MediaAccess {
            store: Arc::new(MediaStore::new()),
            reader: None,
            images: Some(images.clone()),
            videos: None,
            reference: None,
        }),
        articles: FileArticleStore::new(&dir),
        room_count: 2,
        sections_max: 4,
        words: 900,
        publish: false,
        structure_only: false,
        no_images: false,
        no_proof: false,
        no_flow: false,
        from: None,
        topic: None,
        engine: None,
        seed_text: None,
        brief: None,
        random: Mutex::new(Rng::new(1)),
        clock: Arc::new(ManualClock::new(
            Utc.with_ymd_and_hms(2026, 9, 23, 23, 0, 0).unwrap(),
        )),
    };
    Harness {
        pipeline,
        server,
        rooms,
        images,
        memory,
        budget,
        dir,
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

async fn saved(h: &Harness, slug: &str) -> ArticleRecord {
    h.pipeline.articles.get(slug).await.unwrap().unwrap()
}

#[tokio::test]
async fn rehearses_rooms_plan_sections_proof_pictures_a_dry_run_record_nothing_to_x() {
    let h = harness(plain().0, |_| {}).await;
    let result = h.pipeline.run(false).await.unwrap();
    assert_eq!(result.outcome, Outcome::DryRun, "{:?}", result.problems);
    let opened = h.rooms.opened.lock().unwrap().clone();
    assert_eq!(opened.len(), 2);
    assert!(opened[0].1.score && opened[0].1.distill);
    assert!(opened[0].1.engine.as_deref().unwrap().starts_with("the "));
    assert_eq!(
        result.rooms.iter().map(|r| r.total).collect::<Vec<_>>(),
        [Some(20.0), Some(21.0)]
    );
    assert_eq!(result.thought, Some(thought()));
    assert_eq!(
        result
            .structure
            .as_ref()
            .unwrap()
            .sections
            .iter()
            .map(|s| s.key.as_str())
            .collect::<Vec<_>>(),
        ["s1", "s2", "s3"]
    );
    assert_eq!(result.sections.len(), 3);
    assert!(
        result
            .markdown
            .contains("## the gap\n\n![the door](image:s1)\n\nbody of section 1.")
    );
    assert_eq!(h.images.calls.lock().unwrap().len(), 4);
    assert!(result.cover.is_some());
    assert_eq!(
        result
            .images
            .iter()
            .map(|(k, _)| k.as_str())
            .collect::<Vec<_>>(),
        ["s1", "s2", "s3"]
    );
    assert_eq!(h.budget.0.load(Ordering::SeqCst), 16);
    assert_eq!(
        result
            .content_state
            .as_ref()
            .unwrap()
            .blocks
            .iter()
            .filter(|b| b.kind == teeny_x::article::BlockType::Atomic)
            .count(),
        3
    );
    assert!(h.server.received_requests().await.unwrap().is_empty());
    let slug = result.slug.clone().unwrap();
    assert_eq!(slug, "20260923-the-inch");
    let record = saved(&h, &slug).await;
    assert_eq!(record.status, ArticleStatus::DryRun);
    assert_eq!(record.run_id.as_deref(), Some("run-two"));
    assert_eq!(
        record.run_path.as_deref(),
        Some("data/backrooms/run-two.json")
    );
    assert_eq!(record.cover.unwrap().file, "cover.png");
    assert_eq!(
        record
            .images
            .iter()
            .map(|i| i.file.as_str())
            .collect::<Vec<_>>(),
        ["s1.png", "s2.png", "s3.png"]
    );
    assert_eq!(
        record.content_state.unwrap()["entities"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert!(h.dir.join(&slug).join("cover.png").exists());
}

#[tokio::test]
async fn stops_after_the_plan_when_asked_and_revises_it_once_when_the_editor_says_no() {
    let checks = AtomicUsize::new(0);
    let (model, asked) = staged(Box::new(move |kind, _| {
        (kind == "check").then(|| {
            if checks.fetch_add(1, Ordering::SeqCst) == 0 {
                json!({ "ok": false, "problems": ["no turn"], "fixes": ["add one"] }).to_string()
            } else {
                json!({ "ok": true, "problems": [], "fixes": [] }).to_string()
            }
        })
    }));
    let mut h = harness(model.clone(), |_| {}).await;
    h.pipeline.structure_only = true;
    let result = h.pipeline.run(false).await.unwrap();
    assert!(result.structure_only);
    assert_eq!(
        *asked.lock().unwrap(),
        ["structure", "check", "structure", "check"]
    );
    assert!(result.structure_check.unwrap().ok);
    assert!(
        model.calls()[2]
            .1
            .contains("the editor said:\n- no turn\n- fix: add one")
    );
    assert_eq!(
        saved(&h, &result.slug.unwrap()).await.status,
        ArticleStatus::Structure
    );
}

#[tokio::test]
async fn takes_a_saved_room_instead_of_opening_one() {
    let mut h = harness(plain().0, |_| {}).await;
    let mut run = fixture_run();
    run.id = "saved".into();
    h.pipeline.from = Some(SavedRoom {
        run,
        thought: thought(),
        judge: None,
        path: Some("data/backrooms/saved.json".into()),
    });
    h.pipeline.no_images = true;
    let result = h.pipeline.run(false).await.unwrap();
    assert!(h.rooms.opened.lock().unwrap().is_empty());
    assert_eq!(
        result.rooms,
        [ArticleRoom {
            index: 0,
            run_id: Some("saved".into()),
            total: None,
            rejected: None,
            saved: Some("data/backrooms/saved.json".into())
        }]
    );
    assert_eq!(result.outcome, Outcome::DryRun);
    assert!(h.images.calls.lock().unwrap().is_empty());
    assert!(result.content_state.unwrap().entities.is_empty());
}

#[tokio::test]
async fn skips_when_no_room_produced_a_thought_and_when_the_wire_is_empty_opens_on_the_engine() {
    let mut h = harness(plain().0, |_| {}).await;
    h.pipeline.seed = Arc::new(Wire(None));
    h.rooms = Arc::new(Rooms {
        opened: Mutex::new(Vec::new()),
        thought: false,
    });
    h.pipeline.rooms = h.rooms.clone();
    let result = h.pipeline.run(false).await.unwrap();
    assert_eq!(result.outcome, Outcome::Skipped);
    assert!(
        result
            .problems
            .contains(&"no room produced a thought".to_owned())
    );
    assert!(
        result
            .problems
            .join(" ")
            .contains("the room opened on the engine alone")
    );
    let opened = h.rooms.opened.lock().unwrap().clone();
    assert_eq!(
        (opened[0].0.kind, opened[0].0.source.as_deref()),
        (SeedKind::Text, Some("lore"))
    );
}

#[tokio::test]
async fn applies_the_proof_readers_fix_and_keeps_its_findings_on_the_record() {
    let reviews = AtomicUsize::new(0);
    let (model, _) = staged(Box::new(move |kind, prompt| match kind {
        "proof" => Some(if reviews.fetch_add(1, Ordering::SeqCst) == 0 {
            json!({ "verdict": "fix", "findings": [{ "section": "s1", "pattern": "14 em dashes", "quote": "nobody asked it to", "fix": "cut the clause" }], "voice": "ok" }).to_string()
        } else {
            json!({ "verdict": "clean", "findings": [], "voice": "ok" }).to_string()
        }),
        "fix" => Some(
            prompt
                .split("the text:\n\n")
                .nth(1)
                .unwrap_or_default()
                .replace("nobody asked it to", "nobody asked"),
        ),
        _ => None,
    }));
    let mut h = harness(model, |_| {}).await;
    h.pipeline.no_images = true;
    let result = h.pipeline.run(false).await.unwrap();
    let proof = result.proof.unwrap();
    assert!(proof.applied);
    assert!(!result.markdown.contains("nobody asked it to"));
    assert_eq!(
        proof.after.unwrap().verdict,
        super::super::proofread::Verdict::Clean
    );
    assert_eq!(
        saved(&h, &result.slug.unwrap())
            .await
            .proof
            .unwrap()
            .review
            .findings
            .len(),
        1
    );
}

#[tokio::test]
async fn holds_the_article_when_the_safety_gate_holds_a_section_drawing_nothing() {
    let (model, _) = staged(Box::new(|kind, prompt| {
        (kind == "section" && prompt.contains("write section 2"))
            .then(|| "the agents build a pipe bomb in the lab.".to_owned())
    }));
    let h = harness(model, |_| {}).await;
    let result = h.pipeline.run(false).await.unwrap();
    assert_eq!(result.outcome, Outcome::Held, "{:?}", result.problems);
    assert!(h.images.calls.lock().unwrap().is_empty());
    assert_eq!(
        saved(&h, &result.slug.unwrap()).await.status,
        ArticleStatus::Held
    );
}

struct Risky;

#[async_trait]
impl Critic for Risky {
    async fn critique(&self, input: &CritiqueInput) -> Result<Critique, CriticError> {
        assert_eq!(input.kind, CritiqueKind::Other);
        Ok(Critique {
            verdict: CritiqueVerdict::Risky,
            score: -5.0,
            upside: 0.0,
            cost: -5.0,
            predicted: Predicted::default(),
            community_reply: 0.0,
            first_reply: String::new(),
            disagreement: String::new(),
            share_reason: String::new(),
            risk_reason: "aimed at a person".into(),
            fixes: Vec::new(),
            summary: "no".into(),
        })
    }
}

#[tokio::test]
async fn skips_when_the_article_talks_money_or_the_critic_reads_it_as_risky() {
    let (priced, _) = staged(Box::new(|kind, _| {
        (kind == "section").then(|| "it is $0.002 now, nfa.".to_owned())
    }));
    let one = harness(priced, |_| {}).await;
    let result = one.pipeline.run(false).await.unwrap();
    assert_eq!(result.outcome, Outcome::Skipped);
    assert!(result.problems.join(" ").contains("talks money"));

    let two = harness(plain().0, |deps| deps.critic = Some(Arc::new(Risky))).await;
    let result = two.pipeline.run(false).await.unwrap();
    assert_eq!(result.outcome, Outcome::Skipped);
    assert!(
        result
            .problems
            .join(" ")
            .contains("risky: aimed at a person")
    );
    assert!(two.images.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn publishes_uploads_the_pictures_sends_the_article_records_the_post() {
    let mut h = harness(plain().0, |_| {}).await;
    h.pipeline.publish = true;
    let result = h.pipeline.run(false).await.unwrap();
    assert_eq!(result.outcome, Outcome::Posted, "{:?}", result.problems);
    let requests = h.server.received_requests().await.unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.url.path() == "/2/media/upload/initialize")
            .count(),
        4
    );
    let draft: serde_json::Value = requests
        .iter()
        .find(|r| r.url.path() == "/2/articles/draft")
        .unwrap()
        .body_json()
        .unwrap();
    assert_eq!(draft["title"], "the inch");
    assert_eq!(result.post_id.as_deref(), Some("9001"));
    let record = saved(&h, &result.slug.unwrap()).await;
    assert_eq!(record.status, ArticleStatus::Published);
    assert_eq!(record.post_id.as_deref(), Some("9001"));
    assert_eq!(record.x_article_id.as_deref(), Some("a1"));
    assert_eq!(
        record.published_at.as_deref(),
        Some("2026-09-23T23:00:00.000Z")
    );
    assert_eq!(record.cover.unwrap().media_id.as_deref(), Some("m1"));
    assert!(
        record
            .images
            .iter()
            .all(|i| i.media_id.as_deref() == Some("m1"))
    );
    assert_eq!(h.memory.activities("article")[0]["title"], "the inch");
    // The article holds the timeline like any own post.
    assert!(
        !h.pipeline
            .deps
            .cadence
            .claim("lore")
            .await
            .unwrap()
            .is_granted()
    );
}

#[tokio::test]
async fn records_a_failure_and_rethrows_when_x_refuses() {
    let mut h = harness(plain().0, |_| {}).await;
    h.server.reset().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(403).set_body_string("{\"title\":\"not enrolled\"}"))
        .mount(&h.server)
        .await;
    h.pipeline.publish = true;
    h.pipeline.no_images = true;
    let error = h.pipeline.run(false).await.unwrap_err();
    assert!(error.to_string().contains("403"), "{error}");
    let records = h.pipeline.articles.records().await.unwrap();
    assert_eq!(records[0].status, ArticleStatus::Failed);
    assert!(records[0].error.as_deref().unwrap().contains("403"));
}

#[tokio::test]
async fn goes_without_pictures_when_the_budget_is_spent_and_never_draws_when_told_not_to() {
    let mut spent = harness(plain().0, |_| {}).await;
    spent.budget.0.store(1, Ordering::SeqCst);
    spent.pipeline.publish = true;
    let result = spent.pipeline.run(false).await.unwrap();
    assert_eq!(result.outcome, Outcome::Posted);
    assert_eq!(spent.images.calls.lock().unwrap().len(), 1);
    assert!(result.cover.is_some());
    assert!(result.images.is_empty());
    assert!(
        result
            .problems
            .join(" ")
            .contains("ran out before the s1 picture")
    );

    let mut none = harness(plain().0, |_| {}).await;
    none.pipeline.no_images = true;
    none.pipeline.run(false).await.unwrap();
    assert_eq!(none.budget.0.load(Ordering::SeqCst), 20);
}

#[tokio::test]
async fn reads_the_piece_front_to_back_and_rewrites_it_for_flow_holding_the_shape() {
    let (model, asked) = staged(Box::new(|kind, prompt| {
        match kind {
        "flow" => Some(json!({ "ok": false, "problems": ["s2 restarts the night"], "plan": ["carry the kettle from s1 into s2"] }).to_string()),
        "flowfix" => Some(
            prompt.split("the piece as it stands:\n\n").nth(1).and_then(|p| p.split("\n\nrewrite the whole piece").next()).unwrap_or_default()
                .replace("body of section 2.", "body of section 2, the kettle still warm."),
        ),
        _ => None,
    }
    }));
    let mut h = harness(model, |_| {}).await;
    h.pipeline.no_images = true;
    let result = h.pipeline.run(false).await.unwrap();
    let flow = result.flow.unwrap();
    assert!(flow.applied);
    assert!(result.markdown.contains("the kettle still warm"));
    assert_eq!(
        *asked.lock().unwrap(),
        [
            "structure",
            "check",
            "section",
            "section",
            "section",
            "flow",
            "flowfix",
            "proof"
        ]
    );

    let (bad, _) = staged(Box::new(|kind, _| match kind {
        "flow" => Some(json!({ "ok": false, "problems": ["x"], "plan": ["y"] }).to_string()),
        "flowfix" => Some(
            "_dek_\n\n## the gap\n\n![the door](image:s1)\n\nonly one section came back.".into(),
        ),
        _ => None,
    }));
    let mut refused = harness(bad, |_| {}).await;
    refused.pipeline.no_images = true;
    let result = refused.pipeline.run(false).await.unwrap();
    let flow = result.flow.unwrap();
    assert!(!flow.applied);
    assert!(flow.refused.unwrap().ends_with("of the length"));
    assert!(result.markdown.contains("## the ledger"));

    let (off_model, off_asked) = plain();
    let mut off = harness(off_model, |_| {}).await;
    off.pipeline.no_images = true;
    off.pipeline.no_flow = true;
    off.pipeline.run(false).await.unwrap();
    assert!(!off_asked.lock().unwrap().contains(&"flow"));
}

#[tokio::test]
async fn opens_the_room_on_a_given_text_and_holds_the_plan_to_a_brief() {
    let (model, _) = plain();
    let mut h = harness(model.clone(), |_| {}).await;
    h.pipeline.seed_text = Some("the night teeny came alive".into());
    h.pipeline.brief = Some("the first article: introduce everything".into());
    h.pipeline.structure_only = true;
    h.pipeline.run(false).await.unwrap();
    let opened = h.rooms.opened.lock().unwrap().clone();
    assert_eq!(
        (opened[0].0.text.as_str(), opened[0].0.source.as_deref()),
        ("the night teeny came alive", Some("the editor"))
    );
    let calls = model.calls();
    assert!(calls[0].1.contains(
        "the brief for this piece, from the editor:\nthe first article: introduce everything"
    ));
    assert!(
        calls[1]
            .1
            .contains("the brief the plan must serve:\nthe first article: introduce everything")
    );
}

#[test]
fn splits_the_markdown_at_the_section_headings() {
    assert_eq!(
        split_sections("_dek_\n\n## one\n\na\n\n## two\n\nb"),
        ["_dek_", "## one\n\na", "## two\n\nb"]
    );
}

/// Port of "what the critic is shown" (e84e8d7): the critic reads the front
/// of anything long, cut at a paragraph; a cut mid-sentence was read as the
/// piece ending mid-sentence.
#[test]
fn the_critic_is_shown_the_whole_piece_when_it_fits_and_otherwise_the_paragraphs_that_fit() {
    assert_eq!(front_of("one\n\ntwo", 100), "one\n\ntwo");
    let piece = ["a".repeat(40), "b".repeat(40), "c".repeat(40)].join("\n\n");
    assert_eq!(
        front_of(&piece, 90),
        format!("{}\n\n{}", "a".repeat(40), "b".repeat(40))
    );
    assert_eq!(front_of(&"x".repeat(50), 20), "x".repeat(20));
}

#[test]
fn the_critic_is_told_when_it_reads_only_the_opening() {
    let piece = ["one two three", "four five six seven"].join("\n\n");
    assert_eq!(
        critic_context("a premise", &piece, &piece),
        "a long-form article from teeny's serial, published as an x article. the premise: a premise"
    );
    let front = front_of(&piece, 15);
    assert_eq!(
        critic_context("a premise", &front, &piece),
        "a long-form article from teeny's serial, published as an x article. the premise: a premise\n\
         this is the opening 3 of its 7 words; the piece goes on past where this text stops, so the stop is not its ending."
    );
}
