//! Everything media-shaped a turn may reach, built once from configuration
//! and shared by every drafter in the process, so a generated id can be
//! attached by whichever job asked for it.

use std::sync::Arc;
use std::time::Duration;

use teeny_core::Config;
use teeny_core::config::VideoGenApi;

use crate::images::OpenAiImageGenerator;
use crate::reference::{OpenAiTextModel, ReferenceSheet, TextModel};
use crate::store::MediaStore;
use crate::types::{ImageGenerator, ImageReader, VideoGenerator};
use crate::videos::{LadderVideoGenerator, OpenAiVideoGenerator};
use crate::vision::OpenAiVisionReader;

/// What a turn may see and make. The daily budget is not here: it belongs to
/// the cadence rules and is passed to each generation.
#[derive(Clone)]
pub struct MediaAccess {
    pub store: Arc<MediaStore>,
    /// Absent: the account cannot see pictures and the prompts say so.
    pub reader: Option<Arc<dyn ImageReader>>,
    pub images: Option<Arc<dyn ImageGenerator>>,
    pub videos: Option<Arc<dyn VideoGenerator>>,
    /// What Tiny looks like, for the generators. Absent without a reader,
    /// without a generator, or with the list off.
    pub reference: Option<Arc<ReferenceSheet>>,
}

impl std::fmt::Debug for MediaAccess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaAccess")
            .field("store", &self.store)
            .field("reader", &self.reader.is_some())
            .field("images", &self.images.is_some())
            .field("videos", &self.videos.is_some())
            .field("reference", &self.reference)
            .finish()
    }
}

impl MediaAccess {
    /// The same access with some of the generators taken away.
    ///
    /// Who may generate is decided per turn, not per process. Seeing is never
    /// taken away, and the store is the same object, so an id made elsewhere
    /// is still attachable.
    pub fn gate(&self, images: bool, videos: bool) -> Self {
        let images = self.images.clone().filter(|_| images);
        let videos = self.videos.clone().filter(|_| videos);
        let reference = self
            .reference
            .clone()
            .filter(|_| images.is_some() || videos.is_some());
        Self {
            store: Arc::clone(&self.store),
            reader: self.reader.clone(),
            images,
            videos,
            reference,
        }
    }

    /// Whether anything here can generate: (images, videos).
    pub fn can_generate(&self) -> (bool, bool) {
        (self.images.is_some(), self.videos.is_some())
    }
}

/// Builds media access from configuration, or nothing at all when every part
/// is off, so the tool list and the prompts stay exactly as they were on a box
/// that has none of it.
///
/// Every endpoint defaults to the LLM's: the ladder serves vision, images and
/// video on the same URL and key as chat, so the common case is one setting.
pub fn build_media(config: &Config) -> Option<MediaAccess> {
    let media = &config.media;
    let llm = &config.llm;
    let llm_key = llm.api_key.expose();
    let pick = |url: &Option<String>| url.clone().unwrap_or_else(|| llm.base_url.clone());
    let key = |secret: &Option<teeny_core::Secret>| {
        secret
            .as_ref()
            .map_or(llm_key, |secret| secret.expose())
            .to_owned()
    };

    let vision_model = media.vision_model.trim();
    let reader: Option<Arc<dyn ImageReader>> = (!vision_model.is_empty()).then(|| {
        Arc::new(OpenAiVisionReader::new(
            pick(&media.vision_base_url),
            key(&media.vision_api_key),
            vision_model,
        )) as Arc<dyn ImageReader>
    });
    let images: Option<Arc<dyn ImageGenerator>> = media.image_gen_enabled.then(|| {
        let mut generator = OpenAiImageGenerator::new(
            pick(&media.image_gen_base_url),
            key(&media.image_gen_api_key),
            &media.image_gen_model,
        );
        if let Some(quality) = &media.image_gen_quality {
            generator = generator.with_quality(quality);
        }
        Arc::new(generator) as Arc<dyn ImageGenerator>
    });
    let videos: Option<Arc<dyn VideoGenerator>> = media.video_gen_enabled.then(|| {
        let (base_url, api_key, model) = (
            pick(&media.video_gen_base_url),
            key(&media.video_gen_api_key),
            media.video_gen_model.clone(),
        );
        let timeout = Duration::from_secs(u64::from(media.video_gen_timeout_s));
        match media.video_gen_api {
            VideoGenApi::OpenAi => Arc::new(
                OpenAiVideoGenerator::new(base_url, api_key, model)
                    .with_seconds(media.video_gen_seconds)
                    .with_timeout(timeout),
            ) as Arc<dyn VideoGenerator>,
            VideoGenApi::Ladder => {
                Arc::new(LadderVideoGenerator::new(base_url, api_key, model).with_timeout(timeout))
            }
        }
    });
    if reader.is_none() && images.is_none() && videos.is_none() {
        return None;
    }

    // The sheet needs eyes to be built and a generator to be worth building.
    let reference = match &reader {
        Some(reader)
            if (images.is_some() || videos.is_some())
                && !media.tiny_reference_images.is_empty() =>
        {
            let text: Arc<dyn TextModel> =
                Arc::new(OpenAiTextModel::new(&llm.base_url, llm_key, &llm.model));
            Some(Arc::new(
                ReferenceSheet::new(
                    media.tiny_reference_images.clone(),
                    Arc::clone(reader),
                    text,
                )
                .with_cache_path(&media.reference_sheet_path),
            ))
        }
        _ => None,
    };

    Some(MediaAccess {
        store: Arc::new(MediaStore::new().with_output_dir(&media.output_dir)),
        reader,
        images,
        videos,
        reference,
    })
}

#[cfg(test)]
#[path = "wiring_tests.rs"]
mod tests;
