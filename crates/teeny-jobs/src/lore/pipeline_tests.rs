use std::sync::Arc;

use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use teeny_agent::deps::Deps;
use teeny_cadence::clock::ManualClock;
use teeny_x::XApi;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::backrooms::evaluate::stats;
use crate::backrooms::testing::{FixedMemory, run};
use crate::backrooms::{BackroomsRun, RunResult, SeedKind};

pub(crate) fn seed() -> Seed {
    Seed {
        kind: SeedKind::News,
        title: Some("AI agents now file taxes".into()),
        text: "A startup says its agents filed a return unsupervised.".into(),
        source: Some("the wire".into()),
    }
}

pub(crate) fn thought() -> Thought {
    Thought {
        reasons: String::new(),
        premise: "the tiny agents file the kettle as an asset and the ledger balances.".into(),
        wire: "agents filing taxes unsupervised".into(),
        objects: vec!["the kettle".into(), "the ledger".into()],
        thought: "the agents put the kettle on the ledger tonight. in crayon. it balanced.".into(),
        post: "the kettle is on the ledger now.".into(),
        kept: Vec::new(),
    }
}

/// A room that answers with a fixed thought and a fixed post, and keeps what it was asked.
pub(crate) struct FakeRooms {
    pub thought: Option<Thought>,
    pub post: String,
    pub opened: std::sync::Mutex<Vec<(Seed, Option<u32>)>>,
}

impl FakeRooms {
    pub fn new(post: &str) -> Self {
        Self {
            thought: Some(thought()),
            post: post.into(),
            opened: std::sync::Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl RoomRunner for FakeRooms {
    async fn open(&self, seed: &Seed, options: &RunOptions) -> anyhow::Result<RunResult> {
        self.opened
            .lock()
            .unwrap()
            .push((seed.clone(), options.turns));
        let run: BackroomsRun = run();
        Ok(RunResult {
            stats: stats(&run),
            run,
            judge: None,
            thought: self.thought.clone(),
            gm: None,
            problems: vec!["reddit was down".into()],
            saved: None,
        })
    }

    async fn write_lore(
        &self,
        _run: &BackroomsRun,
        _thought: &Thought,
        _problems: &mut Vec<String>,
    ) -> anyhow::Result<LorePost> {
        Ok(LorePost {
            reasons: "kept the kettle".into(),
            continues: String::new(),
            text: self.post.clone(),
        })
    }
}

pub(crate) struct Found(pub Option<Seed>, pub bool);

#[async_trait]
impl SeedFinder for Found {
    async fn find(&self, _topic: &str) -> anyhow::Result<Option<Seed>> {
        if self.1 {
            anyhow::bail!("4chan 503");
        }
        Ok(self.0.clone())
    }
}

pub(crate) async fn deps(live: bool) -> (MockServer, Arc<FixedMemory>, Arc<Deps>) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/2/tweets"))
        .respond_with(
            ResponseTemplate::new(201)
                .set_body_json(serde_json::json!({ "data": { "id": "own-1", "text": "x" } })),
        )
        .mount(&server)
        .await;
    let config = teeny_core::Config::from_vars(teeny_agent::deps::fake_vars()).unwrap();
    let memory = Arc::new(FixedMemory::default());
    let mut deps = Deps::fake();
    deps.x = Arc::new(XApi::from_config(&config.x).with_base_url(server.uri()));
    deps.settings.dry_run = !live;
    deps.memory = memory.clone();
    (server, memory, Arc::new(deps))
}

pub(crate) fn pipeline(
    rooms: Arc<FakeRooms>,
    seeds: Vec<(SeedSource, Arc<dyn SeedFinder>)>,
    deps: Arc<Deps>,
) -> LorePipeline {
    LorePipeline {
        rooms,
        seeds,
        topics: vec!["ai agents".into()],
        turns: Some(6),
        deps,
        kind: "lore".into(),
        random: Mutex::new(Rng::new(1)),
        clock: Arc::new(ManualClock::new(
            Utc.with_ymd_and_hms(2026, 9, 22, 15, 0, 0).unwrap(),
        )),
    }
}

fn news_then_forum() -> Vec<(SeedSource, Arc<dyn SeedFinder>)> {
    vec![
        (SeedSource::News, Arc::new(Found(None, false))),
        (SeedSource::Forum, Arc::new(Found(Some(seed()), false))),
    ]
}

#[tokio::test]
async fn tries_the_sources_in_order_opens_the_room_on_the_first_seed_writes_the_post_and_posts_it()
{
    let (server, memory, deps) = deps(true).await;
    let rooms = Arc::new(FakeRooms::new(
        "the kettle is on the ledger now. in crayon.",
    ));
    let result = pipeline(rooms.clone(), news_then_forum(), deps)
        .run(false)
        .await
        .unwrap();
    assert_eq!(result.outcome, Outcome::Posted);
    assert_eq!(result.from, Some((SeedSource::Forum, "ai agents".into())));
    assert_eq!(rooms.opened.lock().unwrap()[0], (seed(), Some(6)));
    assert_eq!(result.posted_id.as_deref(), Some("own-1"));
    assert_eq!(
        result.problems,
        [
            "news had nothing on \"ai agents\"",
            "the room: reddit was down"
        ]
    );
    let sent: serde_json::Value = server.received_requests().await.unwrap()[0]
        .body_json()
        .unwrap();
    assert_eq!(sent["text"], "the kettle is on the ledger now. in crayon.");
    let captured = memory.activities("lore_post");
    assert_eq!(captured[0]["premise"], thought().premise);
    assert_eq!(captured[0]["published"], true);
}

#[tokio::test]
async fn stays_quiet_when_nothing_comes_through_the_wire_and_when_the_room_leaves_no_thought() {
    let (server, _memory, deps) = deps(true).await;
    let rooms = Arc::new(FakeRooms::new("x"));
    let dry: Vec<(SeedSource, Arc<dyn SeedFinder>)> = vec![
        (SeedSource::News, Arc::new(Found(None, false))),
        (SeedSource::Forum, Arc::new(Found(None, true))),
    ];
    let quiet = pipeline(rooms.clone(), dry, deps.clone())
        .run(false)
        .await
        .unwrap();
    assert_eq!(quiet.outcome, Outcome::Skipped);
    assert!(
        quiet
            .problems
            .contains(&"forum could not be read: 4chan 503".to_owned())
    );
    assert!(rooms.opened.lock().unwrap().is_empty());

    let mut empty = FakeRooms::new("x");
    empty.thought = None;
    let none = pipeline(Arc::new(empty), news_then_forum(), deps)
        .run(false)
        .await
        .unwrap();
    assert_eq!(none.outcome, Outcome::Skipped);
    assert!(
        none.problems
            .contains(&"the room ran but left no thought".to_owned())
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn dry_runs_without_posting_and_still_remembers_labelled_unpublished() {
    let (server, memory, deps) = deps(false).await;
    let rooms = Arc::new(FakeRooms::new("the kettle is on the ledger now."));
    let result = pipeline(rooms, news_then_forum(), deps)
        .run(false)
        .await
        .unwrap();
    assert_eq!(result.outcome, Outcome::DryRun);
    assert_eq!(result.text, "the kettle is on the ledger now.");
    assert!(server.received_requests().await.unwrap().is_empty());
    assert_eq!(memory.activities("lore_post")[0]["published"], false);
}

#[tokio::test]
async fn a_rehearsal_posts_nothing_even_live() {
    let (server, _memory, deps) = deps(true).await;
    let rooms = Arc::new(FakeRooms::new("the kettle is on the ledger now."));
    let result = pipeline(rooms, news_then_forum(), deps)
        .run(true)
        .await
        .unwrap();
    assert_eq!(result.outcome, Outcome::DryRun);
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn holds_a_post_the_filter_refuses() {
    let (server, _memory, deps) = deps(true).await;
    let rooms = Arc::new(FakeRooms::new("the agents will kill the humans tonight"));
    let result = pipeline(rooms, news_then_forum(), deps)
        .run(false)
        .await
        .unwrap();
    assert_eq!(result.outcome, Outcome::Held);
    assert_eq!(result.text, "");
    assert!(server.received_requests().await.unwrap().is_empty());
}
