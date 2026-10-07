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
use teeny_media::types::{MediaKind, MimeType};
use teeny_media::vision::{MAX_IMAGE_BYTES, sniff};

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
struct File {
    #[serde(default)]
    file_size: Option<u64>,
    #[serde(default)]
    file_path: Option<String>,
}

#[derive(Deserialize)]
struct Me {
    id: i64,
    username: Option<String>,
}

/// How long downloading a picture someone sent may take.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(60);

/// Telegram's longest caption.
pub const MAX_CAPTION_LENGTH: usize = 1_024;

/// Whether a `file_path` from `getFile` stays a relative path under the
/// file API: no scheme, no host, no climbing out, no query.
fn is_file_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains("://")
        && !path.contains(['?', '#', '\\'])
        && !path.split('/').any(|part| part == ".." || part.is_empty())
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

    /// Points the client at a stand-in Bot API.
    #[cfg(test)]
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

    /// A request that failed on the way. The token is in the URL's path, and
    /// a `reqwest::Error` prints its URL, so the URL is dropped first: this
    /// text is logged.
    fn unsent(method: &'static str, error: reqwest::Error) -> TelegramError {
        Self::failed(method, error.without_url())
    }

    async fn read<T: DeserializeOwned>(
        method: &'static str,
        response: reqwest::Response,
    ) -> Result<T, TelegramError> {
        let answer: Answer<T> = response
            .json()
            .await
            .map_err(|error| Self::unsent(method, error))?;
        match (answer.ok, answer.result) {
            (true, Some(result)) => Ok(result),
            _ => Err(TelegramError {
                method,
                code: answer.error_code,
                description: answer
                    .description
                    .unwrap_or_else(|| "no reason given".into()),
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
            .map_err(|error| Self::unsent(method, error))?;
        Self::read(method, response).await
    }

    /// The bot's own id and handle. A bot without a username cannot tell
    /// which group lines are for it, so that is an error.
    pub async fn get_me(&self) -> Result<BotIdentity, TelegramError> {
        let me: Me = self.call("getMe", json!({}), SLACK).await?;
        let username = me.username.ok_or_else(|| {
            Self::failed("getMe", "the bot has no username; set one with @BotFather")
        })?;
        Ok(BotIdentity {
            id: me.id,
            username,
        })
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

    /// A GIF by its URL (Giphy's; never one from a message), threaded into
    /// topic `thread` when given.
    pub async fn send_animation(
        &self,
        chat_id: i64,
        url: &str,
        thread: Option<i64>,
    ) -> Result<i64, TelegramError> {
        let mut body = json!({ "chat_id": chat_id, "animation": url });
        if let Some(thread) = thread {
            body["message_thread_id"] = json!(thread);
        }
        let sent: Sent = self.call("sendAnimation", body, SLACK).await?;
        Ok(sent.message_id)
    }

    /// The bytes of a picture someone sent, by its `file_id`, through
    /// Telegram's own file API only: `getFile`, then the file under
    /// `/file/bot<token>/`. Refused past the vision limit (on the size
    /// `getFile` reports, then on the bytes themselves), and unless the
    /// bytes are a PNG, JPEG, GIF or WebP: the type comes from them, not
    /// from what the server declares.
    pub async fn download_image(
        &self,
        file_id: &str,
    ) -> Result<(Vec<u8>, MimeType), TelegramError> {
        const METHOD: &str = "downloadFile";
        let file: File = self
            .call("getFile", json!({ "file_id": file_id }), SLACK)
            .await?;
        if file
            .file_size
            .is_some_and(|size| size > MAX_IMAGE_BYTES as u64)
        {
            return Err(Self::failed(
                METHOD,
                format!("the file is over the {MAX_IMAGE_BYTES}-byte limit"),
            ));
        }
        let path = file
            .file_path
            .filter(|path| is_file_path(path))
            .ok_or_else(|| Self::failed(METHOD, "getFile returned no usable file_path"))?;
        let url = format!("{}/file/bot{}/{path}", self.base, self.token);
        let mut response = self
            .http
            .get(url)
            .timeout(DOWNLOAD_TIMEOUT)
            .send()
            .await
            .map_err(|error| Self::unsent(METHOD, error))?;
        if !response.status().is_success() {
            return Err(TelegramError {
                method: METHOD,
                code: Some(i64::from(response.status().as_u16())),
                description: "the download failed".into(),
                retry_after: None,
            });
        }
        let too_big = || {
            Self::failed(
                METHOD,
                format!("the file is over the {MAX_IMAGE_BYTES}-byte limit"),
            )
        };
        if response
            .content_length()
            .is_some_and(|length| length > MAX_IMAGE_BYTES as u64)
        {
            return Err(too_big());
        }
        let mut data = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| Self::unsent(METHOD, error))?
        {
            if data.len() + chunk.len() > MAX_IMAGE_BYTES {
                return Err(too_big());
            }
            data.extend_from_slice(&chunk);
        }
        let mime_type =
            sniff(&data).ok_or_else(|| Self::failed(METHOD, "the file is not a picture"))?;
        Ok((data, mime_type))
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

    /// A photo or a clip, with `caption` when given, threaded under
    /// `reply_to` and into topic `thread` when given.
    pub async fn send_media(
        &self,
        chat_id: i64,
        media: &StoredMedia,
        caption: Option<&str>,
        reply_to: Option<i64>,
        thread: Option<i64>,
    ) -> Result<i64, TelegramError> {
        let (method, field, file) = match media.kind {
            MediaKind::Video => ("sendVideo", "video", "clip.mp4"),
            _ => ("sendPhoto", "photo", "picture.png"),
        };
        let part = reqwest::multipart::Part::bytes(media.data.clone())
            .file_name(file)
            .mime_str(media.mime_type.as_str())
            .map_err(|error| Self::unsent(method, error))?;
        let mut form = reqwest::multipart::Form::new()
            .text("chat_id", chat_id.to_string())
            .part(field, part);
        if let Some(caption) = caption {
            form = form.text("caption", caption.to_owned());
        }
        if let Some(reply_to) = reply_to {
            form = form.text(
                "reply_parameters",
                json!({ "message_id": reply_to, "allow_sending_without_reply": true }).to_string(),
            );
        }
        if let Some(thread) = thread {
            form = form.text("message_thread_id", thread.to_string());
        }
        let response = self
            .http
            .post(self.url(method))
            .timeout(Duration::from_secs(120))
            .multipart(form)
            .send()
            .await
            .map_err(|error| Self::unsent(method, error))?;
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
        Ok(TelegramApi::send_media(self, chat_id, media, caption, None, None).await?)
    }
}

#[cfg(test)]
#[path = "api_tests.rs"]
mod tests;
