//! Teeny's own Telegram transport: one Bot API long-poll loop, a port of
//! the transport half of `legacy/src/telegram/bot.ts`. The turn itself is
//! `teeny_jobs::chat` ([`JobsHandler`]): the content filter both ways, the
//! transcript and its token budget, the house style and `TELEGRAM_DRY_RUN`
//! all live there; this file only carries messages.
//!
//! - **Debounce.** People type in pieces ("hey", "quick q", then the
//!   question). A line for the account waits `TELEGRAM_DEBOUNCE_MS` for more
//!   from the same person in the same chat, and the burst is answered as one
//!   turn: the lines joined, under the last one's id.
//! - **Order.** One turn at a time per chat, chats in parallel: a DM never
//!   waits on a slow group, and two replies to one person never race.
//! - **Pictures in.** A photo addressed to the account (its largest size, or
//!   an image sent as a document) is fetched through Telegram's own file API
//!   (`getFile`; size-capped, its type sniffed from the bytes) and described
//!   by the vision model before the turn, written in as `[image: …]`. With no
//!   eyes it reads `[sent a photo]`; a failure reads `[sent a photo that
//!   could not be read]`. A group photo not for the account is never looked
//!   at. Nothing in a message is ever fetched as a URL.
//! - **Sending.** "typing…", then the reaction (instant, so the person knows
//!   they were heard), then generated media with the words as its caption
//!   when they fit (the words alone if it will not send), then the text,
//!   split at Telegram's 4096 characters, then a GIF; threaded under the
//!   last message in a group, plain in a DM.
//! - **Lines not for the account** are kept in the transcript, unanswered,
//!   so the next reply knows what the room was talking about.
//! - **Failures** are logged and polled past; a 429 waits what Telegram
//!   asks, a 409 (another poller on the token) thirty seconds.
//! - **Stopping** ends the poll, answers every burst still waiting, and
//!   waits for the turns in flight.
//!
//! `TELEGRAM_DRY_RUN`: nothing is sent and nothing downloaded is sent on;
//! the turn logs what it would have said.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use teeny_jobs::chat::{self, ChatAnswer, ChatMessage, GifRation};
use teeny_jobs::ctx::JobCtx;
use teeny_media::types::{ImageInput, MimeType};
use teeny_memory::ChatKind;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use super::api::{MAX_CAPTION_LENGTH, TelegramApi};
use super::inbox::{
    BotIdentity, MAX_MESSAGE_LENGTH, Message, Update, command_of, is_addressed, picture_files,
    split_for_telegram, to_chat_message, with_pictures_seen,
};

/// The turn behind the transport.
#[async_trait]
pub trait ChatHandler: Send + Sync {
    /// A line the account saw but was not asked about.
    async fn keep_line(&self, message: &ChatMessage);
    /// A message (or a joined burst) for the account: what to send, if anything.
    async fn respond(&self, message: &ChatMessage) -> Option<ChatAnswer>;
    /// Whether anything here can look at a picture.
    fn can_see(&self) -> bool {
        false
    }
    /// What is in a picture someone sent, in words. Untrusted text.
    async fn describe_image(&self, _data: Vec<u8>, _mime_type: MimeType) -> anyhow::Result<String> {
        anyhow::bail!("nothing here can look at a picture")
    }
}

/// The chat turn of `teeny_jobs`, on the service's [`JobCtx`], with the
/// GIF ration it keeps for as long as the transport runs.
pub struct JobsHandler {
    ctx: Arc<JobCtx>,
    gifs: GifRation,
}

impl JobsHandler {
    pub fn new(ctx: Arc<JobCtx>) -> Self {
        Self {
            ctx,
            gifs: GifRation::default(),
        }
    }
}

#[async_trait]
impl ChatHandler for JobsHandler {
    async fn keep_line(&self, message: &ChatMessage) {
        if let Err(error) = chat::keep_line(&self.ctx, message).await {
            tracing::warn!(%error, chat_id = message.chat_id, "could not keep a telegram line");
        }
    }

    async fn respond(&self, message: &ChatMessage) -> Option<ChatAnswer> {
        chat::respond(&self.ctx, message, &self.gifs).await
    }

    fn can_see(&self) -> bool {
        self.ctx
            .deps
            .media
            .as_ref()
            .is_some_and(|media| media.reader.is_some())
    }

    async fn describe_image(&self, data: Vec<u8>, mime_type: MimeType) -> anyhow::Result<String> {
        let reader = self
            .ctx
            .deps
            .media
            .as_ref()
            .and_then(|media| media.reader.clone())
            .ok_or_else(|| anyhow::anyhow!("nothing here can look at a picture"))?;
        Ok(reader
            .describe(ImageInput::Bytes { data, mime_type }, None)
            .await?)
    }
}

/// How the loop runs.
#[derive(Debug, Clone)]
pub struct PollerOptions {
    pub bot: BotIdentity,
    /// `TELEGRAM_DEBOUNCE_MS`.
    pub debounce: Duration,
    /// `TELEGRAM_POLL_TIMEOUT_S`.
    pub poll_timeout_s: u32,
    /// `TELEGRAM_DRY_RUN`: nothing is sent, not even "typing…".
    pub dry_run: bool,
}

/// One person's lines in one chat, waiting to be answered together.
struct Batch {
    chat_id: i64,
    thread: Option<i64>,
    last_id: i64,
    messages: Vec<ChatMessage>,
    /// Bumped by every line; a debounce timer only flushes its own.
    generation: u64,
}

struct Inner {
    api: Arc<TelegramApi>,
    handler: Arc<dyn ChatHandler>,
    options: PollerOptions,
    batches: Mutex<HashMap<String, Batch>>,
    chats: Mutex<HashMap<i64, Arc<tokio::sync::Mutex<()>>>>,
    turns: Mutex<Vec<JoinHandle<()>>>,
}

fn locked<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poison| poison.into_inner())
}

impl Inner {
    async fn receive(self: &Arc<Self>, update: Update) {
        let Some(raw) = update.message else {
            return;
        };
        let bot = &self.options.bot;
        let Some(message) = to_chat_message(&raw, bot) else {
            return;
        };
        if !is_addressed(&raw, bot) {
            // A vision call per photo in a busy group is a bill for pictures
            // nobody asked about: the room's lines keep `[sent a photo]`.
            self.handler.keep_line(&message).await;
            return;
        }
        let message = self.look(&raw, message).await;
        if let Some(command) = command_of(&raw).filter(|command| command != "start") {
            tracing::info!(
                chat_id = message.chat_id,
                command,
                "unknown telegram command, answering as text"
            );
        }
        // Per person per chat: two people addressing the account at once
        // are two conversations.
        let key = format!("{}:{}", message.chat_id, message.author_id);
        let generation = {
            let mut batches = locked(&self.batches);
            let batch = batches.entry(key.clone()).or_insert_with(|| Batch {
                chat_id: raw.chat.id,
                thread: raw.message_thread_id,
                last_id: raw.message_id,
                messages: Vec::new(),
                generation: 0,
            });
            batch.last_id = raw.message_id;
            batch.thread = raw.message_thread_id.or(batch.thread);
            batch.messages.push(message);
            batch.generation += 1;
            batch.generation
        };
        if self.options.debounce.is_zero() {
            self.flush(&key, None);
            return;
        }
        let inner = Arc::clone(self);
        let wait = self.options.debounce;
        tokio::spawn(async move {
            tokio::time::sleep(wait).await;
            inner.flush(&key, Some(generation));
        });
    }

    /// The message with its pictures written in as `[image: …]`, when
    /// something here can look; each failure is one picture described as
    /// unread rather than a message dropped.
    async fn look(&self, raw: &Message, message: ChatMessage) -> ChatMessage {
        let files = picture_files(raw);
        if files.is_empty() || !self.handler.can_see() {
            return message;
        }
        let mut notes = Vec::with_capacity(files.len());
        for file in files {
            let seen = match self.api.download_image(&file).await {
                Ok((data, mime_type)) => self.handler.describe_image(data, mime_type).await,
                Err(error) => Err(error.into()),
            };
            notes.push(match seen {
                Ok(description) => format!(
                    "[image: {}]",
                    description.split_whitespace().collect::<Vec<_>>().join(" ")
                ),
                Err(error) => {
                    tracing::warn!(%error, chat_id = message.chat_id, "could not look at a telegram photo");
                    "[sent a photo that could not be read]".to_owned()
                }
            });
        }
        with_pictures_seen(raw, &self.options.bot, notes).unwrap_or(message)
    }

    /// Hands the batch under `key` to its chat's turn, unless a later line
    /// has restarted its wait.
    fn flush(self: &Arc<Self>, key: &str, generation: Option<u64>) {
        let batch = {
            let mut batches = locked(&self.batches);
            match batches.get(key) {
                Some(batch) if generation.is_none_or(|g| g == batch.generation) => {
                    batches.remove(key)
                }
                _ => None,
            }
        };
        let Some(batch) = batch else {
            return;
        };
        let lane = Arc::clone(
            locked(&self.chats)
                .entry(batch.chat_id)
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
        );
        let inner = Arc::clone(self);
        let turn = tokio::spawn(async move {
            let _turn = lane.lock().await;
            inner.answer(batch).await;
        });
        let mut turns = locked(&self.turns);
        turns.retain(|turn| !turn.is_finished());
        turns.push(turn);
    }

    /// One burst, answered as one turn.
    async fn answer(&self, batch: Batch) {
        let Some(last) = batch.messages.last() else {
            return;
        };
        let mut message = last.clone();
        if batch.messages.len() > 1 {
            message.text = batch
                .messages
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            message.batch_ids = batch.messages.iter().map(|line| line.id.clone()).collect();
        }
        let api = &self.api;
        if !self.options.dry_run
            && let Err(error) = api.typing(batch.chat_id, batch.thread).await
        {
            tracing::debug!(%error, "could not show typing");
        }
        let Some(answer) = self.handler.respond(&message).await else {
            return;
        };
        if self.options.dry_run {
            return;
        }
        if let Some(reaction) = &answer.reaction
            && let Err(error) = api
                .set_reaction(batch.chat_id, batch.last_id, reaction)
                .await
        {
            tracing::warn!(%error, reaction, "could not react on telegram");
        }
        let reply_to = (message.chat_kind == ChatKind::Group).then_some(batch.last_id);
        let pieces = answer
            .text
            .as_deref()
            .map(|text| split_for_telegram(text, MAX_MESSAGE_LENGTH))
            .unwrap_or_default();
        // A generated picture goes out with the words as its caption when
        // they fit, the way a person sends one; otherwise the words follow.
        // A picture that will not send falls back to the words alone.
        let mut remaining = pieces.as_slice();
        if let Some(media) = &answer.media {
            let caption = match pieces.as_slice() {
                [only] if only.chars().count() <= MAX_CAPTION_LENGTH => Some(only.as_str()),
                _ => None,
            };
            match api
                .send_media(batch.chat_id, media, caption, reply_to, batch.thread)
                .await
            {
                Ok(_) => {
                    tracing::info!(
                        chat_id = batch.chat_id,
                        media_id = media.id,
                        kind = media.kind.as_str(),
                        captioned = caption.is_some(),
                        "sent generated media on telegram"
                    );
                    if caption.is_some() {
                        remaining = &[];
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, media_id = media.id, "could not send generated media; text only");
                }
            }
        }
        for piece in remaining {
            if let Err(error) = api
                .send_message(batch.chat_id, piece, reply_to, batch.thread)
                .await
            {
                tracing::error!(%error, chat_id = batch.chat_id, "could not send a telegram reply");
                return;
            }
        }
        // A GIF is a nice touch rather than the point: after the words, and
        // only with words.
        if let Some(url) = answer.gif_url.as_deref().filter(|_| !pieces.is_empty())
            && let Err(error) = api.send_animation(batch.chat_id, url, batch.thread).await
        {
            tracing::warn!(%error, "could not send a telegram gif");
        }
    }

    /// Answers every waiting burst now and waits for the turns in flight.
    async fn drain(self: &Arc<Self>) {
        let keys: Vec<String> = locked(&self.batches).keys().cloned().collect();
        for key in keys {
            self.flush(&key, None);
        }
        let turns: Vec<JoinHandle<()>> = std::mem::take(&mut *locked(&self.turns));
        for turn in turns {
            let _ = turn.await;
        }
    }
}

/// The loop, before it starts.
pub struct Poller {
    inner: Arc<Inner>,
}

/// The loop, running.
pub struct Running {
    stop: watch::Sender<bool>,
    handle: JoinHandle<()>,
    inner: Arc<Inner>,
}

impl Poller {
    pub fn new(
        api: Arc<TelegramApi>,
        handler: Arc<dyn ChatHandler>,
        options: PollerOptions,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                api,
                handler,
                options,
                batches: Mutex::new(HashMap::new()),
                chats: Mutex::new(HashMap::new()),
                turns: Mutex::new(Vec::new()),
            }),
        }
    }

    pub fn start(self) -> Running {
        let (stop, mut stopped) = watch::channel(false);
        let inner = Arc::clone(&self.inner);
        tracing::info!(
            bot = inner.options.bot.username,
            dry_run = inner.options.dry_run,
            "telegram: polling"
        );
        let handle = tokio::spawn(async move {
            let mut offset: Option<i64> = None;
            while !*stopped.borrow() {
                let polled = tokio::select! {
                    polled = inner.api.get_updates(offset, inner.options.poll_timeout_s) => polled,
                    _ = stopped.changed() => break,
                };
                let wait = match polled {
                    Ok(updates) => {
                        for update in updates {
                            offset = Some(update.update_id + 1);
                            inner.receive(update).await;
                        }
                        continue;
                    }
                    Err(error) => {
                        let wait = match (error.retry_after, error.code) {
                            (Some(seconds), _) => Duration::from_secs(seconds),
                            (None, Some(409)) => Duration::from_secs(30),
                            _ => Duration::from_secs(5),
                        };
                        tracing::error!(%error, retry_after_ms = wait.as_millis() as u64, "telegram poll failed");
                        wait
                    }
                };
                tokio::select! {
                    () = tokio::time::sleep(wait) => {}
                    _ = stopped.changed() => break,
                }
            }
        });
        Running {
            stop,
            handle,
            inner: self.inner,
        }
    }
}

impl Running {
    /// Stops polling, then lets every waiting and running turn finish.
    pub async fn stop(self) {
        let _ = self.stop.send(true);
        let _ = self.handle.await;
        self.inner.drain().await;
        tracing::info!("telegram: stopped");
    }
}

#[cfg(test)]
#[path = "poller_tests.rs"]
mod tests;
