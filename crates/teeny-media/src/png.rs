//! SVG to PNG.
//!
//! X will not accept an SVG as media, so the chart has to be rasterised
//! somewhere. resvg renders text through the system fonts, so the runtime
//! image has fonts installed alongside it: without them every label silently
//! disappears and the chart posts as candles with no axis, which is why the
//! image installs Noto and this asks for it by name.

use std::sync::{Arc, OnceLock};

use resvg::{tiny_skia, usvg};

use crate::error::{Error, Result};

/// What the legacy renderer asked for when a font family is not installed.
const DEFAULT_FONT_FAMILY: &str = "Noto Sans";

/// The chart's background, painted under everything so a transparent corner
/// cannot reach X as black or white depending on the client.
const BACKGROUND: [u8; 3] = [0x13, 0x17, 0x22];

/// The system fonts, loaded once: scanning them takes longer than a render.
fn fonts() -> Arc<usvg::fontdb::Database> {
    static FONTS: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut database = usvg::fontdb::Database::new();
            database.load_system_fonts();
            Arc::new(database)
        })
        .clone()
}

/// The SVG rasterised at its own size, on the chart background.
pub fn svg_to_png(svg: &str) -> Result<Vec<u8>> {
    let options = usvg::Options {
        font_family: DEFAULT_FONT_FAMILY.to_owned(),
        fontdb: fonts(),
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_str(svg, &options)
        .map_err(|error| Error::failed(format!("could not read the SVG: {error}")))?;
    let size = tree.size().to_int_size();
    let mut pixmap = tiny_skia::Pixmap::new(size.width(), size.height())
        .ok_or_else(|| Error::failed("the SVG has no area to draw on"))?;
    let [red, green, blue] = BACKGROUND;
    pixmap.fill(tiny_skia::Color::from_rgba8(red, green, blue, 0xff));
    resvg::render(&tree, tiny_skia::Transform::default(), &mut pixmap.as_mut());
    pixmap
        .encode_png()
        .map_err(|error| Error::failed(format!("could not encode the PNG: {error}")))
}

#[cfg(test)]
#[path = "png_tests.rs"]
mod tests;
