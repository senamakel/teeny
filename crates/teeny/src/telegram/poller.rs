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
//! - **Sending.** "typing…", then the reaction (instant, so the person knows
//!   they were heard), then the text, split at Telegram's 4096 characters;
//!   threaded under the last message in a group, plain in a DM.
//! - **Lines not for the account** are kept in the transcript, unanswered,
//!   so the next reply knows what the room was talking about.
//! - **Failures** are logged and polled past; a 429 waits what Telegram
//!   asks, a 409 (another poller on the token) thirty seconds.
//! - **Stopping** ends the poll, answers every burst still waiting, and
//!   waits for the turns in flight.
//!
//! Not ported: GIFs and generated media in a chat reply (the Rust chat
//! drafter returns text and a reaction only), and describing a photo with
//! the vision model (a picture reads as `[sent a photo]`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use teeny_jobs::chat::{self, ChatAnswer, ChatMessage};
use teeny_jobs::ctx::JobCtx;
use teeny_memory::ChatKind;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use super::api::TelegramApi;
use super::inbox::{
    BotIdentity, MAX_MESSAGE_LENGTH, Update, command_of, is_addressed, split_for_telegram,
    to_chat_message,
};

/// The turn behind the transport.
#[async_trait]
pub trait ChatHandler: Send + Sync {
    /// A line the account saw but was not asked about.
    async fn keep_line(&self, message: &ChatMessage);
    /// A message (or a joined burst) for the account: what to send, if anything.
    async fn respond(&self, message: &ChatMessage) -> Option<ChatAnswer>;
}

/// The chat turn of `teeny_jobs`, on the service's [`JobCtx`].
pub struct JobsHandler(pub Arc<JobCtx>);

#[async_trait]
impl ChatHandler for JobsHandler {
    async fn keep_line(&self, message: &ChatMessage) {
        if let Err(error) = chat::keep_line(&self.0, message).await {
            tracing::warn!(%error, chat_id = message.chat_id, "could not keep a telegram line");
        }
    }

    async fn respond(&self, message: &ChatMessage) -> Option<ChatAnswer> {
        chat::respond(&self.0, message).await
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
            self.handler.keep_line(&message).await;
            return;
        }
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
        for piece in pieces {
            if let Err(error) = api
                .send_message(batch.chat_id, &piece, reply_to, batch.thread)
                .await
            {
                tracing::error!(%error, chat_id = batch.chat_id, "could not send a telegram reply");
                return;
            }
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
