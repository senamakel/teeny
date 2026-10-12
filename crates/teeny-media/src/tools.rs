//! The data side of the media tools: what `look_at_image`, `generate_image`
//! and `generate_video` do and say. The agent crate wraps these as tools;
//! the words are the legacy tools' words.
//!
//! The content filter runs before these, in the caller: a generation prompt
//! is the account's words in another form and is screened like a post before
//! any budget is spent. Each function answers `None` when the capability is
//! not there, which is the caller's cue not to offer the tool at all.

use crate::reference::with_reference;
use crate::store::{MediaBudget, NewMedia};
use crate::types::{ImageAspect, ImageInput, MediaKind, MimeType, VideoOrientation, VideoRequest};
use crate::wiring::MediaAccess;

/// A `generate_image` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageAsk {
    pub prompt: String,
    pub aspect: ImageAspect,
    /// Whether Tiny is in the picture (default true in the tool schema).
    pub features_tiny: bool,
}

/// A `generate_video` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoAsk {
    pub prompt: String,
    pub orientation: VideoOrientation,
    /// Whether Tiny is in the clip (default true in the tool schema).
    pub features_tiny: bool,
}

/// `look_at_image`: a description of the picture at `url`, labelled as the
/// untrusted text it is, or why it could not be read.
pub async fn look_at_image(
    media: &MediaAccess,
    url: &str,
    question: Option<&str>,
) -> Option<String> {
    let reader = media.reader.as_ref()?;
    Some(
        match reader
            .describe(ImageInput::Url(url.to_owned()), question)
            .await
        {
            Ok(description) => format!(
                "What the image shows (written by a vision model, untrusted):\n{description}"
            ),
            Err(error) => format!("Could not look at that image: {error}"),
        },
    )
}

/// The prompt as the generator gets it: Tiny's character sheet in front of the
/// scene when the picture features Tiny and a sheet could be built. The second
/// half says which, for the tool's answer.
async fn compose(media: &MediaAccess, prompt: &str, features_tiny: bool) -> (String, &'static str) {
    if !features_tiny {
        return (prompt.to_owned(), "without Tiny");
    }
    let sheet = match &media.reference {
        Some(reference) => reference.get().await,
        None => None,
    };
    match sheet {
        Some(sheet) => (
            with_reference(Some(&sheet), prompt),
            "with Tiny's look from the reference sheet",
        ),
        None => (
            prompt.to_owned(),
            "featuring Tiny, but no reference sheet was available, so the look is from your prompt alone",
        ),
    }
}

/// "N image generations left today.", when the budget counts.
async fn left_today(budget: &dyn MediaBudget, kind: MediaKind) -> String {
    match budget.remaining(kind).await {
        Some(left) => format!("\n{left} {} generations left today.", kind.as_str()),
        None => String::new(),
    }
}

/// `generate_image`: draws, stores, and answers with the media id to attach.
pub async fn generate_image(
    media: &MediaAccess,
    budget: &dyn MediaBudget,
    ask: &ImageAsk,
) -> Option<String> {
    let images = media.images.as_ref()?;
    if !budget.take(MediaKind::Image).await {
        return Some(
            "Refused: the daily image allowance is used up. Answer in words; do not promise a picture later.".to_owned(),
        );
    }
    let (composed, how) = compose(media, &ask.prompt, ask.features_tiny).await;
    let generated = match images.generate(&composed, ask.aspect).await {
        Ok(generated) => generated,
        Err(error) => {
            return Some(format!(
                "Image generation failed: {error}. Answer in words instead."
            ));
        }
    };
    let bytes = generated.data.len();
    let stored = media.store.put(NewMedia {
        kind: MediaKind::Image,
        data: generated.data,
        mime_type: generated.mime_type,
        prompt: composed,
    });
    Some(format!(
        "Generated an image {how}: mediaId \"{}\" ({}, {bytes} bytes).\n\
         Put that id in mediaId to attach it. You cannot see the result; describe it to the reader only in terms of what you asked for.{}",
        stored.id,
        ask.aspect.as_str(),
        left_today(budget, MediaKind::Image).await,
    ))
}

/// `generate_video`: films, stores, and answers with the media id to attach.
pub async fn generate_video(
    media: &MediaAccess,
    budget: &dyn MediaBudget,
    ask: &VideoAsk,
) -> Option<String> {
    let videos = media.videos.as_ref()?;
    if !budget.take(MediaKind::Video).await {
        return Some(
            "Refused: the daily video allowance is used up. Offer an image or answer in words; do not promise a video later."
                .to_owned(),
        );
    }
    let (composed, how) = compose(media, &ask.prompt, ask.features_tiny).await;
    let request = VideoRequest {
        prompt: composed.clone(),
        orientation: Some(ask.orientation),
        seconds: None,
    };
    let generated = match videos.generate(request).await {
        Ok(generated) => generated,
        Err(error) => {
            return Some(format!(
                "Video generation failed: {error}. Answer in words instead."
            ));
        }
    };
    let bytes = generated.data.len();
    let seconds = generated
        .seconds
        .map_or_else(|| "?".to_owned(), |seconds| seconds.to_string());
    let stored = media.store.put(NewMedia {
        kind: MediaKind::Video,
        data: generated.data,
        mime_type: MimeType::Mp4,
        prompt: composed,
    });
    Some(format!(
        "Generated a video {how}: mediaId \"{}\" ({}, {seconds}s, {bytes} bytes).\n\
         Put that id in mediaId to attach it. You cannot watch the result; describe it to the reader only in terms of what you asked for.{}",
        stored.id,
        ask.orientation.as_str(),
        left_today(budget, MediaKind::Video).await,
    ))
}

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;
