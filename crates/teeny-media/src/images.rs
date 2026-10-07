//! Image generation through an OpenAI-shaped `/images/generations` endpoint:
//! the LLM ladder's `image` ladder, `gpt-image-*` or `dall-e-*` directly, or
//! anything else that answers that route. One image per call, always.

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use crate::error::{Error, Result};
use crate::http::{decode_base64_lenient, endpoint, error_detail, json_body};
use crate::types::{GeneratedImage, ImageAspect, ImageGenerator, MimeType};
use crate::vision::sniff;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(180);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    GptImage,
    DallE,
    Other,
}

fn family_of(model: &str) -> Family {
    if model.starts_with("dall-e") {
        Family::DallE
    } else if model.starts_with("gpt-image") {
        Family::GptImage
    } else {
        Family::Other
    }
}

/// The size string for an aspect. Square is left to the endpoint's default,
/// so a square request carries no size at all and can never name one a rung
/// does not know. The two OpenAI families disagree on their non-square sizes
/// and each rejects the other's; anything else gets the gpt-image spelling,
/// which is the one most resellers copy.
fn size_for(family: Family, aspect: ImageAspect) -> Option<&'static str> {
    match aspect {
        ImageAspect::Landscape if family == Family::DallE => Some("1792x1024"),
        ImageAspect::Landscape => Some("1536x1024"),
        ImageAspect::Portrait if family == Family::DallE => Some("1024x1792"),
        ImageAspect::Portrait => Some("1024x1536"),
        ImageAspect::Square => None,
    }
}

/// Image generation on an OpenAI-shaped endpoint. The bytes come back as
/// base64 or as a URL, and as PNG, JPEG or WebP depending on which rung
/// served, so the type is read from the bytes rather than assumed.
#[derive(Debug, Clone)]
pub struct OpenAiImageGenerator {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
    quality: Option<String>,
    timeout: Duration,
}

impl OpenAiImageGenerator {
    /// A generator on this endpoint and model, with the legacy 180 s timeout.
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.into(),
            api_key: api_key.into(),
            model: model.into(),
            quality: None,
            timeout: DEFAULT_TIMEOUT,
        }
    }

    /// `low`, `medium`, `high` for gpt-image; `standard`/`hd` for dall-e-3.
    /// Never sent to any other model.
    pub fn with_quality(mut self, quality: impl Into<String>) -> Self {
        self.quality = Some(quality.into()).filter(|quality| !quality.is_empty());
        self
    }

    /// How long one generation may take.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

#[async_trait]
impl ImageGenerator for OpenAiImageGenerator {
    async fn generate(&self, prompt: &str, aspect: ImageAspect) -> Result<GeneratedImage> {
        let family = family_of(&self.model);
        let mut body = Map::new();
        body.insert("model".into(), json!(self.model));
        body.insert("prompt".into(), json!(prompt));
        body.insert("n".into(), json!(1));
        if let Some(size) = size_for(family, aspect) {
            body.insert("size".into(), json!(size));
        }
        // Only OpenAI's own families take these; on a ladder of many models an
        // unknown parameter is a 400 that does not advance it.
        if let Some(quality) = self.quality.as_ref().filter(|_| family != Family::Other) {
            body.insert("quality".into(), json!(quality));
        }
        match family {
            Family::DallE => {
                body.insert("response_format".into(), json!("b64_json"));
            }
            Family::GptImage => {
                body.insert("output_format".into(), json!("png"));
            }
            Family::Other => {}
        }

        let response = self
            .client
            .post(endpoint(&self.base_url, "/images/generations"))
            .bearer_auth(&self.api_key)
            .timeout(self.timeout)
            .json(&Value::Object(body))
            .send()
            .await?;
        let status = response.status();
        let answer = json_body(response).await;
        if !status.is_success() {
            return Err(Error::failed(format!(
                "image generation failed ({}): {}",
                status.as_u16(),
                error_detail(&answer)
            )));
        }
        let first = answer.pointer("/data/0");
        let field = |name: &str| {
            first.and_then(|first| first.get(name)).and_then(Value::as_str).filter(|value| !value.is_empty())
        };
        if let Some(encoded) = field("b64_json") {
            return typed(decode_base64_lenient(encoded));
        }
        if let Some(url) = field("url") {
            let download = self.client.get(url).timeout(DOWNLOAD_TIMEOUT).send().await?;
            if !download.status().is_success() {
                return Err(Error::failed(format!("image download failed ({})", download.status().as_u16())));
            }
            return typed(download.bytes().await?.to_vec());
        }
        Err(Error::failed("image generation returned no image"))
    }
}

fn typed(data: Vec<u8>) -> Result<GeneratedImage> {
    match sniff(&data) {
        Some(mime_type @ (MimeType::Png | MimeType::Jpeg | MimeType::Webp)) => Ok(GeneratedImage { data, mime_type }),
        Some(other) => Err(Error::failed(format!("image generation returned {}", other.as_str()))),
        None => Err(Error::failed("image generation returned something that is not an image")),
    }
}

#[cfg(test)]
#[path = "images_tests.rs"]
mod tests;
