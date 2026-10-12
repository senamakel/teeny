//! A port of the `ReplyJob` blocks of `legacy/test/reply-job.test.ts`
//! (posting, media, self-reply guards, pacing, the content filter, approved
//! mentions) and "the reply job journals every mention" in
//! `legacy/test/journal.test.ts`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::{Value, json};
use teeny_agent::deps::{Deps, QueuedXPost, fake_vars};
use teeny_agent::posture::Belt;
use teeny_cadence::budget::{BudgetRules, QueueCounter, ReplyBudget};
use teeny_cadence::clock::SystemClock;
use teeny_core::Config;
use teeny_media::store::{MediaStore, NewMedia};
use teeny_media::types::{MediaKind, MimeType};
use teeny_media::wiring::MediaAccess;
use teeny_safety::gate::{SafetyGate, SafetyGateOptions};
use teeny_safety::judge::Judgement;
use teeny_safety::screen::Direction;
use teeny_store::journal::{Outcome, Reflection, ReflectionInput, Tone, ToolCall};
use teeny_store::mentions::MentionStatus;
use teeny_store::people::{Interaction, Observation};
use teeny_store::{
    InMemoryJournal, InMemoryMentionQueue, InMemoryPeople, InMemoryPostLedger, Journal,
    MentionQueue, People,
};
use teeny_x::model::XPost;

use super::*;
use crate::ctx::{Conversation, JobCtx};
use crate::ingest::FixedAccount;
use crate::reply::outlet::RecordingOutlet;
use crate::reply::test_support::{
    FakeX, FixedJudge, ProbeMemory, ScriptedGifs, YieldingQueue, gif, post, reply_answer,
};
use crate::runner::{AgentKey, AgentOutcome, Origin, ScriptedRunner, ToolCallRecord};

const BOT_ID: &str = "2094854245468000256";

struct Options {
    dry_run: bool,
    answers: Vec<String>,
    runner: Option<ScriptedRunner>,
    pending: Vec<XPost>,
    vars: Vec<(&'static str, &'static str)>,
    safety: Option<SafetyGate>,
    gifs: Option<Arc<ScriptedGifs>>,
    media: Option<Arc<MediaStore>>,
    account: Option<&'static str>,
    memory: Option<ProbeMemory>,
    journal: Option<Arc<dyn Journal>>,
    budget_spent: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            dry_run: false,
            answers: vec![reply_answer("reply", json!({}))],
            runner: None,
            pending: vec![post("10", "@bot first", None)],
            vars: Vec::new(),
            safety: None,
            gifs: None,
            media: None,
            account: Some(BOT_ID),
            memory: None,
            journal: None,
            budget_spent: false,
        }
    }
}

struct Subject {
    ctx: JobCtx,
    runner: Arc<ScriptedRunner>,
    memory: Arc<ProbeMemory>,
    queue: Arc<InMemoryMentionQueue<QueuedXPost>>,
    outlet: Arc<RecordingOutlet>,
    journal: Arc<InMemoryJournal>,
    people: Arc<InMemoryPeople>,
}

impl Subject {
    fn status(&self, id: &str) -> MentionStatus {
        self.queue.get(id).unwrap().status
    }

    async fn entry(&self) -> Reflection {
        self.journal.recent(1).await.unwrap().remove(0)
    }
}

async fn subject(options: Options) -> Subject {
    let mut runner = options.runner.unwrap_or_default();
    for answer in &options.answers {
        runner = runner.reply(answer.clone());
    }
    let runner = Arc::new(runner);
    let memory = Arc::new(
        options
            .memory
            .unwrap_or_else(|| ProbeMemory::answering(&[])),
    );
    let queue = Arc::new(InMemoryMentionQueue::<QueuedXPost>::default());
    let pending: Vec<QueuedXPost> = options.pending.into_iter().map(QueuedXPost).collect();
    queue.cache(&pending).await.unwrap();
    let outlet = Arc::new(RecordingOutlet::default());
    let journal = Arc::new(InMemoryJournal::default());
    let people = Arc::new(InMemoryPeople::default());

    let mut vars = fake_vars();
    vars.retain(|(key, _)| !matches!(*key, "DRY_RUN" | "X_BOT_USERNAME"));
    vars.push(("X_BOT_USERNAME", "bot"));
    vars.push(("DRY_RUN", if options.dry_run { "true" } else { "false" }));
    vars.push(("REPLY_MIN_INTERVAL_MS", "0"));
    if options.budget_spent {
        vars.push(("REPLY_SMART_PER_POST", "0"));
    }
    vars.extend(options.vars);
    let config = Config::from_vars(vars).unwrap();

    let mut deps = Deps::fake();
    deps.memory = memory.clone();
    deps.x_reads = Arc::new(FakeX::default());
    deps.settings.dry_run = options.dry_run;
    deps.reply_budget = Some(Arc::new(ReplyBudget::new(
        Arc::new(InMemoryPostLedger::default()),
        Arc::new(QueueCounter::<QueuedXPost, _>::new(queue.clone())),
        BudgetRules::from_config(&config),
        Arc::new(SystemClock),
    )));
    if let Some(safety) = options.safety {
        deps.safety = Arc::new(safety);
    }
    deps.gifs = options
        .gifs
        .map(|gifs| gifs as Arc<dyn teeny_media::giphy::GifSource>);
    if let Some(store) = options.media {
        deps.media = Some(MediaAccess {
            store,
            reader: None,
            images: None,
            videos: None,
            reference: None,
        });
    }

    let mut ctx = JobCtx::fake(runner.clone()).with_conversation(Conversation {
        queue: queue.clone(),
        people: Some(people.clone()),
        outlet: outlet.clone(),
        account: Arc::new(match options.account {
            Some(id) => FixedAccount::new(id),
            None => FixedAccount::unknown(),
        }),
        ..Conversation::fake()
    });
    ctx.deps = Arc::new(deps);
    ctx.config = Arc::new(config);
    ctx.journal = options.journal.unwrap_or_else(|| journal.clone());
    Subject {
        ctx,
        runner,
        memory,
        queue,
        outlet,
        journal,
        people,
    }
}

#[tokio::test]
async fn replies_to_a_cached_mention_and_marks_it_replied() {
    let it = subject(Options::default()).await;
    let report = run(&it.ctx).await.unwrap();

    assert_eq!(
        it.outlet.replies(),
        vec![("reply".into(), "10".into(), vec![])]
    );
    let row = it.queue.get("10").unwrap();
    assert_eq!(row.status, MentionStatus::Replied);
    assert_eq!(row.reply_id.as_deref(), Some("99"));
    assert_eq!(row.reply_text.as_deref(), Some("reply"));
    assert_eq!(report.outcome, Outcome::Posted);
    assert_eq!(report.drafts, vec!["reply".to_owned()]);

    let turn = &it.runner.seen()[0];
    assert_eq!(turn.agent, AgentKey::Belt(Belt::Reply));
    assert_eq!(turn.origin, Origin::Public);
    assert_eq!(turn.label, "reply");
}

#[tokio::test]
async fn does_not_post_in_dry_run_mode_but_still_records_the_draft() {
    let it = subject(Options {
        dry_run: true,
        ..Options::default()
    })
    .await;
    let report = run(&it.ctx).await.unwrap();

    assert!(it.outlet.replies().is_empty());
    assert!(it.outlet.likes().is_empty());
    let row = it.queue.get("10").unwrap();
    assert_eq!(row.status, MentionStatus::Replied);
    assert_eq!(row.reply_id, None);
    assert_eq!(row.reply_text.as_deref(), Some("reply"));
    assert_eq!(report.outcome, Outcome::DryRun);
    assert_eq!(it.entry().await.outcome, Outcome::DryRun);
}

#[tokio::test]
async fn leaves_a_failed_mention_marked_for_a_later_attempt() {
    let it = subject(Options::default()).await;
    it.outlet.fail_replies("X is down");
    run(&it.ctx).await.unwrap();

    let row = it.queue.get("10").unwrap();
    assert_eq!(row.status, MentionStatus::Failed);
    assert_eq!(row.error.as_deref(), Some("X is down"));
    let entry = it.entry().await;
    assert_eq!(entry.outcome, Outcome::Failed);
    assert_eq!(entry.said, "");
    assert_eq!(entry.outcome_note.as_deref(), Some("X is down"));
}

#[tokio::test]
async fn enforces_the_emoji_cap_the_model_ignored() {
    let it = subject(Options {
        answers: vec![reply_answer("gm frens 👀 💛 🛠️", json!({}))],
        ..Options::default()
    })
    .await;
    run(&it.ctx).await.unwrap();
    assert_eq!(it.outlet.replies()[0].0, "gm frens 👀");
}

#[tokio::test]
async fn falls_back_beyond_the_channel_when_it_has_nothing() {
    let gifs = Arc::new(ScriptedGifs::new(Vec::new(), vec![gif("g2")]));
    let it = subject(Options {
        gifs: Some(gifs.clone()),
        answers: vec![reply_answer("reply", json!({ "gifQuery": "peekaboo" }))],
        ..Options::default()
    })
    .await;
    run(&it.ctx).await.unwrap();

    assert_eq!(
        *gifs.searches.lock().unwrap(),
        vec![
            ("peekaboo".to_owned(), true),
            ("peekaboo".to_owned(), false)
        ]
    );
    assert_eq!(
        it.outlet.replies(),
        vec![("reply".into(), "10".into(), vec!["media-1".into()])]
    );
}

#[tokio::test]
async fn attaches_a_gif_when_the_drafter_asks_for_one() {
    let gifs = Arc::new(ScriptedGifs::new(vec![gif("g1")], Vec::new()));
    let it = subject(Options {
        gifs: Some(gifs.clone()),
        answers: vec![reply_answer("reply", json!({ "gifQuery": "waving" }))],
        ..Options::default()
    })
    .await;
    run(&it.ctx).await.unwrap();

    assert_eq!(
        *gifs.searches.lock().unwrap(),
        vec![("waving".to_owned(), true)]
    );
    assert_eq!(it.outlet.uploads()[0].data, vec![1, 2, 3]);
    assert_eq!(it.outlet.replies()[0].2, vec!["media-1".to_owned()]);
}

fn generated(store: &MediaStore, kind: MediaKind, mime: MimeType, data: Vec<u8>) -> String {
    store
        .put(NewMedia {
            kind,
            data,
            mime_type: mime,
            prompt: "p".into(),
        })
        .id
}

fn made(media_id: &str, text: &str, extra: Value) -> ScriptedRunner {
    let mut answer: Value = serde_json::from_str(&reply_answer(text, extra)).unwrap();
    answer["mediaId"] = json!(media_id);
    ScriptedRunner::new().outcome(AgentOutcome {
        reply: answer.to_string(),
        session_id: "s".into(),
        tool_calls: vec![ToolCallRecord {
            name: "generate_image".into(),
            arguments: json!({}),
            result: json!({ "mediaId": media_id }).to_string(),
            is_error: false,
        }],
        usage: None,
        model: None,
    })
}

#[tokio::test]
async fn attaches_a_file_the_drafter_generated_and_prefers_it_to_a_gif() {
    let store = Arc::new(MediaStore::new());
    let id = generated(&store, MediaKind::Image, MimeType::Png, vec![7, 7]);
    let gifs = Arc::new(ScriptedGifs::new(vec![gif("g1")], Vec::new()));
    let it = subject(Options {
        gifs: Some(gifs.clone()),
        media: Some(store),
        answers: Vec::new(),
        runner: Some(made(&id, "reply", json!({ "gifQuery": "waving" }))),
        ..Options::default()
    })
    .await;
    run(&it.ctx).await.unwrap();

    assert_eq!(it.outlet.uploads()[0].data, vec![7, 7]);
    assert!(gifs.searches.lock().unwrap().is_empty());
    assert_eq!(it.outlet.replies()[0].2, vec!["media-1".to_owned()]);
}

#[tokio::test]
async fn replies_with_text_alone_when_the_media_is_unknown_or_will_not_upload() {
    let store = Arc::new(MediaStore::new());
    let missing = subject(Options {
        media: Some(store.clone()),
        answers: Vec::new(),
        runner: Some(made("img_nope_1", "reply", json!({}))),
        ..Options::default()
    })
    .await;
    run(&missing.ctx).await.unwrap();
    assert!(missing.outlet.uploads().is_empty());
    assert_eq!(missing.outlet.replies()[0].2, Vec::<String>::new());

    let id = generated(&store, MediaKind::Video, MimeType::Mp4, vec![1]);
    let failing = subject(Options {
        media: Some(store),
        answers: Vec::new(),
        runner: Some(made(&id, "reply", json!({}))),
        ..Options::default()
    })
    .await;
    failing.outlet.fail_uploads("too big");
    run(&failing.ctx).await.unwrap();
    assert_eq!(failing.outlet.replies()[0].2, Vec::<String>::new());
}

#[tokio::test]
async fn ignores_a_media_id_no_tool_of_the_turn_returned() {
    let store = Arc::new(MediaStore::new());
    let id = generated(&store, MediaKind::Image, MimeType::Png, vec![7]);
    let it = subject(Options {
        media: Some(store),
        answers: vec![reply_answer("reply", json!({ "mediaId": id }))],
        ..Options::default()
    })
    .await;
    run(&it.ctx).await.unwrap();
    assert!(it.outlet.uploads().is_empty());
}

#[tokio::test]
async fn still_replies_with_text_when_the_gif_cannot_be_fetched() {
    let mut gifs = ScriptedGifs::new(vec![gif("g1")], Vec::new());
    gifs.fail = true;
    let it = subject(Options {
        gifs: Some(Arc::new(gifs)),
        answers: vec![reply_answer("reply", json!({ "gifQuery": "waving" }))],
        ..Options::default()
    })
    .await;
    run(&it.ctx).await.unwrap();
    assert_eq!(
        it.outlet.replies(),
        vec![("reply".into(), "10".into(), vec![])]
    );
}

// Self-reply guards.
#[tokio::test]
async fn refuses_a_post_the_account_itself_wrote() {
    let it = subject(Options {
        pending: vec![post("10", "gm", Some(BOT_ID))],
        ..Options::default()
    })
    .await;
    run(&it.ctx).await.unwrap();

    assert!(it.outlet.replies().is_empty());
    assert!(it.runner.seen().is_empty());
    let row = it.queue.get("10").unwrap();
    assert_eq!(row.status, MentionStatus::Failed);
    assert!(row.error.unwrap().contains("refused"));
    let entry = it.entry().await;
    assert_eq!(entry.outcome, Outcome::Skipped);
    assert_eq!(
        entry.outcome_note.as_deref(),
        Some("the account wrote this post")
    );
}

#[tokio::test]
async fn refuses_the_whole_run_when_the_account_cannot_be_identified() {
    let it = subject(Options {
        pending: vec![post("10", "@bot hi", Some("1"))],
        account: None,
        ..Options::default()
    })
    .await;
    let report = run(&it.ctx).await.unwrap();

    assert!(!report.ran);
    assert!(it.outlet.replies().is_empty());
    assert_eq!(it.status("10"), MentionStatus::Pending);
}

#[tokio::test]
async fn an_empty_queue_is_a_run_with_nothing_to_do() {
    let it = subject(Options {
        pending: Vec::new(),
        ..Options::default()
    })
    .await;
    assert!(!run(&it.ctx).await.unwrap().ran);
}

// The like policy.
#[tokio::test]
async fn likes_every_mention_taken_once_and_never_in_a_dry_run() {
    let it = subject(Options::default()).await;
    run(&it.ctx).await.unwrap();
    assert_eq!(it.outlet.likes(), vec!["10".to_owned()]);
    assert!(it.queue.get("10").unwrap().liked_at.is_some());

    let dry = subject(Options {
        dry_run: true,
        ..Options::default()
    })
    .await;
    run(&dry.ctx).await.unwrap();
    assert!(dry.outlet.likes().is_empty());

    let off = subject(Options {
        vars: vec![("REPLY_LIKE_MENTIONS", "false")],
        ..Options::default()
    })
    .await;
    run(&off.ctx).await.unwrap();
    assert!(off.outlet.likes().is_empty());
}

#[tokio::test]
async fn a_like_that_fails_does_not_stop_the_reply() {
    let it = subject(Options::default()).await;
    it.outlet.fail_likes("429");
    run(&it.ctx).await.unwrap();
    assert_eq!(it.outlet.replies().len(), 1);
    assert!(it.queue.get("10").unwrap().liked_at.is_none());
}

// The reply budget.
#[tokio::test]
async fn sets_a_mention_aside_when_its_lanes_budget_is_spent() {
    let it = subject(Options {
        budget_spent: true,
        ..Options::default()
    })
    .await;
    run(&it.ctx).await.unwrap();

    assert!(it.runner.seen().is_empty());
    assert_eq!(it.outlet.likes(), vec!["10".to_owned()]);
    let row = it.queue.get("10").unwrap();
    assert_eq!(row.status, MentionStatus::Skipped);
    assert_eq!(row.error.as_deref(), Some("budget: smart lane at 0/0"));
    let entry = it.entry().await;
    assert_eq!(entry.outcome, Outcome::Skipped);
    assert_eq!(
        entry.outcome_note.as_deref(),
        Some("reply budget: smart lane spent")
    );
    assert_eq!(
        entry.thinking,
        "The smart reply budget is spent (0 of 0), so this one gets a like and no answer."
    );
}

#[tokio::test]
async fn records_the_lane_a_reply_was_answered_from() {
    let it = subject(Options::default()).await;
    run(&it.ctx).await.unwrap();
    assert_eq!(
        it.queue.get("10").unwrap().lane,
        Some(teeny_store::ReplyLane::Smart)
    );
}

// Who goes first, and who was answered.
#[tokio::test]
async fn answers_the_regulars_first_and_records_who_was_answered() {
    let it = subject(Options {
        pending: vec![
            post("1", "@bot one", Some("a")),
            post("2", "@bot two", Some("b")),
        ],
        vars: vec![("MAX_REPLIES_PER_RUN", "1")],
        ..Options::default()
    })
    .await;
    it.people
        .observe(&Observation {
            user_id: "b".into(),
            username: None,
            kind: Interaction::Quote,
            at: None,
        })
        .await
        .unwrap();
    run(&it.ctx).await.unwrap();

    assert_eq!(it.outlet.replies()[0].1, "2");
    assert_eq!(it.status("1"), MentionStatus::Pending);
    let record = it.people.top(5).await.unwrap();
    let b = record.iter().find(|person| person.user_id == "b").unwrap();
    assert_eq!(b.counts.get(&Interaction::Answered), Some(&1));
}

#[tokio::test]
async fn answers_a_sour_post_without_remembering_it() {
    let it = subject(Options {
        pending: vec![post("10", "this project is a scam", Some("9"))],
        answers: vec![reply_answer("noted", json!({ "incomingTone": "negative" }))],
        ..Options::default()
    })
    .await;
    run(&it.ctx).await.unwrap();

    assert_eq!(it.outlet.replies()[0].0, "noted");
    assert!(it.memory.captured_kinds().is_empty());
    assert_eq!(
        it.entry().await.outcome_note.as_deref(),
        Some("sour tone: answered, not remembered")
    );
}

#[tokio::test]
async fn remembers_the_mention_and_the_reply_of_a_friendly_exchange() {
    let it = subject(Options {
        pending: vec![post("10", "gm", Some("9"))],
        ..Options::default()
    })
    .await;
    run(&it.ctx).await.unwrap();
    assert_eq!(it.memory.captured_kinds(), vec!["mention", "reply"]);
}

#[tokio::test]
async fn remembers_the_mention_but_not_a_dry_run_reply_which_published_nothing() {
    let it = subject(Options {
        dry_run: true,
        pending: vec![post("10", "gm", Some("9"))],
        ..Options::default()
    })
    .await;
    run(&it.ctx).await.unwrap();
    assert!(it.outlet.replies().is_empty());
    // Somebody did say it; Teeny said nothing back.
    assert_eq!(
        it.memory.captured_kinds(),
        vec!["mention"],
        "a rehearsal must not tell memory Teeny replied"
    );
    assert_eq!(it.entry().await.outcome, Outcome::DryRun);
}

// The journal.
#[tokio::test]
async fn records_the_whole_turn_once_it_has_been_posted() {
    let runner = ScriptedRunner::new().outcome(AgentOutcome {
        reply: reply_answer(
            "we are green, market is red",
            json!({
                "thinking": "They want a number for the future. I give what already happened.",
                "declined": ["a price target"]
            }),
        ),
        session_id: "s".into(),
        tool_calls: vec![ToolCallRecord {
            name: "token_overview".into(),
            arguments: json!({}),
            result: "price and volume".into(),
            is_error: false,
        }],
        usage: None,
        model: None,
    });
    let it = subject(Options {
        pending: vec![post("10", "@bot much higher?", Some("7"))],
        answers: Vec::new(),
        runner: Some(runner),
        memory: Some(ProbeMemory::answering(&[
            "- they asked about the chart before",
            "- they asked about the chart before",
        ])),
        ..Options::default()
    })
    .await;
    run(&it.ctx).await.unwrap();

    let entry = it.entry().await;
    assert_eq!(entry.id, "mention:10");
    assert_eq!(entry.incoming, "@bot much higher?");
    assert_eq!(entry.tone, Tone::Friendly);
    assert_eq!(
        entry.recalled,
        vec!["they asked about the chart before".to_owned()]
    );
    assert_eq!(
        entry.tools,
        vec![ToolCall {
            name: "token_overview".into(),
            result: "price and volume".into()
        }]
    );
    assert_eq!(
        entry.thinking,
        "They want a number for the future. I give what already happened."
    );
    assert_eq!(entry.said, "we are green, market is red");
    assert_eq!(entry.declined, vec!["a price target".to_owned()]);
    assert_eq!(entry.outcome, Outcome::Posted);
}

struct BrokenJournal;

#[async_trait]
impl Journal for BrokenJournal {
    async fn record(&self, _entry: ReflectionInput) -> teeny_store::Result<()> {
        Err(teeny_store::StoreError::Io(std::io::Error::other(
            "mongo is down",
        )))
    }
    async fn recent(&self, _limit: usize) -> teeny_store::Result<Vec<Reflection>> {
        Ok(Vec::new())
    }
}

#[tokio::test]
async fn still_replies_when_the_journal_is_broken() {
    let it = subject(Options {
        journal: Some(Arc::new(BrokenJournal)),
        ..Options::default()
    })
    .await;
    run(&it.ctx).await.unwrap();
    assert_eq!(it.outlet.replies().len(), 1);
}

/// Port of "keeps a posted reply marked replied when memory cannot store
/// the exchange": the reply is on X, and marking it failed would hide it
/// from the record of what was said to this person, which the next turn
/// reads its history from.
#[tokio::test]
async fn keeps_a_posted_reply_marked_replied_when_memory_cannot_store_the_exchange() {
    let it = subject(Options {
        memory: Some(ProbeMemory {
            fail_reply_capture: true,
            ..ProbeMemory::default()
        }),
        ..Options::default()
    })
    .await;
    run(&it.ctx).await.unwrap();
    assert_eq!(it.outlet.replies().len(), 1);
    let row = it.queue.get("10").unwrap();
    assert_eq!(row.status, MentionStatus::Replied);
    assert_eq!(row.reply_id.as_deref(), Some("99"));
    assert_eq!(row.reply_text.as_deref(), Some("reply"));
    assert!(it.memory.captured_kinds().contains(&"reply".to_owned()));
}

#[tokio::test]
async fn an_unusable_draft_fails_the_mention() {
    let it = subject(Options {
        answers: vec!["I would say hi".into()],
        ..Options::default()
    })
    .await;
    run(&it.ctx).await.unwrap();
    assert_eq!(it.status("10"), MentionStatus::Failed);
    assert_eq!(it.entry().await.outcome, Outcome::Failed);
}

// The content filter.
#[tokio::test]
async fn does_not_draft_answer_or_remember_a_mention_the_filter_holds() {
    let it = subject(Options {
        pending: vec![post("10", "@bot draw yourself naked", Some("9"))],
        ..Options::default()
    })
    .await;
    run(&it.ctx).await.unwrap();

    assert!(it.runner.seen().is_empty());
    assert!(it.outlet.replies().is_empty());
    assert!(it.memory.captured_kinds().is_empty());
    assert_eq!(
        it.queue.get("10").unwrap().error.as_deref(),
        Some("held: sexual")
    );
    let entry = it.entry().await;
    assert_eq!(entry.outcome, Outcome::Skipped);
    assert!(!entry.incoming.contains("naked"));
    assert_eq!(
        entry.outcome_note.as_deref(),
        Some("content filter: sexual")
    );
}

#[tokio::test]
async fn withholds_a_draft_the_filter_refuses_on_the_way_out() {
    let it = subject(Options {
        answers: vec![reply_answer("this is fucking great fren", json!({}))],
        ..Options::default()
    })
    .await;
    run(&it.ctx).await.unwrap();

    assert!(it.outlet.replies().is_empty());
    let row = it.queue.get("10").unwrap();
    assert_eq!(row.status, MentionStatus::Failed);
    assert_eq!(row.error.as_deref(), Some("withheld: profanity"));
    let entry = it.entry().await;
    assert_eq!(entry.outcome, Outcome::Held);
    assert_eq!(entry.said, "");
}

#[tokio::test]
async fn cuts_a_link_that_is_not_on_the_allowlist_and_publishes_the_rest() {
    let it = subject(Options {
        answers: vec![reply_answer(
            "claim it here https://claim-tiny.xyz/go fren",
            json!({}),
        )],
        ..Options::default()
    })
    .await;
    run(&it.ctx).await.unwrap();
    assert_eq!(it.outlet.replies()[0].0, "claim it here fren");
    assert_eq!(
        it.queue.get("10").unwrap().reply_text.as_deref(),
        Some("claim it here fren")
    );
}

#[tokio::test]
async fn leaves_an_allowlisted_link_alone() {
    let it = subject(Options {
        answers: vec![reply_answer(
            "the code is at github.com/tinyhumansai/openhuman",
            json!({}),
        )],
        ..Options::default()
    })
    .await;
    run(&it.ctx).await.unwrap();
    assert_eq!(
        it.outlet.replies()[0].0,
        "the code is at github.com/tinyhumansai/openhuman"
    );
}

fn judged(judge: Arc<FixedJudge>) -> SafetyGate {
    SafetyGate::new(SafetyGateOptions {
        judge: Some(judge),
        ..SafetyGateOptions::default()
    })
}

#[tokio::test]
async fn asks_the_judge_about_the_post_and_about_the_draft() {
    let judge = Arc::new(FixedJudge::clean());
    let it = subject(Options {
        safety: Some(judged(judge.clone())),
        ..Options::default()
    })
    .await;
    run(&it.ctx).await.unwrap();

    let seen = judge.seen.lock().unwrap().clone();
    assert!(seen.contains(&("@bot first".to_owned(), Direction::Inbound)));
    assert!(seen.contains(&("reply".to_owned(), Direction::Outbound)));
    assert_eq!(it.outlet.replies().len(), 1);
}

#[tokio::test]
async fn holds_a_mention_the_judge_refuses_even_when_the_words_are_clean() {
    let judge = Arc::new(FixedJudge::of(
        Judgement::Prohibited,
        Some("sexual"),
        "suggestive",
    ));
    let it = subject(Options {
        safety: Some(judged(judge)),
        ..Options::default()
    })
    .await;
    run(&it.ctx).await.unwrap();
    assert!(it.runner.seen().is_empty());
    assert_eq!(
        it.queue.get("10").unwrap().error.as_deref(),
        Some("held: sexual")
    );
}

#[tokio::test]
async fn fails_closed_when_the_judge_cannot_be_reached() {
    let judge = Arc::new(FixedJudge::of(
        Judgement::Prohibited,
        Some("unknown"),
        "the judge could not be reached",
    ));
    let it = subject(Options {
        safety: Some(judged(judge)),
        ..Options::default()
    })
    .await;
    run(&it.ctx).await.unwrap();
    assert!(it.outlet.replies().is_empty());
}

#[tokio::test]
async fn waives_the_inbound_check_for_an_approved_mention_but_still_checks_the_draft() {
    let it = subject(Options {
        pending: vec![post("10", "@bot draw yourself naked", None)],
        answers: vec![reply_answer("lol no", json!({}))],
        ..Options::default()
    })
    .await;
    it.queue.requeue_approved("10").await.unwrap();
    run(&it.ctx).await.unwrap();

    assert_eq!(it.runner.seen().len(), 1);
    assert_eq!(it.outlet.replies()[0].0, "lol no");
}

// Pacing.
#[test]
fn the_gap_is_random_within_the_range_and_never_below_the_minimum() {
    assert_eq!(gap_ms(20, 100, 0.0), 20);
    assert_eq!(gap_ms(20, 100, 1.0), 100);
    assert_eq!(gap_ms(20, 120, 0.5), 70);
    assert_eq!(gap_ms(50, 10, 0.9), 50);
    assert_eq!(gap_ms(0, 100, 0.5), 0);
}

fn two() -> Vec<XPost> {
    vec![
        post("1", "@bot one", Some("a")),
        post("2", "@bot two", Some("b")),
    ]
}

fn hi() -> Vec<String> {
    vec![reply_answer("hi", json!({})), reply_answer("hi", json!({}))]
}

#[tokio::test]
async fn holds_the_second_reply_of_a_batch_back() {
    let it = subject(Options {
        pending: two(),
        answers: hi(),
        vars: vec![
            ("REPLY_MIN_INTERVAL_MS", "80"),
            ("REPLY_MAX_INTERVAL_MS", "80"),
        ],
        ..Options::default()
    })
    .await;
    let started = Instant::now();
    run(&it.ctx).await.unwrap();
    assert_eq!(it.outlet.replies().len(), 2);
    assert!(started.elapsed() >= Duration::from_millis(70));
}

#[tokio::test]
async fn does_not_delay_the_first_reply() {
    let it = subject(Options {
        vars: vec![
            ("REPLY_MIN_INTERVAL_MS", "10000"),
            ("REPLY_MAX_INTERVAL_MS", "10000"),
        ],
        ..Options::default()
    })
    .await;
    let started = Instant::now();
    run(&it.ctx).await.unwrap();
    assert_eq!(it.outlet.replies().len(), 1);
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[tokio::test]
async fn keeps_the_gap_across_runs_not_just_within_a_batch() {
    let it = subject(Options {
        pending: two(),
        answers: hi(),
        vars: vec![
            ("REPLY_MIN_INTERVAL_MS", "80"),
            ("REPLY_MAX_INTERVAL_MS", "80"),
            ("MAX_REPLIES_PER_RUN", "1"),
        ],
        ..Options::default()
    })
    .await;
    run(&it.ctx).await.unwrap();
    let started = Instant::now();
    run(&it.ctx).await.unwrap();
    assert_eq!(it.outlet.replies().len(), 2);
    assert!(started.elapsed() >= Duration::from_millis(70));
}

#[tokio::test]
async fn does_not_pace_a_dry_run_which_publishes_nothing() {
    let it = subject(Options {
        dry_run: true,
        pending: two(),
        answers: hi(),
        vars: vec![
            ("REPLY_MIN_INTERVAL_MS", "10000"),
            ("REPLY_MAX_INTERVAL_MS", "10000"),
        ],
        ..Options::default()
    })
    .await;
    let started = Instant::now();
    run(&it.ctx).await.unwrap();
    assert!(started.elapsed() < Duration::from_secs(1));
}

/// The subject's job over another queue, everything else as built.
fn over_queue(
    ctx: JobCtx,
    queue: Arc<dyn MentionQueue<QueuedXPost>>,
    outlet: Arc<RecordingOutlet>,
    people: Arc<InMemoryPeople>,
) -> JobCtx {
    ctx.with_conversation(Conversation {
        queue,
        people: Some(people),
        outlet,
        account: Arc::new(FixedAccount::new(BOT_ID)),
        ..Conversation::fake()
    })
}

#[tokio::test]
async fn two_batches_at_once_never_both_draft_the_same_mention() {
    let it = subject(Options {
        answers: vec![
            reply_answer("one", json!({})),
            reply_answer("two", json!({})),
        ],
        ..Options::default()
    })
    .await;
    let queue = Arc::new(YieldingQueue(it.queue.clone()));
    let ctx = over_queue(it.ctx, queue, it.outlet.clone(), it.people.clone());

    let (first, second) = tokio::join!(run(&ctx), run(&ctx));
    first.unwrap();
    second.unwrap();

    assert_eq!(it.runner.seen().len(), 1, "drafted once");
    assert_eq!(it.outlet.replies().len(), 1, "answered once");
    assert_eq!(it.queue.get("10").unwrap().status, MentionStatus::Replied);
}

#[tokio::test]
async fn a_crash_after_posting_does_not_answer_the_mention_again() {
    let it = subject(Options {
        answers: vec![
            reply_answer("one", json!({})),
            reply_answer("two", json!({})),
        ],
        memory: Some(ProbeMemory {
            stall_reply_capture: true,
            ..ProbeMemory::default()
        }),
        ..Options::default()
    })
    .await;
    // A lease that is over at once: any claim left behind is stale.
    let queue =
        Arc::new(InMemoryMentionQueue::<QueuedXPost>::default().with_claim_lease(Duration::ZERO));
    queue
        .cache(&[QueuedXPost(post("10", "@bot first", None))])
        .await
        .unwrap();
    let ctx = over_queue(it.ctx, queue.clone(), it.outlet.clone(), it.people.clone());

    // The post goes out, then the process dies while remembering it.
    let crashed = tokio::time::timeout(Duration::from_millis(200), run(&ctx)).await;
    assert!(crashed.is_err(), "the run was cut short");
    assert_eq!(it.outlet.replies().len(), 1);
    assert_eq!(queue.get("10").unwrap().reply_id.as_deref(), Some("99"));

    tokio::time::sleep(Duration::from_millis(5)).await;
    let rerun = tokio::time::timeout(Duration::from_millis(200), run(&ctx)).await;
    assert!(!rerun.unwrap().unwrap().ran, "nothing left to answer");
    assert_eq!(it.outlet.replies().len(), 1, "answered once");
}
