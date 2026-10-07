//! An episode, sent to the serial's Telegram chats after it went out on X:
//! a send-only Bot API client (`sendPhoto`, `sendVideo`). Receiving is the
//! OpenHuman channel's job; this only sends, and only an episode that was
//! really posted.

use async_trait::async_trait;
use serde::Deserialize;
use teeny_core::Secret;

#[async_trait]
pub trait EpisodeChannel: Send + Sync {
    /// Sends the episode's media with its caption; the message id.
    async fn send(
        &self,
        chat_id: i64,
        data: &[u8],
        mime_type: &str,
        video: bool,
        caption: &str,
    ) -> anyhow::Result<i64>;
}

pub struct BotApi {
    http: reqwest::Client,
    base: String,
    token: Secret,
}

impl BotApi {
    pub fn new(token: Secret) -> Self {
        Self {
            http: reqwest::Client::new(),
            base: "https://api.telegram.org".into(),
            token,
        }
    }

    pub fn with_base_url(mut self, base: impl Into<String>) -> Self {
        self.base = base.into().trim_end_matches('/').to_owned();
        self
    }
}

#[derive(Deserialize)]
struct Answer {
    ok: bool,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    result: Option<Message>,
}

#[derive(Deserialize)]
struct Message {
    message_id: i64,
}

#[async_trait]
impl EpisodeChannel for BotApi {
    async fn send(
        &self,
        chat_id: i64,
        data: &[u8],
        mime_type: &str,
        video: bool,
        caption: &str,
    ) -> anyhow::Result<i64> {
        let (method, field, file) = if video {
            ("sendVideo", "video", "episode.mp4")
        } else {
            ("sendPhoto", "photo", "episode.png")
        };
        let part = reqwest::multipart::Part::bytes(data.to_vec())
            .file_name(file)
            .mime_str(mime_type)?;
        let form = reqwest::multipart::Form::new()
            .text("chat_id", chat_id.to_string())
            .text("caption", caption.to_owned())
            .part(field, part);
        let answer: Answer = self
            .http
            .post(format!("{}/bot{}/{method}", self.base, self.token.expose()))
            .multipart(form)
            .send()
            .await?
            .json()
            .await?;
        match (answer.ok, answer.result) {
            (true, Some(message)) => Ok(message.message_id),
            _ => anyhow::bail!(
                "telegram refused the episode: {}",
                answer.description.unwrap_or_else(|| "no reason given".into())
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    #[tokio::test]
    async fn sends_a_clip_with_its_caption() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/botT/sendVideo"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "ok": true, "result": { "message_id": 77 } })))
            .mount(&server)
            .await;
        let api = BotApi::new(Secret::new("T")).with_base_url(server.uri());
        assert_eq!(api.send(-100, &[1], "video/mp4", true, "the door moved").await.unwrap(), 77);
        let body = String::from_utf8_lossy(&server.received_requests().await.unwrap()[0].body).into_owned();
        assert!(body.contains("the door moved"));
        assert!(body.contains("-100"));
    }
}
