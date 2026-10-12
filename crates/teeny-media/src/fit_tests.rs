use std::io::Cursor;

use image::{ImageFormat, Rgb, RgbImage};

use super::*;

fn encoded(image: &RgbImage, format: ImageFormat) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    image.write_to(&mut out, format).unwrap();
    out.into_inner()
}

fn flat(width: u32, height: u32) -> RgbImage {
    RgbImage::from_pixel(width, height, Rgb([0xff, 0xcc, 0x00]))
}

/// Noise does not compress, so the PNG is as big as the pixels.
fn noise(width: u32, height: u32) -> RgbImage {
    let mut state: u32 = 0x2545_f491;
    RgbImage::from_fn(width, height, |_, _| {
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state & 0xff) as u8
        };
        Rgb([next(), next(), next()])
    })
}

#[test]
fn cuts_a_square_from_the_middle_to_16_9_and_leaves_a_wide_one_alone() {
    let square = GeneratedImage {
        data: encoded(&flat(640, 640), ImageFormat::Png),
        mime_type: MimeType::Png,
    };
    let cut = crop_to_aspect(&square, 16.0 / 9.0).unwrap();
    assert_eq!(cut.mime_type, MimeType::Png);
    let decoded = image::load_from_memory(&cut.data).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (640, 360));

    let wide = GeneratedImage {
        data: encoded(&flat(1600, 900), ImageFormat::Jpeg),
        mime_type: MimeType::Jpeg,
    };
    assert_eq!(crop_to_aspect(&wide, 16.0 / 9.0).unwrap(), wide);
}

#[test]
fn cuts_a_tall_image_from_the_middle_band() {
    let mut tall = flat(300, 900);
    for y in 300..600 {
        for x in 0..300 {
            tall.put_pixel(x, y, Rgb([0, 0, 0xff]));
        }
    }
    let cut = crop_to_aspect(
        &GeneratedImage {
            data: encoded(&tall, ImageFormat::Png),
            mime_type: MimeType::Png,
        },
        1.0,
    )
    .unwrap();
    let decoded = image::load_from_memory(&cut.data).unwrap().to_rgb8();
    assert_eq!(decoded.dimensions(), (300, 300));
    assert_eq!(decoded.get_pixel(150, 150).0, [0, 0, 0xff]);
}

#[test]
fn passes_small_images_gifs_and_videos_through_untouched() {
    let small = encoded(&flat(64, 64), ImageFormat::Png);
    assert_eq!(
        fit_image_for_x(small.clone(), MimeType::Png).unwrap(),
        (small, MimeType::Png)
    );
    let big_gif = vec![0x47; X_IMAGE_MAX_BYTES + 1];
    assert_eq!(
        fit_image_for_x(big_gif.clone(), MimeType::Gif).unwrap(),
        (big_gif, MimeType::Gif)
    );
    let big_video = vec![0; X_IMAGE_MAX_BYTES + 1];
    assert_eq!(
        fit_image_for_x(big_video.clone(), MimeType::Mp4).unwrap(),
        (big_video, MimeType::Mp4)
    );
}

#[test]
fn re_encodes_an_oversized_still_as_a_jpeg_under_the_limit() {
    let png = encoded(&noise(1_600, 1_200), ImageFormat::Png);
    assert!(
        png.len() > X_IMAGE_MAX_BYTES,
        "fixture is only {} bytes",
        png.len()
    );
    let (data, mime_type) = fit_image_for_x(png, MimeType::Png).unwrap();
    assert_eq!(mime_type, MimeType::Jpeg);
    assert!(data.len() <= X_IMAGE_MAX_BYTES);
    let decoded = image::load_from_memory(&data).unwrap();
    assert_eq!(
        decoded.width() % 400,
        0,
        "only ever full width or halved: {}",
        decoded.width()
    );
}
