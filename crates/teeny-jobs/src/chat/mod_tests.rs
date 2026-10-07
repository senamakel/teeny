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
use crate::runner::{AgentKey, AgentOutcome, Origin, ScriptedRunner, ToolCallRecord};

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
    clock: Arc<ManualClock>,
    runner: Arc<ScriptedRunner>,
    memory: Arc<ProbeMemory>,
    transcript: Arc<InMemoryChatTranscript>,
}

fn subject_with(runner: ScriptedRunner, memory: ProbeMemory, extra: &[(&str, &str)]) -> Subject {
    subject_using(runner, memory, extra, |_| {})
}

fn subject_using(
    runner: ScriptedRunner,
    memory: ProbeMemory,
    extra: &[(&str, &str)],
    tweak: impl FnOnce(&mut Deps),
) -> Subject {
    let runner = Arc::new(runner);
    let memory = Arc::new(memory);
    let transcript = Arc::new(InMemoryChatTranscript::default());
    let mut deps = Deps::fake();
    deps.memory = memory.clone();
    tweak(&mut deps);
    let mut vars = fake_vars();
    vars.retain(|(key, _)| *key != "DRY_RUN");
    vars.push(("DRY_RUN", "false"));
    vars.extend_from_slice(extra);
    let clock = Arc::new(ManualClock::new(now()));
    let mut ctx = JobCtx::fake(runner.clone())
        .with_clock(clock.clone())
        .with_conversation(Conversation {
            transcript: transcript.clone(),
            ..Conversation::fake()
        });
    ctx.deps = Arc::new(deps);
    ctx.config = Arc::new(Config::from_vars(vars).unwrap());
    Subject {
        ctx,
        clock,
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

// telegram-bot: "logs instead of sending in dry run" -- but unlike the
// legacy bot, a rehearsal writes nothing to memory: the reply was never
// sent, and CortexDB is the live store. The transcript keeps the line.
#[tokio::test]
async fn sends_nothing_in_dry_run_and_writes_nothing_to_memory() {
    // TELEGRAM_DRY_RUN sends nothing; DRY_RUN alone still lets Telegram
    // answer (the switches are separate), but neither writes to memory.
    for (dry, sent) in [
        (("TELEGRAM_DRY_RUN", "true"), None),
        (("DRY_RUN", "true"), Some("gm fren")),
    ] {
        let it = subject_with(gm(), ProbeMemory::default(), &[dry]);
        assert_eq!(
            handle_message(&it.ctx, &dm("10", "hello")).await.as_deref(),
            sent
        );
        assert!(it.memory.captured_kinds().is_empty(), "{dry:?}");
        let turns = it.transcript.recent("1", 5_000, &[]).await.unwrap();
        assert_eq!(turns.last().unwrap().text, "gm fren");
        assert_eq!(turns.last().unwrap().from, Speaker::Bot);
    }
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
        respond(&alone.ctx, &dm("10", "gm"), &GifRation::default()).await,
        Some(ChatAnswer {
            text: None,
            reaction: Some("🔥".into()),
            gif_url: None,
            media: None,
        })
    );
    let turns = alone.transcript.recent("1", 5_000, &[]).await.unwrap();
    assert_eq!(turns.last().unwrap().text, "(reacted 🔥)");
    assert_eq!(turns.last().unwrap().from, Speaker::Bot);

    let both = subject(ScriptedRunner::new().reply(
        json!({ "reply": "gm fren", "reaction": "❤", "incomingTone": "friendly" }).to_string(),
    ));
    assert_eq!(
        respond(&both.ctx, &dm("10", "gm"), &GifRation::default()).await,
        Some(ChatAnswer {
            text: Some("gm fren".into()),
            reaction: Some("❤".into()),
            gif_url: None,
            media: None,
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
    assert_eq!(
        respond(&it.ctx, &dm("10", "gm"), &GifRation::default()).await,
        None
    );
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

// --- GIFs and generated media: telegram-bot.test.ts and chat-model.ts ---

/// A Giphy that records what it was asked, or is down.
#[derive(Default)]
struct RecordingGifs {
    down: bool,
    asked: std::sync::Mutex<Vec<(String, bool)>>,
}

#[async_trait::async_trait]
impl teeny_media::giphy::GifSource for RecordingGifs {
    async fn search(
        &self,
        query: &str,
        channel_only: bool,
    ) -> teeny_media::Result<Vec<teeny_media::giphy::Gif>> {
        self.asked
            .lock()
            .unwrap()
            .push((query.to_owned(), channel_only));
        if self.down {
            return Err(teeny_media::Error::failed("giphy down"));
        }
        // Nothing in the account's own channel; one on the rest of Giphy.
        Ok(if channel_only {
            Vec::new()
        } else {
            vec![teeny_media::giphy::Gif {
                id: "g".into(),
                title: "wave".into(),
                page_url: "p".into(),
                media_url: "https://g/wave.gif".into(),
                mime_type: teeny_media::types::MimeType::Gif,
            }]
        })
    }
    async fn get(&self, _id: &str) -> teeny_media::Result<Option<teeny_media::giphy::Gif>> {
        Ok(None)
    }
    async fn download(
        &self,
        _gif: &teeny_media::giphy::Gif,
    ) -> teeny_media::Result<(Vec<u8>, teeny_media::types::MimeType)> {
        Ok((Vec::new(), teeny_media::types::MimeType::Gif))
    }
}

fn with_gifs(runner: ScriptedRunner, gifs: Arc<RecordingGifs>, extra: &[(&str, &str)]) -> Subject {
    subject_using(runner, ProbeMemory::default(), extra, |deps| {
        deps.gifs = Some(gifs);
    })
}

fn waving() -> ScriptedRunner {
    ScriptedRunner::new()
        .reply(json!({ "reply": "hi", "gifQuery": "wave", "incomingTone": "friendly" }).to_string())
}

// telegram-bot: "sends a GIF after the text when the drafter asked for one,
// and shrugs when it cannot"
#[tokio::test]
async fn finds_the_gif_the_drafter_asked_for_own_channel_first_and_shrugs_when_it_cannot() {
    let gifs = Arc::new(RecordingGifs::default());
    let it = with_gifs(waving(), gifs.clone(), &[]);
    let answer = respond(&it.ctx, &dm("10", "hi"), &GifRation::default())
        .await
        .unwrap();
    assert_eq!(answer.text.as_deref(), Some("hi"));
    assert_eq!(answer.gif_url.as_deref(), Some("https://g/wave.gif"));
    assert_eq!(
        *gifs.asked.lock().unwrap(),
        [("wave".to_owned(), true), ("wave".to_owned(), false)]
    );
    assert_eq!(it.memory.captured_kinds(), vec!["message", "reply"]);

    let down = Arc::new(RecordingGifs {
        down: true,
        ..RecordingGifs::default()
    });
    let it = with_gifs(waving(), down, &[]);
    let answer = respond(&it.ctx, &dm("10", "hi"), &GifRation::default())
        .await
        .unwrap();
    assert_eq!(answer.text.as_deref(), Some("hi"));
    assert_eq!(answer.gif_url, None);
}

// telegram-bot: "rations GIFs to one per chat per cooldown"
#[tokio::test]
async fn rations_gifs_to_one_per_chat_per_cooldown() {
    let hi =
        || json!({ "reply": "hi", "gifQuery": "wave", "incomingTone": "friendly" }).to_string();
    let it = with_gifs(
        ScriptedRunner::new().reply(hi()).reply(hi()).reply(hi()),
        Arc::new(RecordingGifs::default()),
        &[("TELEGRAM_GIF_COOLDOWN_S", "60")],
    );
    let ration = GifRation::default();
    let first = respond(&it.ctx, &dm("10", "hi"), &ration).await.unwrap();
    assert!(first.gif_url.is_some());
    let second = respond(&it.ctx, &dm("11", "hi again"), &ration)
        .await
        .unwrap();
    assert_eq!(second.gif_url, None, "within the cooldown");

    it.clock.advance(TimeDelta::seconds(61));
    let third = respond(&it.ctx, &dm("12", "hi once more"), &ration)
        .await
        .unwrap();
    assert!(third.gif_url.is_some(), "after the cooldown");
}

// chat-model: "A GIF without words to go with it is a GIF sent at somebody."
#[tokio::test]
async fn no_gif_without_words() {
    let gifs = Arc::new(RecordingGifs::default());
    let it = with_gifs(
        ScriptedRunner::new().reply(
            json!({ "reaction": "🔥", "gifQuery": "wave", "incomingTone": "friendly" }).to_string(),
        ),
        gifs.clone(),
        &[],
    );
    let answer = respond(&it.ctx, &dm("10", "gm"), &GifRation::default())
        .await
        .unwrap();
    assert_eq!(answer.gif_url, None);
    assert!(gifs.asked.lock().unwrap().is_empty());
}

fn generated(it: &Subject, kind: teeny_media::types::MediaKind) -> String {
    let media = it.ctx.deps.media.as_ref().unwrap();
    media
        .store
        .put(teeny_media::store::NewMedia {
            kind,
            data: vec![9],
            mime_type: match kind {
                teeny_media::types::MediaKind::Image => teeny_media::types::MimeType::Png,
                teeny_media::types::MediaKind::Video => teeny_media::types::MimeType::Mp4,
            },
            prompt: "a tiny robot".into(),
        })
        .id
}

fn turn_that_made(reply: serde_json::Value, media_id: &str) -> AgentOutcome {
    AgentOutcome {
        reply: reply.to_string(),
        session_id: "s".into(),
        tool_calls: vec![ToolCallRecord {
            name: "generate_image".into(),
            arguments: json!({ "prompt": "a tiny robot" }),
            result: json!({ "mediaId": media_id }).to_string(),
            is_error: false,
        }],
        usage: None,
        model: None,
    }
}

// telegram-bot: "sends a generated image with the reply as its caption, and
// remembers what it said"; "a picture alone when there are no words"
#[tokio::test]
async fn hands_over_media_the_turn_generated_with_or_without_words() {
    let it = subject(ScriptedRunner::new());
    let id = generated(&it, teeny_media::types::MediaKind::Image);
    let mut ctx = it.ctx;
    ctx.runner = Arc::new(ScriptedRunner::new().outcome(turn_that_made(
        json!({ "reply": "here you go", "mediaId": id, "gifQuery": "wave", "incomingTone": "friendly" }),
        &id,
    )));
    let answer = respond(
        &ctx,
        &dm("10", "draw me a tiny robot"),
        &GifRation::default(),
    )
    .await
    .unwrap();
    assert_eq!(answer.text.as_deref(), Some("here you go"));
    assert_eq!(
        answer.media.as_ref().map(|media| media.id.as_str()),
        Some(id.as_str())
    );
    assert_eq!(
        answer.gif_url, None,
        "no GIF next to a picture it just drew"
    );

    let it = subject(ScriptedRunner::new());
    let id = generated(&it, teeny_media::types::MediaKind::Video);
    let (mut ctx, transcript) = (it.ctx, it.transcript);
    ctx.runner = Arc::new(ScriptedRunner::new().outcome(turn_that_made(
        json!({ "mediaId": id, "incomingTone": "friendly" }),
        &id,
    )));
    let answer = respond(&ctx, &dm("10", "make it dance"), &GifRation::default())
        .await
        .unwrap();
    assert_eq!(answer.text, None);
    assert_eq!(answer.media.unwrap().id, id);
    let turns = transcript.recent("1", 5_000, &[]).await.unwrap();
    assert_eq!(turns.last().unwrap().text, "(sent a generated video)");
}

// chat-model: "Only an id a tool actually returned"; telegram-bot: "falls
// back to the words ... when the id is unknown"
#[tokio::test]
async fn ignores_a_media_id_the_turn_did_not_make_and_keeps_the_words() {
    let it = subject(ScriptedRunner::new());
    // In the store (made by some other job), but not by this turn.
    let id = generated(&it, teeny_media::types::MediaKind::Image);
    let mut ctx = it.ctx;
    ctx.runner =
        Arc::new(ScriptedRunner::new().reply(
            json!({ "reply": "here", "mediaId": id, "incomingTone": "friendly" }).to_string(),
        ));
    let answer = respond(&ctx, &dm("10", "draw"), &GifRation::default())
        .await
        .unwrap();
    assert_eq!(answer.text.as_deref(), Some("here"));
    assert_eq!(answer.media, None);

    // Made this turn but gone from the store: the words alone.
    let it = subject(ScriptedRunner::new().outcome(turn_that_made(
        json!({ "reply": "here", "mediaId": "img_notreal_9", "incomingTone": "friendly" }),
        "img_notreal_9",
    )));
    let answer = respond(&it.ctx, &dm("10", "draw"), &GifRation::default())
        .await
        .unwrap();
    assert_eq!(answer.text.as_deref(), Some("here"));
    assert_eq!(answer.media, None);
}
