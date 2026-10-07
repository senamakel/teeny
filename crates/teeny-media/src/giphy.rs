//! Giphy search, scoped to one channel by default: the data side of
//! `find_gif`, and the lookup and download behind attaching a GIF to a post.

use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;
use url::Url;

use crate::error::{Error, Result};
use crate::http::encode_component;
use crate::types::MimeType;

/// X rejects an animated GIF over 15 MB, so the largest usable rendition wins.
pub const MAX_UPLOAD_BYTES: usize = 15 * 1024 * 1024;

const RENDITIONS: [&str; 4] = ["downsized_medium", "downsized", "original", "fixed_height"];
const DEFAULT_BASE_URL: &str = "https://api.giphy.com";
const API_TIMEOUT: Duration = Duration::from_secs(15);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30);

/// A GIF found on Giphy, with a rendition small enough for an X upload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gif {
    pub id: String,
    /// Giphy's title: untrusted text.
    pub title: String,
    pub page_url: String,
    /// Direct media URL, small enough for an X upload.
    pub media_url: String,
    pub mime_type: MimeType,
}

/// Somewhere GIFs come from.
#[async_trait]
pub trait GifSource: Send + Sync {
    /// GIFs matching the query, from the account's channel unless
    /// `channel_only` is false.
    async fn search(&self, query: &str, channel_only: bool) -> Result<Vec<Gif>>;
    /// One GIF by id; `None` when the source does not know it.
    async fn get(&self, id: &str) -> Result<Option<Gif>>;
    /// The GIF's bytes, ready to upload.
    async fn download(&self, gif: &Gif) -> Result<(Vec<u8>, MimeType)>;
}

/// Giphy's public v1 API.
///
/// Giphy has no by-username endpoint; a channel is searched by putting
/// `@username` in the query, which is what the website does too.
#[derive(Debug, Clone)]
pub struct GiphyClient {
    client: reqwest::Client,
    api_key: String,
    channel: String,
    limit: u32,
    base_url: String,
}

impl GiphyClient {
    /// A client searching this channel (`GIPHY_CHANNEL`), ten results a search.
    pub fn new(api_key: impl Into<String>, channel: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key: api_key.into(),
            channel: channel.into(),
            limit: 10,
            base_url: DEFAULT_BASE_URL.to_owned(),
        }
    }

    /// Results per search.
    pub fn with_limit(mut self, limit: u32) -> Self {
        self.limit = limit;
        self
    }

    /// Another host speaking Giphy's API, e.g. a caching proxy.
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    fn url(&self, path: &str, query: &[(&str, &str)]) -> Result<Url> {
        let mut url = Url::parse(&format!("{}{path}", self.base_url.trim_end_matches('/')))
            .map_err(|error| Error::failed(format!("bad Giphy URL: {error}")))?;
        url.query_pairs_mut().append_pair("api_key", &self.api_key).extend_pairs(query);
        Ok(url)
    }
}

#[async_trait]
impl GifSource for GiphyClient {
    async fn search(&self, query: &str, channel_only: bool) -> Result<Vec<Gif>> {
        let channel = self.channel.strip_prefix('@').unwrap_or(&self.channel);
        let scoped = if channel_only { format!("@{channel} {query}").trim().to_owned() } else { query.to_owned() };
        let limit = self.limit.to_string();
        let url = self.url(
            "/v1/gifs/search",
            &[("q", &scoped), ("limit", &limit), ("rating", "g"), ("bundle", "messaging_non_clips")],
        )?;
        let response = self.client.get(url).timeout(API_TIMEOUT).send().await?;
        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(Error::failed(format!("Giphy search failed ({}): {text}", status.as_u16())));
        }
        let body: Value = response.json().await?;
        let found = body.get("data").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
        Ok(found.iter().filter_map(to_gif).collect())
    }

    async fn get(&self, id: &str) -> Result<Option<Gif>> {
        let url = self.url(&format!("/v1/gifs/{}", encode_component(id)), &[])?;
        let response = self.client.get(url).timeout(API_TIMEOUT).send().await?;
        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(Error::failed(format!("Giphy lookup failed ({}): {text}", status.as_u16())));
        }
        let body: Value = response.json().await?;
        Ok(body.get("data").and_then(to_gif))
    }

    async fn download(&self, gif: &Gif) -> Result<(Vec<u8>, MimeType)> {
        let response = self.client.get(&gif.media_url).timeout(DOWNLOAD_TIMEOUT).send().await?;
        if !response.status().is_success() {
            return Err(Error::failed(format!(
                "Giphy download failed ({}) for {}",
                response.status().as_u16(),
                gif.media_url
            )));
        }
        let data = response.bytes().await?.to_vec();
        if data.len() > MAX_UPLOAD_BYTES {
            return Err(Error::failed(format!(
                "GIF {} is {} bytes, over the X upload limit",
                gif.id,
                data.len()
            )));
        }
        Ok((data, gif.mime_type))
    }
}

/// `Number(size)`: Giphy sends sizes as strings, sometimes as numbers.
fn size_of(image: &Value) -> f64 {
    match image.get("size") {
        Some(Value::String(text)) if text.trim().is_empty() => 0.0,
        Some(Value::String(text)) => text.trim().parse().unwrap_or(f64::NAN),
        Some(Value::Number(number)) => number.as_f64().unwrap_or(f64::NAN),
        _ => 0.0,
    }
}

/// The first rendition, largest first, that X will take.
fn pick_rendition(gif: &Value) -> Option<String> {
    let images = gif.get("images")?;
    RENDITIONS.iter().find_map(|name| {
        let image = images.get(*name)?;
        let url = image.get("url").and_then(Value::as_str).filter(|url| !url.is_empty())?;
        let size = size_of(image);
        (size > 0.0 && size <= MAX_UPLOAD_BYTES as f64).then(|| url.to_owned())
    })
}

fn to_gif(gif: &Value) -> Option<Gif> {
    let media_url = pick_rendition(gif)?;
    let id = gif.get("id").and_then(Value::as_str).filter(|id| !id.is_empty())?;
    let title = gif.get("title").and_then(Value::as_str).map(str::trim).filter(|title| !title.is_empty());
    let page_url = gif
        .get("url")
        .and_then(Value::as_str)
        .map_or_else(|| format!("https://giphy.com/gifs/{id}"), str::to_owned);
    Some(Gif {
        id: id.to_owned(),
        title: title.unwrap_or("(untitled)").to_owned(),
        page_url,
        media_url,
        mime_type: MimeType::Gif,
    })
}

/// What `find_gif` tells the model: each GIF's id, title and page, or why
/// there were none and what to try next. `channel` is the account's own
/// (`GIPHY_CHANNEL`), named in the answer when nothing in it matched.
pub async fn find_gif(source: &dyn GifSource, channel: &str, query: &str, search_all: bool) -> Result<String> {
    let results = source.search(query, !search_all).await?;
    if results.is_empty() {
        return Ok(if search_all {
            format!("No GIF matched \"{query}\".")
        } else {
            format!(
                "No GIF in the @{channel} channel matched \"{query}\". Try another wording, or set searchAllOfGiphy to look beyond the channel."
            )
        });
    }
    Ok(results
        .iter()
        .map(|gif| format!("id: {}\ntitle: {}\npage: {}", gif.id, gif.title, gif.page_url))
        .collect::<Vec<_>>()
        .join("\n---\n"))
}

#[cfg(test)]
#[path = "giphy_tests.rs"]
mod tests;
