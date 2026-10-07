use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::TimeDelta;
use serde_json::json;
use teeny_cadence::calendar::{Aspect, CalendarItem, Channel, Media};
use teeny_media::store::{MediaStore, NewMedia};
use teeny_media::types::{MediaKind, MimeType};
use teeny_safety::gate::{SafetyGate, SafetyGateOptions};
use teeny_store::journal::Outcome;

use super::*;
use crate::morning::test_support::{Recorder, Remembers, at, ctx_at};
use crate::runner::{AgentOutcome, ScriptedRunner, ToolCallRecord};

fn gm() -> CalendarItem {
    CalendarItem {
        name: "gm".into(),
        cron: "0 7 * * *".into(),
        channels: vec![Channel::X, Channel::Telegram],
        brief: "A good-morning post.".into(),
        media: Media::Image,
        aspect: Aspect::Square,
        features_tiny: true,
        telegram_chat_ids: None,
        enabled: true,
    }
}

fn text_only(channels: Vec<Channel>) -> CalendarItem {
    CalendarItem {
        media: Media::None,
        channels,
        ..gm()
    }
}

struct Drafts(Result<ContentDraft, String>);

#[async_trait]
impl ContentDrafter for Drafts {
    async fn draft(
        &self,
        _item: &CalendarItem,
        _today: &str,
        _memory: &str,
    ) -> anyhow::Result<ContentDraft> {
        self.0.clone().map_err(|error| anyhow::anyhow!(error))
    }
}

fn draft(text: &str, media_id: Option<&str>) -> ContentDraft {
    ContentDraft {
        text: text.into(),
        media_id: media_id.map(str::to_owned),
        thinking: "t".into(),
        declined: Vec::new(),
        tools: Vec::new(),
    }
}

#[derive(Default)]
struct Chats {
    sent: Mutex<Vec<(i64, String, Option<MimeType>)>>,
}

#[async_trait]
impl TelegramSender for Chats {
    async fn send_message(&self, chat_id: i64, text: &str) -> anyhow::Result<i64> {
        self.sent.lock().unwrap().push((chat_id, text.into(), None));
        Ok(1)
    }
    async fn send_media(
        &self,
        chat_id: i64,
        media: &StoredMedia,
        caption: Option<&str>,
    ) -> anyhow::Result<i64> {
        self.sent.lock().unwrap().push((
            chat_id,
            caption.unwrap_or("").into(),
            Some(media.mime_type),
        ));
        Ok(2)
    }
}

struct Fixture {
    job: CalendarJob,
    poster: Arc<Recorder>,
    chats: Arc<Chats>,
    memory: Arc<Remembers>,
    media: Arc<MediaStore>,
}

fn fixture(drafted: Result<ContentDraft, String>) -> Fixture {
    let poster = Recorder::new();
    let chats = Arc::new(Chats::default());
    let memory = Remembers::saying("yesterday: Tiny with coffee");
    let media = Arc::new(MediaStore::new());
    let job = CalendarJob {
        drafter: Arc::new(Drafts(drafted)),
        memory: memory.clone(),
        poster: poster.clone(),
        telegram: Some(chats.clone()),
        media: Some(media.clone()),
        safety: None,
        telegram_chat_ids: vec!["-100".into()],
        max_emoji: 1,
        dry_run: false,
    };
    Fixture {
        job,
        poster,
        chats,
        memory,
        media,
    }
}

fn picture(media: &MediaStore, kind: MediaKind, mime_type: MimeType) -> String {
    media
        .put(NewMedia {
            kind,
            data: vec![1],
            mime_type,
            prompt: "p".into(),
        })
        .id
}

fn today_ctx() -> crate::ctx::JobCtx {
    ctx_at(at("2026-09-14T07:00:00Z"), Arc::default()).0
}

#[tokio::test]
async fn posts_to_x_and_telegram_with_the_generated_picture_and_remembers_it() {
    let mut f = fixture(Ok(draft("", None)));
    let id = picture(&f.media, MediaKind::Image, MimeType::Png);
    f.job.drafter = Arc::new(Drafts(Ok(draft("gm frens", Some(&id)))));
    let ctx = today_ctx();
    let report = f.job.run(&ctx, &gm(), false).await.unwrap();
    assert_eq!(report.outcome, Outcome::Posted);
    let posts = f.poster.posts();
    assert_eq!(posts[0].kind, "calendar:gm");
    assert_eq!(posts[0].text, "gm frens");
    assert_eq!(posts[0].media.as_ref().unwrap().mime_type, MimeType::Png);
    assert_eq!(
        *f.chats.sent.lock().unwrap(),
        [(-100, "gm frens".to_owned(), Some(MimeType::Png))]
    );
    let (kind, key, payload) = f.memory.activities()[0].clone();
    assert_eq!(kind, "calendar_post");
    assert!(key.ends_with("calendar:gm:2026-09-14"));
    assert_eq!(payload["published"], true);
    assert_eq!(payload["results"], json!(["x:9001", "telegram:-100:2"]));
    let entry = ctx.journal.recent(1).await.unwrap().remove(0);
    assert_eq!(entry.id, "calendar:gm:2026-09-14");
    assert_eq!(entry.incoming, "gm: A good-morning post.");
}

#[tokio::test]
async fn holds_a_post_that_promised_a_picture_and_has_none() {
    let f = fixture(Ok(draft("gm frens", None)));
    let ctx = today_ctx();
    let report = f.job.run(&ctx, &gm(), false).await.unwrap();
    assert_eq!(report.outcome, Outcome::Held);
    assert!(f.poster.posts().is_empty());
    assert!(f.chats.sent.lock().unwrap().is_empty());
    let entry = ctx.journal.recent(1).await.unwrap().remove(0);
    assert_eq!(
        entry.outcome_note.as_deref(),
        Some("the item carries an image and none was generated")
    );
}

#[tokio::test]
async fn sends_text_as_text_a_video_as_a_video_and_the_items_own_chats() {
    let f = fixture(Ok(draft("friday, shipped", None)));
    let item = CalendarItem {
        name: "ship".into(),
        telegram_chat_ids: Some(vec!["-200".into(), "-201".into()]),
        ..text_only(vec![Channel::Telegram])
    };
    f.job.run(&today_ctx(), &item, false).await.unwrap();
    assert!(f.poster.posts().is_empty());
    let sent = f.chats.sent.lock().unwrap().clone();
    assert_eq!(
        sent,
        [
            (-200, "friday, shipped".to_owned(), None),
            (-201, "friday, shipped".to_owned(), None)
        ]
    );

    let mut f = fixture(Ok(draft("", None)));
    let clip = picture(&f.media, MediaKind::Video, MimeType::Mp4);
    f.job.drafter = Arc::new(Drafts(Ok(draft("", Some(&clip)))));
    let sunday = CalendarItem {
        name: "sunday".into(),
        media: Media::Video,
        ..text_only(vec![Channel::Telegram])
    };
    f.job.run(&today_ctx(), &sunday, false).await.unwrap();
    assert_eq!(f.chats.sent.lock().unwrap()[0].2, Some(MimeType::Mp4));
}

#[tokio::test]
async fn in_dry_run_publishes_nothing_but_remembers_and_journals() {
    let mut f = fixture(Ok(draft("", None)));
    let id = picture(&f.media, MediaKind::Image, MimeType::Png);
    f.job.drafter = Arc::new(Drafts(Ok(draft("gm frens", Some(&id)))));
    f.job.dry_run = true;
    let ctx = today_ctx();
    let report = f.job.run(&ctx, &gm(), false).await.unwrap();
    assert_eq!(report.outcome, Outcome::DryRun);
    assert_eq!(report.drafts, ["gm frens"]);
    assert!(f.poster.posts().is_empty());
    assert!(f.chats.sent.lock().unwrap().is_empty());
    assert_eq!(f.memory.activities()[0].2["published"], false);
    assert_eq!(
        ctx.journal.recent(1).await.unwrap()[0].outcome,
        Outcome::DryRun
    );
}

#[tokio::test]
async fn refuses_the_same_item_twice_inside_the_hour_per_item() {
    let f = fixture(Ok(draft("hi", None)));
    let (ctx, clock) = ctx_at(at("2026-09-14T07:00:00Z"), Arc::default());
    let item = text_only(vec![Channel::Telegram]);
    assert!(f.job.run(&ctx, &item, false).await.unwrap().ran);
    assert!(!f.job.run(&ctx, &item, false).await.unwrap().ran);
    let other = CalendarItem {
        name: "other".into(),
        ..item.clone()
    };
    assert!(f.job.run(&ctx, &other, false).await.unwrap().ran);
    assert!(f.job.run(&ctx, &item, true).await.unwrap().ran);
    clock.advance(TimeDelta::minutes(61));
    assert!(f.job.run(&ctx, &item, false).await.unwrap().ran);
}

#[tokio::test]
async fn keeps_going_when_one_channel_fails_and_records_the_failure() {
    let mut f = fixture(Ok(draft("hi", None)));
    f.job.poster = Arc::new(Recorder {
        refuse: Some("duplicate content".into()),
        ..Recorder::default()
    });
    let ctx = today_ctx();
    let report = f
        .job
        .run(&ctx, &text_only(vec![Channel::X, Channel::Telegram]), false)
        .await
        .unwrap();
    assert_eq!(report.outcome, Outcome::Posted);
    assert_eq!(f.chats.sent.lock().unwrap()[0].1, "hi");
    let entry = ctx.journal.recent(1).await.unwrap().remove(0);
    assert!(entry.outcome_note.unwrap().contains("duplicate content"));

    let mut nowhere = fixture(Ok(draft("hi", None)));
    nowhere.job.telegram_chat_ids = Vec::new();
    let ctx = today_ctx();
    let report = nowhere
        .job
        .run(&ctx, &text_only(vec![Channel::Telegram]), false)
        .await
        .unwrap();
    assert_eq!(report.outcome, Outcome::Failed);
}

#[tokio::test]
async fn withholds_a_draft_the_filter_refuses_on_every_channel() {
    let mut f = fixture(Ok(draft("best gun for home defence? tiny knows", None)));
    f.job.safety = Some(Arc::new(SafetyGate::new(SafetyGateOptions::default())));
    let ctx = today_ctx();
    let report = f
        .job
        .run(&ctx, &text_only(vec![Channel::X, Channel::Telegram]), false)
        .await
        .unwrap();
    assert_eq!(report.outcome, Outcome::Held);
    assert!(f.poster.posts().is_empty());
    assert!(f.chats.sent.lock().unwrap().is_empty());
    assert!(f.memory.activities().is_empty());
    let entry = ctx.journal.recent(1).await.unwrap().remove(0);
    assert_eq!(entry.said, "");
    assert!(
        entry
            .outcome_note
            .unwrap()
            .contains("content filter withheld")
    );
}

#[tokio::test]
async fn publishes_what_the_filter_handed_back_with_a_link_cut() {
    let mut f = fixture(Ok(draft(
        "gm frens, see https://totally-legit-claim.xyz/airdrop for more",
        None,
    )));
    f.job.safety = Some(Arc::new(SafetyGate::new(SafetyGateOptions::default())));
    f.job
        .run(&today_ctx(), &text_only(vec![Channel::X]), false)
        .await
        .unwrap();
    assert_eq!(f.poster.posts()[0].text, "gm frens, see for more");
}

#[tokio::test]
async fn journals_a_drafter_that_failed_and_publishes_nothing() {
    let f = fixture(Err("model down".into()));
    let ctx = today_ctx();
    let report = f.job.run(&ctx, &gm(), false).await.unwrap();
    assert_eq!(report.outcome, Outcome::Failed);
    assert!(f.poster.posts().is_empty());
    let entry = ctx.journal.recent(1).await.unwrap().remove(0);
    assert_eq!(entry.outcome_note.as_deref(), Some("model down"));
}

fn turn(reply: &str, calls: Vec<ToolCallRecord>) -> AgentOutcome {
    AgentOutcome {
        reply: reply.into(),
        session_id: "s".into(),
        tool_calls: calls,
        usage: None,
        model: None,
    }
}

#[tokio::test]
async fn the_drafter_asks_the_publisher_belt_once_and_keeps_only_a_media_id_it_made() {
    let made = ToolCallRecord {
        name: "generate_image".into(),
        arguments: json!({ "prompt": "Tiny" }),
        result: r#"{"mediaId": "img_abc_1"}"#.into(),
        is_error: false,
    };
    let runner = Arc::new(ScriptedRunner::new().outcome(turn(
        r#"Here: {"text": "gm frens", "mediaId": "img_abc_1", "thinking": "said coffee yesterday", "declined": ["a pun"]}"#,
        vec![made],
    )));
    let drafter = RunnerDrafter {
        runner: runner.clone(),
    };
    let drafted = drafter.draft(&gm(), "2026-09-14", "").await.unwrap();
    assert_eq!(drafted.text, "gm frens");
    assert_eq!(drafted.media_id.as_deref(), Some("img_abc_1"));
    assert_eq!(drafted.declined, ["a pun"]);
    let seen = runner.seen();
    assert_eq!(
        seen[0].origin,
        crate::runner::Origin::Scheduled { once: true }
    );
    assert!(
        seen[0]
            .prompt
            .contains("Write today's \"gm\" post. Today is 2026-09-14.")
    );
    assert!(
        seen[0]
            .prompt
            .contains("This post CARRIES an image (square, featuring Tiny)")
    );
    assert!(
        seen[0]
            .prompt
            .contains("(nothing recalled: this is the first one, and do not pretend otherwise)")
    );

    let invented = Arc::new(
        ScriptedRunner::new().reply(r#"{"text": "gm", "mediaId": "img_zzz_9", "thinking": "t"}"#),
    );
    let drafted = RunnerDrafter { runner: invented }
        .draft(&gm(), "d", "")
        .await
        .unwrap();
    assert_eq!(drafted.media_id, None);

    let posted_itself = Arc::new(ScriptedRunner::new().outcome(turn(
        r#"{"text": "gm", "thinking": "t"}"#,
        vec![ToolCallRecord {
            name: "x_post".into(),
            arguments: json!({}),
            result: "Published post 1.".into(),
            is_error: false,
        }],
    )));
    let error = RunnerDrafter {
        runner: posted_itself,
    }
    .draft(&gm(), "d", "")
    .await
    .unwrap_err();
    assert!(error.to_string().contains("x_post"));
}
