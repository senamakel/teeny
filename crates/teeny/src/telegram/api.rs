//! The Telegram Bot API, as much of it as Teeny uses: the long poll, sending
//! text and media, reacting, the typing indicator, and who the bot is. A
//! port of the calls in `legacy/src/clients/telegram.ts`. Every call is a
//! JSON (or multipart) POST to `<base>/bot<token>/<method>`.

use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use teeny_media::store::StoredMedia;
use teeny_media::types::MediaKind;

use super::inbox::{BotIdentity, Update};

/// A call Telegram refused or that never arrived.
#[derive(Debug, thiserror::Error)]
#[error("telegram {method}: {description}")]
pub struct TelegramError {
    pub method: &'static str,
    /// Telegram's `error_code`; `None` when the request itself failed.
    pub code: Option<i64>,
    pub description: String,
    /// `parameters.retry_after`, on a 429.
    pub retry_after: Option<u64>,
}

#[derive(Deserialize)]
struct Answer<T> {
    ok: bool,
    result: Option<T>,
    error_code: Option<i64>,
    description: Option<String>,
    parameters: Option<Parameters>,
}

#[derive(Deserialize)]
struct Parameters {
    retry_after: Option<u64>,
}

#[derive(Deserialize)]
struct Sent {
    message_id: i64,
}

#[derive(Deserialize)]
struct Me {
    id: i64,
    username: Option<String>,
}

/// How long a call may take beyond the long poll's own wait.
const SLACK: Duration = Duration::from_secs(15);

/// The client.
pub struct TelegramApi {
    http: reqwest::Client,
    base: String,
    token: String,
}

impl TelegramApi {
    pub fn new(token: &str) -> Self {
        Self {
            http: reqwest::Client::new(),
            base: "https://api.telegram.org".into(),
            token: token.to_owned(),
        }
    }

    pub fn with_base_url(mut self, base: impl Into<String>) -> Self {
        self.base = base.into().trim_end_matches('/').to_owned();
        self
    }

    fn url(&self, method: &str) -> String {
        format!("{}/bot{}/{method}", self.base, self.token)
    }

    fn failed(method: &'static str, error: impl std::fmt::Display) -> TelegramError {
        TelegramError {
            method,
            code: None,
            description: error.to_string(),
            retry_after: None,
        }
    }

    async fn read<T: DeserializeOwned>(
        method: &'static str,
        response: reqwest::Response,
    ) -> Result<T, TelegramError> {
        let answer: Answer<T> = response
            .json()
            .await
            .map_err(|error| Self::failed(method, error))?;
        match (answer.ok, answer.result) {
            (true, Some(result)) => Ok(result),
            _ => Err(TelegramError {
                method,
                code: answer.error_code,
                description: answer.description.unwrap_or_else(|| "no reason given".into()),
                retry_after: answer.parameters.and_then(|p| p.retry_after),
            }),
        }
    }

    async fn call<T: DeserializeOwned>(
        &self,
        method: &'static str,
        body: Value,
        timeout: Duration,
    ) -> Result<T, TelegramError> {
        let response = self
            .http
            .post(self.url(method))
            .timeout(timeout)
            .json(&body)
            .send()
            .await
            .map_err(|error| Self::failed(method, error))?;
        Self::read(method, response).await
    }

    /// The bot's own id and handle. A bot without a username cannot tell
    /// which group lines are for it, so that is an error.
    pub async fn get_me(&self) -> Result<BotIdentity, TelegramError> {
        let me: Me = self.call("getMe", json!({}), SLACK).await?;
        let username = me.username.ok_or_else(|| {
            Self::failed("getMe", "the bot has no username; set one with @BotFather")
        })?;
        Ok(BotIdentity { id: me.id, username })
    }

    /// One long poll: what arrived from `offset` on, waiting up to
    /// `timeout_s` seconds for something to.
    pub async fn get_updates(
        &self,
        offset: Option<i64>,
        timeout_s: u32,
    ) -> Result<Vec<Update>, TelegramError> {
        let mut body = json!({ "timeout": timeout_s, "allowed_updates": ["message"] });
        if let Some(offset) = offset {
            body["offset"] = json!(offset);
        }
        self.call(
            "getUpdates",
            body,
            Duration::from_secs(u64::from(timeout_s)) + SLACK,
        )
        .await
    }

    /// Sends `text`, threaded under `reply_to` and into topic `thread` when
    /// given; the sent message's id.
    pub async fn send_message(
        &self,
        chat_id: i64,
        text: &str,
        reply_to: Option<i64>,
        thread: Option<i64>,
    ) -> Result<i64, TelegramError> {
        let mut body = json!({ "chat_id": chat_id, "text": text });
        if let Some(reply_to) = reply_to {
            body["reply_parameters"] =
                json!({ "message_id": reply_to, "allow_sending_without_reply": true });
        }
        if let Some(thread) = thread {
            body["message_thread_id"] = json!(thread);
        }
        let sent: Sent = self.call("sendMessage", body, SLACK).await?;
        Ok(sent.message_id)
    }

    /// An emoji reaction on a message.
    pub async fn set_reaction(
        &self,
        chat_id: i64,
        message_id: i64,
        emoji: &str,
    ) -> Result<(), TelegramError> {
        let body = json!({
            "chat_id": chat_id,
            "message_id": message_id,
            "reaction": [{ "type": "emoji", "emoji": emoji }],
        });
        self.call::<Value>("setMessageReaction", body, SLACK)
            .await
            .map(drop)
    }

    /// "typing…" in the chat (and topic).
    pub async fn typing(&self, chat_id: i64, thread: Option<i64>) -> Result<(), TelegramError> {
        let mut body = json!({ "chat_id": chat_id, "action": "typing" });
        if let Some(thread) = thread {
            body["message_thread_id"] = json!(thread);
        }
        self.call::<Value>("sendChatAction", body, SLACK)
            .await
            .map(drop)
    }

    /// A photo or a clip, with `caption` when given.
    pub async fn send_media(
        &self,
        chat_id: i64,
        media: &StoredMedia,
        caption: Option<&str>,
    ) -> Result<i64, TelegramError> {
        let (method, field, file) = match media.kind {
            MediaKind::Video => ("sendVideo", "video", "clip.mp4"),
            _ => ("sendPhoto", "photo", "picture.png"),
        };
        let part = reqwest::multipart::Part::bytes(media.data.clone())
            .file_name(file)
            .mime_str(media.mime_type.as_str())
            .map_err(|error| Self::failed(method, error))?;
        let mut form = reqwest::multipart::Form::new()
            .text("chat_id", chat_id.to_string())
            .part(field, part);
        if let Some(caption) = caption {
            form = form.text("caption", caption.to_owned());
        }
        let response = self
            .http
            .post(self.url(method))
            .timeout(Duration::from_secs(120))
            .multipart(form)
            .send()
            .await
            .map_err(|error| Self::failed(method, error))?;
        let sent: Sent = Self::read(method, response).await?;
        Ok(sent.message_id)
    }
}

/// What approving a held chat draft sends with (`crate::release`).
#[async_trait]
impl crate::release::ChatSend for TelegramApi {
    async fn send_reply(
        &self,
        chat_id: i64,
        text: &str,
        reply_to: Option<i64>,
        thread: Option<i64>,
    ) -> anyhow::Result<i64> {
        Ok(self.send_message(chat_id, text, reply_to, thread).await?)
    }
}

/// The calendar's Telegram half.
#[async_trait]
impl teeny_jobs::calendar::TelegramSender for TelegramApi {
    async fn send_message(&self, chat_id: i64, text: &str) -> anyhow::Result<i64> {
        Ok(TelegramApi::send_message(self, chat_id, text, None, None).await?)
    }

    async fn send_media(
        &self,
        chat_id: i64,
        media: &StoredMedia,
        caption: Option<&str>,
    ) -> anyhow::Result<i64> {
        Ok(TelegramApi::send_media(self, chat_id, media, caption).await?)
    }
}
