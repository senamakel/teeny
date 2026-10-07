//! What Tiny looks like, in words an image model can follow: the reference
//! sheet, its cache, and the prompts it goes in front of.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::Mutex;

pub use teeny_core::config::DEFAULT_REFERENCE_IMAGES;

use crate::error::{Error, Result};
use crate::http::{endpoint, error_detail, json_body};
use crate::types::{ImageInput, ImageReader, MimeType};
use crate::vision::sniff;

const LOOK_QUESTION: &str = "This is official artwork of a character called Tiny. Describe Tiny for an illustrator \
who has to draw the same character again without seeing this: body shape and \
proportions, head and face, eyes, colours with specific names, materials and texture, \
clothing or markings, anything always present. Then the art style: rendering, line, \
palette, lighting, mood. Say what is in the scene only in one short clause; the \
character and the style are what matter. Be concrete and dense; no preamble.";

const SHEET_INSTRUCTION: &str = "Below are several descriptions of the same character, Tiny, each written from a \
different official picture. Write ONE character sheet for an image model, 120 to 180 \
words, that captures what is consistent across them: the body, the face, the colours \
(named precisely), the materials, the proportions, and the art style. Leave out \
anything that appears in only one picture, and leave out every scene. Write it as a \
plain paragraph starting \"Tiny is\", present tense, no headings, no bullets, nothing \
before or after the paragraph.";

const SHEET_SYSTEM: &str = "You write terse, concrete visual briefs for image models.";

const TEXT_TIMEOUT: Duration = Duration::from_secs(90);

/// A reference is a URL or a path on disk; the reader takes either shape.
pub fn reference_input(reference: &str) -> Result<ImageInput> {
    let lower = reference.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return Ok(ImageInput::Url(reference.to_owned()));
    }
    let data = std::fs::read(reference)?;
    let by_extension = || {
        let extension = Path::new(reference).extension()?.to_str()?.to_ascii_lowercase();
        match extension.as_str() {
            "png" => Some(MimeType::Png),
            "jpg" | "jpeg" => Some(MimeType::Jpeg),
            "webp" => Some(MimeType::Webp),
            "gif" => Some(MimeType::Gif),
            _ => None,
        }
    };
    let Some(mime_type) = sniff(&data).or_else(by_extension) else {
        return Err(Error::failed(format!("{reference} is not an image this can read")));
    };
    Ok(ImageInput::Bytes { data, mime_type })
}

/// A plain chat completion: one system line, one prompt, one answer.
#[async_trait]
pub trait TextModel: Send + Sync {
    async fn complete(&self, system: Option<&str>, prompt: &str) -> Result<String>;
}

/// A plain chat completion on an OpenAI-compatible endpoint, for the one text
/// step that distils the looks into a sheet.
#[derive(Debug, Clone)]
pub struct OpenAiTextModel {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
    temperature: f64,
}

impl OpenAiTextModel {
    /// A model on this endpoint at temperature 0.2: the sheet wants the same
    /// answer twice.
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.into(),
            api_key: api_key.into(),
            model: model.into(),
            temperature: 0.2,
        }
    }

    /// Sampling temperature.
    pub fn with_temperature(mut self, temperature: f64) -> Self {
        self.temperature = temperature;
        self
    }
}

#[async_trait]
impl TextModel for OpenAiTextModel {
    async fn complete(&self, system: Option<&str>, prompt: &str) -> Result<String> {
        let mut messages = Vec::new();
        if let Some(system) = system.filter(|system| !system.is_empty()) {
            messages.push(json!({ "role": "system", "content": system }));
        }
        messages.push(json!({ "role": "user", "content": prompt }));
        let response = self
            .client
            .post(endpoint(&self.base_url, "/chat/completions"))
            .bearer_auth(&self.api_key)
            .timeout(TEXT_TIMEOUT)
            .json(&json!({ "model": self.model, "temperature": self.temperature, "messages": messages }))
            .send()
            .await?;
        let status = response.status();
        let body = json_body(response).await;
        if !status.is_success() {
            return Err(Error::failed(format!(
                "text request failed ({}): {}",
                status.as_u16(),
                error_detail(&body)
            )));
        }
        let text = body.pointer("/choices/0/message/content").and_then(Value::as_str).map(str::trim).unwrap_or("");
        if text.is_empty() {
            return Err(Error::failed("the text model returned nothing"));
        }
        Ok(text.to_owned())
    }
}

/// One of the reference pictures, and what the vision model saw in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceSeed {
    /// The path or URL the picture was read from.
    pub url: String,
    /// The illustrator's description of that one picture.
    pub look: String,
}

/// The cache file: `data/media/reference-sheet.json`, as the legacy service
/// wrote it.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CachedSheet {
    urls: Vec<String>,
    sheet: String,
    #[serde(default)]
    looks: BTreeMap<String, String>,
    #[serde(default)]
    created_at: String,
}

#[derive(Default)]
struct Built {
    done: bool,
    sheet: Option<String>,
    looks: BTreeMap<String, String>,
}

/// What Tiny looks like, in words an image model can follow.
///
/// The image ladder is text-to-image only, so the artwork cannot be handed to
/// the generator as pixels. It can be handed to the vision model, which
/// describes each picture for an illustrator; a text step then distils the
/// descriptions into one paragraph of what is constant across them. That
/// paragraph is prepended to every generation prompt that features Tiny.
///
/// Built once and cached on disk, keyed by the URL list: vision calls cost
/// money and the artwork does not change between restarts. Every failure is a
/// missing sheet rather than a failed generation, and is not remembered: the
/// next caller tries again.
pub struct ReferenceSheet {
    urls: Vec<String>,
    reader: Arc<dyn ImageReader>,
    text: Arc<dyn TextModel>,
    cache_path: Option<PathBuf>,
    built: Mutex<Built>,
}

impl std::fmt::Debug for ReferenceSheet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReferenceSheet")
            .field("urls", &self.urls)
            .field("cache_path", &self.cache_path)
            .finish_non_exhaustive()
    }
}

impl ReferenceSheet {
    /// A sheet from these pictures (paths on disk or https URLs).
    pub fn new(urls: Vec<String>, reader: Arc<dyn ImageReader>, text: Arc<dyn TextModel>) -> Self {
        Self { urls, reader, text, cache_path: None, built: Mutex::new(Built::default()) }
    }

    /// Where the sheet is cached (`REFERENCE_SHEET_PATH`).
    pub fn with_cache_path(mut self, path: impl AsRef<Path>) -> Self {
        self.cache_path = Some(path.as_ref().to_path_buf());
        self
    }

    /// The sheet, built on first call and remembered; `None` when it could not
    /// be built. Concurrent callers wait for the one build.
    pub async fn get(&self) -> Option<String> {
        let mut built = self.built.lock().await;
        if built.done {
            return built.sheet.clone();
        }
        match self.build().await {
            Ok((sheet, looks)) => {
                *built = Built { done: true, sheet: sheet.clone(), looks };
                sheet
            }
            Err(error) => {
                tracing::warn!(err = %error, "reference sheet failed");
                None
            }
        }
    }

    /// Starts the build without waiting for it, so the first generation does
    /// not pay for it.
    pub fn warm(self: &Arc<Self>) {
        let sheet = Arc::clone(self);
        tokio::spawn(async move {
            sheet.get().await;
        });
    }

    /// One reference picture, chosen by `random` (in `[0, 1)`), with its own
    /// description.
    ///
    /// The sheet is what is constant across the artwork; a seed is one picture
    /// in particular, and a generation that starts from one comes out looking
    /// like a sibling of a published picture rather than a generic rendering
    /// of the sheet. `None` until the sheet has been built, or when no picture
    /// could be described.
    pub async fn seed(&self, random: f64) -> Option<ReferenceSeed> {
        self.get().await;
        let built = self.built.lock().await;
        let entries: Vec<(&String, &String)> = built.looks.iter().filter(|(_, look)| !look.trim().is_empty()).collect();
        if entries.is_empty() {
            return None;
        }
        let index = ((random * entries.len() as f64).floor().max(0.0) as usize).min(entries.len() - 1);
        let (url, look) = entries[index];
        Some(ReferenceSeed { url: url.clone(), look: look.clone() })
    }

    async fn build(&self) -> Result<(Option<String>, BTreeMap<String, String>)> {
        let mut urls: Vec<String> = Vec::new();
        for url in &self.urls {
            if !urls.contains(url) {
                urls.push(url.clone());
            }
        }
        if urls.is_empty() {
            return Ok((None, BTreeMap::new()));
        }

        if let Some(cached) = self.read_cache().filter(|cached| same_urls(&cached.urls, &urls)) {
            tracing::info!(urls = urls.len(), "reference sheet loaded from cache");
            return Ok((Some(cached.sheet), cached.looks));
        }

        let described = futures::future::join_all(urls.iter().map(|url| async move {
            let look = match reference_input(url) {
                Ok(input) => self.reader.describe(input, Some(LOOK_QUESTION)).await,
                Err(error) => Err(error),
            };
            (url, look)
        }))
        .await;
        let mut looks = BTreeMap::new();
        let mut pictures = Vec::new();
        for (url, look) in described {
            match look {
                Ok(look) => {
                    pictures.push(look.clone());
                    looks.insert(url.clone(), look);
                }
                Err(error) => tracing::warn!(url = %url, err = %error, "could not look at a reference image"),
            }
        }
        if pictures.is_empty() {
            return Err(Error::failed("none of the reference images could be described"));
        }

        let numbered: Vec<String> =
            pictures.iter().enumerate().map(|(index, look)| format!("PICTURE {}:\n{look}", index + 1)).collect();
        let sheet = self
            .text
            .complete(Some(SHEET_SYSTEM), &format!("{SHEET_INSTRUCTION}\n\n{}", numbered.join("\n\n")))
            .await?;
        let trimmed = sheet.split_whitespace().collect::<Vec<_>>().join(" ");
        if !trimmed.to_lowercase().starts_with("tiny is") {
            tracing::info!("reference sheet did not start with \"Tiny is\"; keeping it anyway");
        }
        self.write_cache(&CachedSheet {
            urls,
            sheet: trimmed.clone(),
            looks: looks.clone(),
            created_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        });
        tracing::info!(pictures = pictures.len(), words = trimmed.split(' ').count(), "reference sheet built");
        Ok((Some(trimmed), looks))
    }

    fn read_cache(&self) -> Option<CachedSheet> {
        let text = std::fs::read_to_string(self.cache_path.as_ref()?).ok()?;
        // Absent or unreadable: build it again.
        serde_json::from_str::<CachedSheet>(&text).ok().filter(|cached| !cached.sheet.trim().is_empty())
    }

    fn write_cache(&self, sheet: &CachedSheet) {
        let Some(path) = &self.cache_path else {
            return;
        };
        let written = serde_json::to_string_pretty(sheet).map_err(std::io::Error::other).and_then(|text| {
            if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(path, text)
        });
        if let Err(error) = written {
            tracing::warn!(err = %error, "could not cache the reference sheet");
        }
    }
}

fn same_urls(a: &[String], b: &[String]) -> bool {
    let mut a = a.to_vec();
    let mut b = b.to_vec();
    a.sort();
    b.sort();
    a == b
}

/// The generation prompt with Tiny's look in front of the scene.
pub fn with_reference(sheet: Option<&str>, prompt: &str) -> String {
    match sheet.filter(|sheet| !sheet.is_empty()) {
        Some(sheet) => format!("{sheet}\n\nScene: {prompt}"),
        None => prompt.to_owned(),
    }
}

/// The generation prompt with the sheet and one seed picture in front of the
/// scene: what Tiny always looks like, then the particular picture today's
/// generation should look like a sibling of, then the scene. Over
/// `max_length` (in UTF-16 units, as the legacy limit was counted) the seed is
/// the part to lose: the sheet is what makes it Tiny and the scene is what
/// makes it today.
pub fn with_seed(sheet: Option<&str>, seed: Option<&ReferenceSeed>, prompt: &str, max_length: Option<usize>) -> String {
    let Some(seed) = seed else {
        return fit(with_reference(sheet, prompt), max_length);
    };
    let mut parts: Vec<String> = Vec::new();
    if let Some(sheet) = sheet.filter(|sheet| !sheet.is_empty()) {
        parts.push(sheet.to_owned());
    }
    parts.push(format!(
        "Style reference, one of Tiny's official pictures (match its rendering, palette and mood, not its scene): {}",
        seed.look
    ));
    parts.push(format!("Scene: {prompt}"));
    let composed = parts.join("\n\n");
    match max_length {
        Some(max) if max > 0 && composed.encode_utf16().count() > max => fit(with_reference(sheet, prompt), max_length),
        _ => composed,
    }
}

/// Under the limit, cut at a sentence when one is near enough.
fn fit(prompt: String, max_length: Option<usize>) -> String {
    let units: Vec<u16> = prompt.encode_utf16().collect();
    let Some(max) = max_length.filter(|max| *max > 0 && units.len() > *max) else {
        return prompt;
    };
    let head = &units[..max];
    let stop = head.windows(2).rposition(|pair| pair == [u16::from(b'.'), u16::from(b' ')]);
    let cut = match stop {
        Some(stop) if stop as f64 > max as f64 * 0.7 => &head[..=stop],
        _ => head,
    };
    String::from_utf16_lossy(cut)
}

#[cfg(test)]
#[path = "reference_tests.rs"]
mod tests;
