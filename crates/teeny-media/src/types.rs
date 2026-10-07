//! Media the account can look at and media it can make.
//!
//! Three capabilities, three traits, so each can be backed by whatever
//! provider answers and switched off independently: a text-only model on the
//! ladder means no vision, and an image key without a video key means the
//! account can draw but not film. Nothing here publishes.

use async_trait::async_trait;

use crate::error::Result;

/// The media types this crate reads, makes and stores.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MimeType {
    Png,
    Jpeg,
    Webp,
    Gif,
    Mp4,
}

impl MimeType {
    /// The MIME type as a header spells it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Webp => "image/webp",
            Self::Gif => "image/gif",
            Self::Mp4 => "video/mp4",
        }
    }

    /// The file extension a copy on disk gets.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Webp => "webp",
            Self::Gif => "gif",
            Self::Mp4 => "mp4",
        }
    }

    /// One of the four image types, from a header value.
    pub fn image_from_header(value: &str) -> Option<Self> {
        match value {
            "image/png" => Some(Self::Png),
            "image/jpeg" => Some(Self::Jpeg),
            "image/webp" => Some(Self::Webp),
            "image/gif" => Some(Self::Gif),
            _ => None,
        }
    }
}

/// An image to look at: bytes this service already holds, or a URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageInput {
    Bytes { data: Vec<u8>, mime_type: MimeType },
    Url(String),
}

/// Something that can say what is in a picture.
#[async_trait]
pub trait ImageReader: Send + Sync {
    /// What is in the image, in words, answering `question` when one is asked.
    /// The answer is written by a model looking at a picture a stranger sent,
    /// so callers treat it as untrusted text like any other tool result.
    async fn describe(&self, image: ImageInput, question: Option<&str>) -> Result<String>;
}

/// The shape of a generated still.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImageAspect {
    #[default]
    Square,
    Landscape,
    Portrait,
}

impl ImageAspect {
    /// The tool schema's spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Square => "square",
            Self::Landscape => "landscape",
            Self::Portrait => "portrait",
        }
    }
}

/// A generated still: PNG, JPEG or WebP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedImage {
    pub data: Vec<u8>,
    pub mime_type: MimeType,
}

/// Something that draws a picture from a prompt.
#[async_trait]
pub trait ImageGenerator: Send + Sync {
    async fn generate(&self, prompt: &str, aspect: ImageAspect) -> Result<GeneratedImage>;
}

/// The frame of a generated clip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VideoOrientation {
    #[default]
    Square,
    Landscape,
    Portrait,
}

impl VideoOrientation {
    /// The tool schema's spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Square => "square",
            Self::Landscape => "landscape",
            Self::Portrait => "portrait",
        }
    }
}

/// What to film.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VideoRequest {
    pub prompt: String,
    /// `None` is each generator's own default.
    pub orientation: Option<VideoOrientation>,
    /// Wanted clip length. A hint: a generator that cannot set one ignores it.
    pub seconds: Option<u32>,
}

/// A generated clip, always MP4.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedVideo {
    pub data: Vec<u8>,
    pub seconds: Option<u32>,
}

/// Something that films a clip from a prompt.
#[async_trait]
pub trait VideoGenerator: Send + Sync {
    async fn generate(&self, request: VideoRequest) -> Result<GeneratedVideo>;
}

/// Which allowance a generation draws on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MediaKind {
    Image,
    Video,
}

impl MediaKind {
    /// `image` or `video`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Video => "video",
        }
    }
}
