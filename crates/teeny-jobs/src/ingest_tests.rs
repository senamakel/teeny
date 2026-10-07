//! A port of the `IngestJob` and `thread depth` blocks of
//! `legacy/test/reply-job.test.ts`, plus the shared dedupe with the
//! webhook/stream intake.

use std::sync::Arc;

use chrono::{TimeDelta, TimeZone, Utc};
use serde_json::json;
use teeny_agent::deps::fake_vars;
use teeny_cadence::clock::{ManualClock, iso};
use teeny_core::Config;
use teeny_core::types::UserId;
use teeny_store::{InMemoryMentionQueue, InMemoryStateStore, MentionQueue, StateStore};
use teeny_x::intake::{ActivityIntake, ActivityMemory, IntakeError, Source};
use teeny_x::model::{WindowStart, XPost};

use super::*;
use crate::ctx::{Conversation, JobCtx};
use crate::runner::ScriptedRunner;

const BOT: &str = "2094854245468000256";

struct Subject {
    ctx: JobCtx,
    queue: Arc<InMemoryMentionQueue<QueuedXPost>>,
    cursor: Arc<InMemoryStateStore>,
    mentions: Arc<ScriptedMentions>,
}

fn post(id: &str, text: &str) -> XPost {
    XPost::new(id, text)
}

fn by(mut post: XPost, author: &str, in_reply_to: Option<&str>) -> XPost {
    post.author_id = Some(UserId::from(author));
    post.in_reply_to_user_id = in_reply_to.map(UserId::from);
    post
}

fn subject_with(found: Vec<XPost>, account: Option<&str>, extra: &[(&str, &str)]) -> Subject {
    let mut vars = fake_vars();
    vars.retain(|(key, _)| *key != "X_BOT_USERNAME");
    vars.push(("X_BOT_USERNAME", "bot"));
    vars.extend_from_slice(extra);
    let config = Config::from_vars(vars).expect("config");
    let queue = Arc::new(InMemoryMentionQueue::<QueuedXPost>::default());
    let cursor = Arc::new(InMemoryStateStore::default());
    let mentions = Arc::new(ScriptedMentions::new(found));
    let conversation = Conversation {
        queue: queue.clone(),
        cursor: cursor.clone(),
        mentions: mentions.clone(),
        account: Arc::new(match account {
            Some(id) => FixedAccount::new(id),
            None => FixedAccount::unknown(),
        }),
        ..Conversation::fake()
    };
    let mut ctx = JobCtx::fake(Arc::new(ScriptedRunner::new())).with_conversation(conversation);
    ctx.config = Arc::new(config);
    Subject {
        ctx,
        queue,
        cursor,
        mentions,
    }
}

fn subject(found: Vec<XPost>) -> Subject {
    subject_with(found, Some(BOT), &[])
}

async fn queued_ids(queue: &InMemoryMentionQueue<QueuedXPost>) -> Vec<String> {
    let mut ids: Vec<String> = queue
        .take_pending(100)
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.post_id)
        .collect();
    ids.sort();
    ids
}

#[tokio::test]
async fn caches_what_it_retrieved_and_advances_the_cursor_past_all_of_it() {
    let it = subject(vec![post("11", "@bot second"), post("10", "@bot first")]);
    run(&it.ctx).await.unwrap();

    assert_eq!(queued_ids(&it.queue).await, vec!["10", "11"]);
    let state = it.cursor.load().await.unwrap();
    assert_eq!(state.since_id.as_deref(), Some("11"));
    assert!(state.last_polled_at.is_some());
}

#[tokio::test]
async fn records_the_poll_even_when_nothing_matched_so_the_window_stays_narrow() {
    let it = subject(Vec::new());
    run(&it.ctx).await.unwrap();

    let state = it.cursor.load().await.unwrap();
    assert!(state.last_polled_at.is_some());
    assert!(state.since_id.is_none());
}

#[tokio::test]
async fn never_sends_since_id_and_start_time_together() {
    let it = subject(vec![post("11", "@bot second"), post("10", "@bot first")]);
    run(&it.ctx).await.unwrap();
    run(&it.ctx).await.unwrap();

    let windows = it.mentions.windows();
    assert_eq!(windows.len(), 2);
    assert!(matches!(windows[0].start, WindowStart::StartTime(_)));
    assert_eq!(windows[1].start, WindowStart::SinceId("11".into()));
}

#[tokio::test]
async fn starts_a_fresh_state_at_the_configured_lookback() {
    let now = Utc.with_ymd_and_hms(2026, 9, 2, 12, 0, 0).unwrap();
    let mut it = subject_with(Vec::new(), Some(BOT), &[("MENTION_LOOKBACK_MINUTES", "30")]);
    it.ctx = it.ctx.with_clock(Arc::new(ManualClock::new(now)));
    run(&it.ctx).await.unwrap();

    let windows = it.mentions.windows();
    assert_eq!(
        windows[0].start,
        WindowStart::StartTime(iso(now - TimeDelta::minutes(30)))
    );
    assert_eq!(windows[0].max_results, Some(10));
}

#[tokio::test]
async fn reopens_the_window_a_minute_before_the_last_poll() {
    let now = Utc.with_ymd_and_hms(2026, 9, 2, 12, 0, 0).unwrap();
    let mut it = subject(Vec::new());
    it.ctx = it.ctx.with_clock(Arc::new(ManualClock::new(now)));
    let last = now - TimeDelta::minutes(5);
    it.cursor
        .save(&teeny_store::BotState {
            last_polled_at: Some(iso(last)),
            ..Default::default()
        })
        .await
        .unwrap();
    run(&it.ctx).await.unwrap();

    assert_eq!(
        it.mentions.windows()[0].start,
        WindowStart::StartTime(iso(last - TimeDelta::minutes(1)))
    );
    // And never further back than X's seven days.
    it.cursor
        .save(&teeny_store::BotState {
            last_polled_at: Some(iso(now - TimeDelta::days(30))),
            ..Default::default()
        })
        .await
        .unwrap();
    run(&it.ctx).await.unwrap();
    assert_eq!(
        it.mentions.windows()[1].start,
        WindowStart::StartTime(iso(now - TimeDelta::days(7) + TimeDelta::minutes(1)))
    );
}

#[tokio::test]
async fn queues_a_direct_mention_and_a_direct_reply_to_the_account() {
    let it = subject(vec![
        by(post("20", "@bot hello"), "1", None),
        by(post("21", "@bot nice"), "2", Some(BOT)),
    ]);
    run(&it.ctx).await.unwrap();
    assert_eq!(queued_ids(&it.queue).await, vec!["20", "21"]);
}

#[tokio::test]
async fn drops_a_deep_reply_that_only_carries_the_handle_x_prefixed_for_it() {
    let it = subject(vec![
        by(post("30", "@bot hi"), "1", None),
        by(
            post(
                "31",
                "@someone @bot Rhyming is a bold choice for the timeline",
            ),
            "2",
            Some("999"),
        ),
    ]);
    run(&it.ctx).await.unwrap();
    assert_eq!(queued_ids(&it.queue).await, vec!["30"]);
}

#[tokio::test]
async fn keeps_a_deep_reply_where_somebody_actually_typed_the_handle() {
    let it = subject(vec![by(
        post("32", "@someone Oh hi fren @bot"),
        "2",
        Some("999"),
    )]);
    run(&it.ctx).await.unwrap();
    assert_eq!(queued_ids(&it.queue).await, vec!["32"]);
}

#[tokio::test]
async fn still_advances_the_cursor_past_what_it_dropped() {
    let it = subject(vec![
        by(post("40", "@someone @bot deep"), "2", Some("999")),
        by(post("41", "@someone @bot deeper"), "3", Some("888")),
    ]);
    run(&it.ctx).await.unwrap();

    assert!(queued_ids(&it.queue).await.is_empty());
    assert_eq!(
        it.cursor.load().await.unwrap().since_id.as_deref(),
        Some("41")
    );
}

#[tokio::test]
async fn never_queues_the_accounts_own_post() {
    let it = subject(vec![by(post("50", "@bot me again"), BOT, None)]);
    run(&it.ctx).await.unwrap();
    assert!(queued_ids(&it.queue).await.is_empty());
}

#[tokio::test]
async fn without_the_accounts_id_a_reply_is_never_close_enough() {
    let it = subject_with(
        vec![
            by(post("60", "@bot hello"), "1", None),
            by(post("61", "nice"), "2", Some(BOT)),
        ],
        None,
        &[],
    );
    run(&it.ctx).await.unwrap();
    assert_eq!(queued_ids(&it.queue).await, vec!["60"]);
}

struct NoMemory;

#[async_trait]
impl ActivityMemory for NoMemory {
    async fn capture_activity(
        &self,
        _entry: teeny_x::activity::ActivityEntry,
    ) -> Result<(), IntakeError> {
        Ok(())
    }
}

// The webhook / stream intake and the poll feed one queue, and a mention
// that arrives both ways is queued once.
#[tokio::test]
async fn a_mention_the_stream_delivered_is_not_queued_again_by_the_poll() {
    let mention = by(post("70", "@bot gm"), "5", None);
    let it = subject(vec![mention.clone()]);
    let conversation = it.ctx.conversation().unwrap();
    let intake = ActivityIntake::new(Arc::new(NoMemory)).with_queue(
        Arc::new(QueueSink(conversation.queue.clone())),
        conversation.account.clone(),
        "bot",
    );
    let delivery = json!({
        "data": {
            "event_type": "post.mention.create",
            "event_uuid": "e-1",
            "payload": {
                "id": "70", "text": "@bot gm", "author_id": "5",
                "conversation_id": "70"
            }
        }
    });
    let delivered = intake.deliver(&delivery, Source::Stream).await.unwrap();
    assert_eq!(delivered.queued, 1);
    let report = run(&it.ctx).await.unwrap();

    assert_eq!(queued_ids(&it.queue).await, vec!["70"]);
    assert_eq!(
        report.note.as_deref(),
        Some("retrieved 1, too deep 0, cached 0")
    );
}
