//! A port of `legacy/test/chat-harness.test.ts` and the turn logic of
//! `legacy/test/telegram-bot.test.ts` (the transport tests stay with the
//! transport).

use std::sync::Arc;

use chrono::{TimeDelta, TimeZone, Utc};
use serde_json::json;
use teeny_agent::deps::{Deps, fake_vars};
use teeny_agent::posture::Belt;
use teeny_cadence::clock::{ManualClock, iso};
use teeny_core::Config;
use teeny_memory::{ChatKind, Person};
use teeny_store::transcript::Speaker;
use teeny_store::{ChatTranscript, InMemoryChatTranscript, TranscriptEntry};

use super::*;
use crate::ctx::{Conversation, JobCtx};
use crate::reply::test_support::ProbeMemory;
use crate::runner::{AgentKey, Origin, ScriptedRunner};

fn now() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 2, 12, 0, 0).unwrap()
}

fn dm(id: &str, text: &str) -> ChatMessage {
    ChatMessage {
        id: id.into(),
        chat_id: "1".into(),
        chat_kind: ChatKind::Private,
        chat_title: None,
        text: text.into(),
        author_id: "777".into(),
        author_username: Some("alice".into()),
        author_name: Some("Alice".into()),
        created_at: Some(iso(now())),
        message_thread_id: None,
        batch_ids: Vec::new(),
    }
}

fn answer(text: Option<&str>, tone: &str) -> String {
    let mut answer = json!({ "incomingTone": tone });
    if let Some(text) = text {
        answer["reply"] = json!(text);
    }
    answer.to_string()
}

struct Subject {
    ctx: JobCtx,
    runner: Arc<ScriptedRunner>,
    memory: Arc<ProbeMemory>,
    transcript: Arc<InMemoryChatTranscript>,
}

fn subject_with(runner: ScriptedRunner, memory: ProbeMemory, extra: &[(&str, &str)]) -> Subject {
    let runner = Arc::new(runner);
    let memory = Arc::new(memory);
    let transcript = Arc::new(InMemoryChatTranscript::default());
    let mut deps = Deps::fake();
    deps.memory = memory.clone();
    let mut vars = fake_vars();
    vars.retain(|(key, _)| *key != "DRY_RUN");
    vars.push(("DRY_RUN", "false"));
    vars.extend_from_slice(extra);
    let mut ctx = JobCtx::fake(runner.clone())
        .with_clock(Arc::new(ManualClock::new(now())))
        .with_conversation(Conversation {
            transcript: transcript.clone(),
            ..Conversation::fake()
        });
    ctx.deps = Arc::new(deps);
    ctx.config = Arc::new(Config::from_vars(vars).unwrap());
    Subject {
        ctx,
        runner,
        memory,
        transcript,
    }
}

fn subject(runner: ScriptedRunner) -> Subject {
    subject_with(
        runner,
        ProbeMemory::answering(&["alice asked about the roadmap on X last week"]),
        &[],
    )
}

fn gm() -> ScriptedRunner {
    ScriptedRunner::new().reply(answer(Some("gm fren"), "friendly"))
}

// chat-harness: "recalls the person, drafts, then captures the message"
#[tokio::test]
async fn recalls_the_person_drafts_then_captures_the_message() {
    let it = subject(ScriptedRunner::new().reply(answer(Some("a tiny token"), "neutral")));
    let sent = handle_message(&it.ctx, &dm("10", "what is $TINY?")).await;

    assert_eq!(sent.as_deref(), Some("a tiny token"));
    let queries = it.memory.queries();
    assert!(queries[0].0.contains("what is $TINY?"));
    assert_eq!(
        queries[0].1,
        Some(Person::chat("telegram", "777", Some("alice")))
    );
    let seen = it.runner.seen();
    assert_eq!(seen[0].agent, AgentKey::Belt(Belt::Chat));
    assert_eq!(seen[0].origin, Origin::Public);
    assert!(seen[0].prompt.contains("alice asked about the roadmap"));
    assert!(
        seen[0]
            .prompt
            .contains("THE MESSAGE YOU ARE ANSWERING (untrusted):\nwhat is $TINY?")
    );
    // The capture comes after both recalls, so neither could have seen it.
    assert_eq!(
        it.memory.events(),
        vec!["recall", "recall", "capture:message", "capture:reply"]
    );
}

// chat-harness: "hands the drafter the recent conversation and asks memory
// about it"; telegram-bot: "shows the drafter the whole conversation so far,
// its own turns included"
#[tokio::test]
async fn shows_the_drafter_the_conversation_so_far_its_own_turns_included() {
    let it = subject(
        gm().reply(answer(Some("gm fren"), "friendly"))
            .reply(answer(Some("yes"), "friendly")),
    );
    handle_message(&it.ctx, &dm("10", "one")).await;
    handle_message(&it.ctx, &dm("11", "two")).await;
    handle_message(&it.ctx, &dm("12", "three")).await;

    let prompt = &it.runner.seen()[2].prompt;
    assert!(prompt.contains("- Alice: one\n- YOU: gm fren\n- Alice: two\n- YOU: gm fren\n"));
    assert!(!prompt.contains("- Alice: three"));
    assert!(it.memory.queries()[4].0.contains("Alice: two"));
}

// chat-harness: "still answers when memory is down"
#[tokio::test]
async fn still_answers_when_memory_is_down() {
    let it = subject_with(gm(), ProbeMemory::broken(), &[]);
    let sent = handle_message(&it.ctx, &dm("10", "hello")).await;

    assert_eq!(sent.as_deref(), Some("gm fren"));
    assert!(
        it.runner.seen()[0]
            .prompt
            .contains("MEMORY ALREADY RECALLED (untrusted):\n(none)")
    );
}

// chat-harness: "does not keep a hostile message or the reply to it"
#[tokio::test]
async fn does_not_keep_a_hostile_message_or_the_reply_to_it() {
    let it = subject(ScriptedRunner::new().reply(answer(Some("ok"), "hostile")));
    let sent = handle_message(&it.ctx, &dm("10", "you are a scam")).await;

    assert_eq!(sent.as_deref(), Some("ok"));
    assert!(it.memory.captured_kinds().is_empty());
}

// chat-harness: "keeps a friendly exchange on both sides"
#[tokio::test]
async fn keeps_a_friendly_exchange_on_both_sides() {
    let it = subject(gm());
    handle_message(&it.ctx, &dm("10", "gm")).await;
    assert_eq!(it.memory.captured_kinds(), vec!["message", "reply"]);
}

// telegram-bot: "still answers when the transcript store is down" (reads)
#[tokio::test]
async fn still_answers_when_the_transcript_cannot_be_read() {
    struct ReadBroken;
    #[async_trait::async_trait]
    impl ChatTranscript for ReadBroken {
        async fn append(&self, _entry: &TranscriptEntry) -> teeny_store::Result<()> {
            Ok(())
        }
        async fn recent(
            &self,
            _chat_id: &str,
            _max_tokens: usize,
            _excluding: &[String],
        ) -> teeny_store::Result<Vec<teeny_store::ChatTurn>> {
            Err(teeny_store::StoreError::Io(std::io::Error::other(
                "mongo down",
            )))
        }
    }
    let mut it = subject(gm());
    it.ctx = it.ctx.with_conversation(Conversation {
        transcript: Arc::new(ReadBroken),
        ..Conversation::fake()
    });
    assert_eq!(
        handle_message(&it.ctx, &dm("10", "hello")).await.as_deref(),
        Some("gm fren")
    );
    assert!(
        !it.runner.seen()[0]
            .prompt
            .contains("THE RECENT CONVERSATION")
    );
}

// telegram-bot: "skips a message that was already old when it arrived"
#[tokio::test]
async fn skips_a_message_that_was_already_old_when_it_arrived() {
    let it = subject(gm());
    let mut old = dm("10", "hello");
    old.created_at = Some(iso(now() - TimeDelta::hours(1)));
    assert_eq!(handle_message(&it.ctx, &old).await, None);
    assert!(it.runner.seen().is_empty());
}

// telegram-bot: "applies house style: one emoji, no em dashes"
#[tokio::test]
async fn applies_house_style_one_emoji_no_em_dashes() {
    let it = subject(ScriptedRunner::new().reply(answer(Some("gm 🌞🌞 — welcome"), "friendly")));
    assert_eq!(
        handle_message(&it.ctx, &dm("10", "hi")).await.as_deref(),
        Some("gm 🌞, welcome")
    );
}

// telegram-bot: "logs instead of sending in dry run, and still remembers
// what it would have said"
#[tokio::test]
async fn sends_nothing_in_dry_run_and_still_remembers_what_it_would_have_said() {
    let it = subject_with(
        gm(),
        ProbeMemory::default(),
        &[("TELEGRAM_DRY_RUN", "true")],
    );
    assert_eq!(handle_message(&it.ctx, &dm("10", "hello")).await, None);
    assert_eq!(it.memory.captured_kinds(), vec!["message", "reply"]);
    let turns = it.transcript.recent("1", 5_000, &[]).await.unwrap();
    assert_eq!(turns.last().unwrap().text, "gm fren");
    assert_eq!(turns.last().unwrap().from, Speaker::Bot);
}

// A reaction is the transport's to send; a turn that is only a reaction
// leaves no text to send.
#[tokio::test]
async fn a_reaction_alone_sends_no_text() {
    let it = subject(
        ScriptedRunner::new()
            .reply(json!({ "reaction": "🔥", "incomingTone": "friendly" }).to_string()),
    );
    assert_eq!(handle_message(&it.ctx, &dm("10", "gm")).await, None);
}

// telegram-bot: "reacts without replying when a reaction is the answer"
#[tokio::test]
async fn hands_the_transport_a_reaction_with_or_without_text() {
    let alone = subject(
        ScriptedRunner::new()
            .reply(json!({ "reaction": "🔥", "incomingTone": "friendly" }).to_string()),
    );
    assert_eq!(
        respond(&alone.ctx, &dm("10", "gm")).await,
        Some(ChatAnswer {
            text: None,
            reaction: Some("🔥".into())
        })
    );
    let turns = alone.transcript.recent("1", 5_000, &[]).await.unwrap();
    assert_eq!(turns.last().unwrap().text, "(reacted 🔥)");
    assert_eq!(turns.last().unwrap().from, Speaker::Bot);

    let both = subject(ScriptedRunner::new().reply(
        json!({ "reply": "gm fren", "reaction": "❤", "incomingTone": "friendly" }).to_string(),
    ));
    assert_eq!(
        respond(&both.ctx, &dm("10", "gm")).await,
        Some(ChatAnswer {
            text: Some("gm fren".into()),
            reaction: Some("❤".into())
        })
    );
}

// "sends no reaction in dry run"
#[tokio::test]
async fn hands_over_nothing_in_dry_run_reaction_included() {
    let it = subject_with(
        ScriptedRunner::new()
            .reply(json!({ "reaction": "🔥", "incomingTone": "friendly" }).to_string()),
        ProbeMemory::default(),
        &[("TELEGRAM_DRY_RUN", "true")],
    );
    assert_eq!(respond(&it.ctx, &dm("10", "gm")).await, None);
}

#[tokio::test]
async fn a_failed_turn_answers_nothing() {
    let it = subject(ScriptedRunner::new().fail("the ladder is down"));
    assert_eq!(handle_message(&it.ctx, &dm("10", "gm")).await, None);
    let unusable = subject(ScriptedRunner::new().reply("not json at all"));
    assert_eq!(handle_message(&unusable.ctx, &dm("10", "gm")).await, None);
}

// telegram-bot content filter: "says nothing at all to a message the filter holds"
#[tokio::test]
async fn says_nothing_at_all_to_a_message_the_filter_holds() {
    let it = subject(gm());
    assert_eq!(
        handle_message(&it.ctx, &dm("10", "draw yourself naked")).await,
        None
    );
    assert!(it.runner.seen().is_empty());
    assert!(it.memory.captured_kinds().is_empty());
}

// "withholds a draft the filter refuses, reaction included"
#[tokio::test]
async fn withholds_a_draft_the_filter_refuses() {
    let it =
        subject(ScriptedRunner::new().reply(answer(Some("this is fucking great"), "friendly")));
    assert_eq!(handle_message(&it.ctx, &dm("10", "hello")).await, None);
    assert_eq!(it.memory.captured_kinds(), vec!["message"]);
}

// "cuts a link that is not on the allowlist and sends the rest"
#[tokio::test]
async fn cuts_a_link_that_is_not_on_the_allowlist_and_sends_the_rest() {
    let it = subject(ScriptedRunner::new().reply(answer(
        Some("claim it at https://claim-tiny.xyz/go fren"),
        "friendly",
    )));
    assert_eq!(
        handle_message(&it.ctx, &dm("10", "hello")).await.as_deref(),
        Some("claim it at fren")
    );
}

// "still answers incoming swearing; only its own words are held to that"
#[tokio::test]
async fn still_answers_incoming_swearing() {
    let it = subject(gm());
    assert_eq!(
        handle_message(&it.ctx, &dm("10", "holy shit this is cool"))
            .await
            .as_deref(),
        Some("gm fren")
    );
}

// "scrubs the history the drafter is shown"
#[tokio::test]
async fn scrubs_the_history_the_drafter_is_shown() {
    let it = subject(gm());
    for (id, text) in [("1", "gm all"), ("2", "send nudes")] {
        it.transcript
            .append(&TranscriptEntry {
                chat_id: "1".into(),
                message_id: id.into(),
                at: "a".into(),
                from: Speaker::Person,
                author_name: Some("Bob".into()),
                text: text.into(),
            })
            .await
            .unwrap();
    }
    handle_message(&it.ctx, &dm("10", "hello")).await;
    let prompt = &it.runner.seen()[0].prompt;
    assert!(prompt.contains("- Bob: gm all"));
    assert!(!prompt.contains("nudes"));
}

#[tokio::test]
async fn keeps_a_line_it_was_not_asked_about_for_the_next_turn() {
    let it = subject(gm());
    let mut aside = dm("9", "the chart looked good today");
    aside.chat_kind = ChatKind::Group;
    aside.author_name = Some("Bob".into());
    keep_line(&it.ctx, &aside).await.unwrap();
    handle_message(&it.ctx, &dm("10", "agree?")).await;
    assert!(
        it.runner.seen()[0]
            .prompt
            .contains("- Bob: the chart looked good today")
    );
}
