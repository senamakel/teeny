//! What a visitor actually looks like, for a generator that only takes
//! words: a photograph found on the web, described by the vision rung as a
//! casting note, kept per person. The photograph's page is untrusted; only
//! the reader's description of the picture is used, and only in an image
//! prompt.

use std::path::PathBuf;
use std::sync::{Arc, LazyLock};

use async_trait::async_trait;
use chrono::Utc;
use regex::Regex;
use serde::{Deserialize, Serialize};
use teeny_media::types::{ImageInput, ImageReader};
use teeny_media::vision::is_fetchable_image_url;

use crate::backrooms::sources::{WebQuery, WebSearch};
use crate::backrooms::text::{flatten, head};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Likeness {
    pub name: String,
    pub description: String,
    pub photo_url: String,
    pub written_at: String,
}

#[async_trait]
pub trait LikenessSource: Send + Sync {
    async fn find(&self, name: &str) -> anyhow::Result<Option<Likeness>>;
}

const QUESTION: &str = "Describe the person in this photograph in 40 to 70 words so an illustrator could draw them without seeing it: face shape, hair (colour, length, how it sits), eyes and brows, skin tone, build and height impression, the clothes they are wearing here, and one thing about how they hold themselves. If more than one person is in the photo, describe only the one most prominently in the centre. Plain descriptive words, present tense, no name, no opinions. Answer with the description only.";

pub struct LikenessFinder {
    pub web: Arc<dyn WebSearch>,
    pub reader: Arc<dyn ImageReader>,
    pub cache_dir: Option<PathBuf>,
}

static IMAGE_EXTENSION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\.(jpe?g|png|webp)(\?|$)").expect("valid"));
static SEPARATORS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[-_/.]+").expect("valid"));

impl LikenessFinder {
    fn path(&self, name: &str) -> Option<PathBuf> {
        let slug = crate::backrooms::store::slugify(name, 80);
        self.cache_dir.as_ref().map(|dir| dir.join(format!("{slug}.json")))
    }

    /// A picture whose own URL names the person first, then any picture on
    /// a page that names them, then the first picture.
    async fn photo(&self, name: &str) -> anyhow::Result<Option<String>> {
        let hits = self
            .web
            .search(&WebQuery {
                query: format!("{name} photo portrait"),
                num_results: 8,
                image_links: 3,
                ..WebQuery::default()
            })
            .await?;
        let parts: Vec<String> = name.trim().to_lowercase().split_whitespace().map(str::to_owned).collect();
        let names = |text: &str| {
            let text = text.to_lowercase();
            parts.iter().all(|part| text.contains(part.as_str()))
        };
        let mut candidates: Vec<(String, String, String)> = Vec::new();
        for hit in &hits {
            for raw in hit.image.iter().chain(&hit.image_links) {
                let url = raw.replace("&amp;", "&");
                if is_fetchable_image_url(&url) && IMAGE_EXTENSION.is_match(&url) {
                    candidates.push((url, hit.title.clone().unwrap_or_default(), hit.url.clone()));
                }
            }
        }
        let by_url = candidates.iter().find(|(url, _, _)| names(&SEPARATORS.replace_all(url, " ")));
        let by_page = candidates
            .iter()
            .find(|(_, title, page)| names(title) || names(&SEPARATORS.replace_all(page, " ")));
        Ok(by_url.or(by_page).or(candidates.first()).map(|(url, _, _)| url.clone()))
    }
}

#[async_trait]
impl LikenessSource for LikenessFinder {
    async fn find(&self, name: &str) -> anyhow::Result<Option<Likeness>> {
        let path = self.path(name);
        if let Some(path) = &path
            && let Ok(text) = tokio::fs::read_to_string(path).await
            && let Ok(cached) = serde_json::from_str::<Likeness>(&text)
        {
            return Ok(Some(cached));
        }
        let Some(photo_url) = self.photo(name).await? else {
            return Ok(None);
        };
        let description = flatten(
            &self
                .reader
                .describe(ImageInput::Url(photo_url.clone()), Some(QUESTION))
                .await?,
        );
        if description.chars().count() < 20 {
            return Ok(None);
        }
        let likeness = Likeness {
            name: name.to_owned(),
            description: head(&description, 600),
            photo_url,
            written_at: Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        };
        if let (Some(path), Some(dir)) = (&path, &self.cache_dir) {
            tokio::fs::create_dir_all(dir).await?;
            tokio::fs::write(path, format!("{}\n", serde_json::to_string_pretty(&likeness)?)).await?;
        }
        tracing::info!(name, photo = %likeness.photo_url, "narrative: wrote a likeness");
        Ok(Some(likeness))
    }
}

/// The visitor's name in a still or shot list, followed once by what they look like.
pub fn with_likeness(text: &str, visitor: &str, likeness: &Likeness) -> String {
    let Ok(pattern) = Regex::new(&format!("(?i){}", regex::escape(visitor.trim()))) else {
        return text.to_owned();
    };
    if !pattern.is_match(text) {
        return format!("{visitor} ({}) is in the scene. {text}", likeness.description);
    }
    pattern
        .replacen(text, 1, |c: &regex::Captures<'_>| format!("{} ({})", &c[0], likeness.description))
        .into_owned()
}

/// The still or shot list with the visitor's name replaced by the stand-in
/// once, and "the visitor" after, for a model that will not draw them.
pub fn swap_visitor(text: &str, name: &str, stand_in: &str) -> String {
    let name = name.trim();
    if name.is_empty() {
        return text.to_owned();
    }
    let Ok(pattern) = Regex::new(&format!("(?i){}", regex::escape(name))) else {
        return text.to_owned();
    };
    let mut first = true;
    pattern
        .replace_all(text, |_: &regex::Captures<'_>| {
            if std::mem::take(&mut first) {
                stand_in.to_owned()
            } else {
                "the visitor".to_owned()
            }
        })
        .into_owned()
}

#[cfg(test)]
#[path = "likeness_tests.rs"]
mod tests;
