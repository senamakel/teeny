//! Fitting images to what X and the posts need: the legacy service did this
//! with sharp; here it is the `image` crate.

use std::io::Cursor;

use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, ImageFormat};

use crate::error::{Error, Result};
use crate::types::{GeneratedImage, MimeType};

/// X refuses a still image over this; GIFs and videos have their own, larger
/// limits.
pub const X_IMAGE_MAX_BYTES: usize = 5 * 1024 * 1024;

/// Below this width a picture is a thumbnail, not a post.
const MIN_WIDTH: u32 = 256;

fn decode(data: &[u8]) -> Result<DynamicImage> {
    image::load_from_memory(data).map_err(|error| Error::failed(format!("could not read the image: {error}")))
}

fn jpeg(image: &DynamicImage, quality: u8) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    // JPEG has no alpha; the channel is dropped, as sharp's JPEG output did.
    image
        .to_rgb8()
        .write_with_encoder(JpegEncoder::new_with_quality(&mut out, quality))
        .map_err(|error| Error::failed(format!("could not encode the JPEG: {error}")))?;
    Ok(out)
}

/// A still image small enough for X.
///
/// The image ladder's rungs render what they like: one served a 2048-square
/// PNG of nine megabytes, which X rejects outright. Anything over the limit is
/// re-encoded as JPEG at a high quality (90, 80, 70), and halved in width and
/// tried again if that is still not enough. GIFs and videos pass through
/// untouched; so does anything already under the limit, so the common case
/// costs nothing.
pub fn fit_image_for_x(data: Vec<u8>, mime_type: MimeType) -> Result<(Vec<u8>, MimeType)> {
    if matches!(mime_type, MimeType::Gif | MimeType::Mp4) || data.len() <= X_IMAGE_MAX_BYTES {
        return Ok((data, mime_type));
    }
    let original = decode(&data)?;
    let mut width = original.width();
    loop {
        let scaled = if width < original.width() {
            let height = (f64::from(original.height()) * f64::from(width) / f64::from(original.width())).round();
            original.resize_exact(width, (height as u32).max(1), FilterType::Lanczos3)
        } else {
            original.clone()
        };
        for quality in [90, 80, 70] {
            let encoded = jpeg(&scaled, quality)?;
            if encoded.len() <= X_IMAGE_MAX_BYTES {
                return Ok((encoded, MimeType::Jpeg));
            }
        }
        // Quality has been spent; halve the side and start again.
        width /= 2;
        if width < MIN_WIDTH {
            return Err(Error::failed("could not fit the image under X's size limit"));
        }
    }
}

/// The image cut to an aspect (width over height), from the middle.
///
/// A rung that ignores the size it was asked for answers square; a cover that
/// has to be wide is cut from that, so the prompt that drew it should have
/// kept the subject in the middle band. An image already at the aspect (to
/// within 1%) passes through; a cut one comes back as PNG.
pub fn crop_to_aspect(image: &GeneratedImage, aspect: f64) -> Result<GeneratedImage> {
    let decoded = decode(&image.data)?;
    let (width, height) = (decoded.width(), decoded.height());
    if width == 0 || height == 0 || (f64::from(width) / f64::from(height) - aspect).abs() < 0.01 {
        return Ok(image.clone());
    }
    let (cut_width, cut_height) = if f64::from(width) / f64::from(height) > aspect {
        ((f64::from(height) * aspect).round() as u32, height)
    } else {
        (width, (f64::from(width) / aspect).round() as u32)
    };
    let (cut_width, cut_height) = (cut_width.clamp(1, width), cut_height.clamp(1, height));
    let cut = decoded.crop_imm((width - cut_width) / 2, (height - cut_height) / 2, cut_width, cut_height);
    let mut out = Cursor::new(Vec::new());
    cut.write_to(&mut out, ImageFormat::Png)
        .map_err(|error| Error::failed(format!("could not encode the PNG: {error}")))?;
    Ok(GeneratedImage { data: out.into_inner(), mime_type: MimeType::Png })
}

#[cfg(test)]
#[path = "fit_tests.rs"]
mod tests;
