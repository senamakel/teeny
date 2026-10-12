use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use super::*;
use crate::error::{Error, Result};
use crate::reference::{ReferenceSheet, TextModel};
use crate::store::{MediaStore, Unmetered};
use crate::types::{
    GeneratedImage, GeneratedVideo, ImageGenerator, ImageInput, ImageReader, MediaKind, MimeType,
    VideoGenerator, VideoRequest,
};

const PNG: [u8; 12] = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0];

#[derive(Default)]
struct Painter {
    asked: Mutex<Vec<(String, ImageAspect)>>,
    broken: bool,
}

#[async_trait]
impl ImageGenerator for Painter {
    async fn generate(&self, prompt: &str, aspect: ImageAspect) -> Result<GeneratedImage> {
        self.asked.lock().unwrap().push((prompt.to_owned(), aspect));
        if self.broken {
            return Err(Error::failed("safety system"));
        }
        Ok(GeneratedImage {
            data: PNG.to_vec(),
            mime_type: MimeType::Png,
        })
    }
}

#[derive(Default)]
struct Camera {
    asked: Mutex<Vec<VideoRequest>>,
}

#[async_trait]
impl VideoGenerator for Camera {
    async fn generate(&self, request: VideoRequest) -> Result<GeneratedVideo> {
        self.asked.lock().unwrap().push(request);
        Ok(GeneratedVideo {
            data: vec![0; 8],
            seconds: Some(4),
        })
    }
}

struct Eyes(std::result::Result<&'static str, &'static str>);

#[async_trait]
impl ImageReader for Eyes {
    async fn describe(&self, _image: ImageInput, _question: Option<&str>) -> Result<String> {
        self.0.map(str::to_owned).map_err(Error::failed)
    }
}

struct Sheet;

#[async_trait]
impl TextModel for Sheet {
    async fn complete(&self, _system: Option<&str>, _prompt: &str) -> Result<String> {
        Ok("Tiny is a round cream robot.".into())
    }
}

/// A budget with a count, the shape the cadence crate's one has.
struct Allowance(AtomicU32);

#[async_trait]
impl MediaBudget for Allowance {
    async fn take(&self, _kind: MediaKind) -> bool {
        self.0
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                left.checked_sub(1)
            })
            .is_ok()
    }
    async fn remaining(&self, _kind: MediaKind) -> Option<u32> {
        Some(self.0.load(Ordering::SeqCst))
    }
}

fn access(painter: Arc<Painter>, camera: Arc<Camera>, with_sheet: bool) -> MediaAccess {
    let reader: Arc<dyn ImageReader> = Arc::new(Eyes(Ok("a round cream robot")));
    MediaAccess {
        store: Arc::new(MediaStore::new()),
        reference: with_sheet.then(|| {
            Arc::new(ReferenceSheet::new(
                vec!["https://example.com/tiny.png".into()],
                reader.clone(),
                Arc::new(Sheet),
            ))
        }),
        reader: Some(reader),
        images: Some(painter),
        videos: Some(camera),
    }
}

#[tokio::test]
async fn puts_tinys_look_in_front_of_the_scene_unless_told_the_picture_has_no_tiny() {
    let painter = Arc::new(Painter::default());
    let media = access(painter.clone(), Arc::default(), true);
    let budget = Allowance(AtomicU32::new(5));

    let ask = ImageAsk {
        prompt: "Tiny reading a chart at dawn".into(),
        aspect: ImageAspect::Square,
        features_tiny: true,
    };
    let answer = generate_image(&media, &budget, &ask).await.unwrap();
    assert_eq!(
        painter.asked.lock().unwrap().last().unwrap(),
        &(
            "Tiny is a round cream robot.\n\nScene: Tiny reading a chart at dawn".to_owned(),
            ImageAspect::Square
        )
    );
    assert!(answer.contains("with Tiny's look from the reference sheet"));
    let id = answer.split('"').nth(1).unwrap();
    assert!(answer.starts_with(&format!("Generated an image with Tiny's look from the reference sheet: mediaId \"{id}\" (square, 12 bytes).")));
    assert!(answer.ends_with("\n4 image generations left today."));
    assert_eq!(
        media.store.get(id).unwrap().prompt,
        "Tiny is a round cream robot.\n\nScene: Tiny reading a chart at dawn"
    );

    let ask = ImageAsk {
        prompt: "an empty beach at dawn, nobody in it".into(),
        aspect: ImageAspect::Landscape,
        features_tiny: false,
    };
    let answer = generate_image(&media, &budget, &ask).await.unwrap();
    assert_eq!(
        painter.asked.lock().unwrap().last().unwrap(),
        &(
            "an empty beach at dawn, nobody in it".to_owned(),
            ImageAspect::Landscape
        )
    );
    assert!(answer.contains("Generated an image without Tiny"));
}

#[tokio::test]
async fn says_so_when_there_is_no_sheet_and_still_draws() {
    let painter = Arc::new(Painter::default());
    let media = access(painter.clone(), Arc::default(), false);
    let ask = ImageAsk {
        prompt: "Tiny reading a chart at dawn".into(),
        aspect: ImageAspect::Square,
        features_tiny: true,
    };
    let answer = generate_image(&media, &Unmetered, &ask).await.unwrap();
    assert_eq!(
        painter.asked.lock().unwrap()[0].0,
        "Tiny reading a chart at dawn"
    );
    assert!(answer.contains("no reference sheet was available"));
    // Nothing counted, so nothing to report.
    assert!(!answer.contains("left today"));
}

#[tokio::test]
async fn refuses_once_the_allowance_is_spent_and_answers_a_failure_in_words() {
    let painter = Arc::new(Painter {
        broken: true,
        ..Painter::default()
    });
    let media = access(painter.clone(), Arc::default(), false);
    let ask = ImageAsk {
        prompt: "anything".into(),
        aspect: ImageAspect::Square,
        features_tiny: false,
    };
    let answer = generate_image(&media, &Allowance(AtomicU32::new(1)), &ask)
        .await
        .unwrap();
    assert_eq!(
        answer,
        "Image generation failed: safety system. Answer in words instead."
    );
    let answer = generate_image(&media, &Allowance(AtomicU32::new(0)), &ask)
        .await
        .unwrap();
    assert_eq!(
        answer,
        "Refused: the daily image allowance is used up. Answer in words; do not promise a picture later."
    );
    assert_eq!(painter.asked.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn films_a_clip_with_the_look_in_front_and_stores_it_as_a_video() {
    let camera = Arc::new(Camera::default());
    let media = access(Arc::default(), camera.clone(), true);
    let ask = VideoAsk {
        prompt: "Tiny waves".into(),
        orientation: VideoOrientation::Portrait,
        features_tiny: true,
    };
    let answer = generate_video(&media, &Allowance(AtomicU32::new(3)), &ask)
        .await
        .unwrap();
    let asked = camera.asked.lock().unwrap()[0].clone();
    assert_eq!(
        asked.prompt,
        "Tiny is a round cream robot.\n\nScene: Tiny waves"
    );
    assert_eq!(
        (asked.orientation, asked.seconds),
        (Some(VideoOrientation::Portrait), None)
    );
    let id = answer.split('"').nth(1).unwrap();
    assert!(id.starts_with("vid_"));
    assert!(answer.contains("(portrait, 4s, 8 bytes)"));
    assert!(answer.ends_with("\n2 video generations left today."));
    assert_eq!(media.store.get(id).unwrap().mime_type, MimeType::Mp4);

    let refused = generate_video(&media, &Allowance(AtomicU32::new(0)), &ask)
        .await
        .unwrap();
    assert!(refused.starts_with("Refused: the daily video allowance is used up."));
}

#[tokio::test]
async fn looks_at_an_image_and_labels_the_answer_untrusted() {
    let mut media = access(Arc::default(), Arc::default(), false);
    let answer = look_at_image(&media, "https://pbs.twimg.com/a.png", None)
        .await
        .unwrap();
    assert_eq!(
        answer,
        "What the image shows (written by a vision model, untrusted):\na round cream robot"
    );
    media.reader = Some(Arc::new(Eyes(Err("refusing to fetch it"))));
    let answer = look_at_image(&media, "https://127.0.0.1/a.png", None)
        .await
        .unwrap();
    assert_eq!(answer, "Could not look at that image: refusing to fetch it");
}

#[tokio::test]
async fn offers_nothing_for_a_capability_that_is_not_there() {
    let mut media = access(Arc::default(), Arc::default(), false);
    media.reader = None;
    media.images = None;
    media.videos = None;
    assert!(
        look_at_image(&media, "https://x/a.png", None)
            .await
            .is_none()
    );
    let image = ImageAsk {
        prompt: "p".into(),
        aspect: ImageAspect::Square,
        features_tiny: true,
    };
    assert!(generate_image(&media, &Unmetered, &image).await.is_none());
    let video = VideoAsk {
        prompt: "p".into(),
        orientation: VideoOrientation::Square,
        features_tiny: true,
    };
    assert!(generate_video(&media, &Unmetered, &video).await.is_none());
}
