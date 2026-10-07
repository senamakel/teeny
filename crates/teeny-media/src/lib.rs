//! Giphy, vision, image and video generation, the media store and the chart
//! renderer (SVG to PNG via resvg): the Rust port of the legacy
//! `src/media`, `src/chart`, the moon picture and the Giphy client.
//!
//! - [`chart`] and [`moon`] draw SVG byte-for-byte as the TypeScript did;
//!   [`png::svg_to_png`] rasterises it with the system fonts.
//! - [`vision`], [`images`], [`videos`] and [`reference`] speak the
//!   OpenAI-compatible ladder; [`wiring::build_media`] builds them from
//!   [`teeny_core::Config`].
//! - [`store`] holds generated media under the legacy ids and paths, and
//!   defines the [`store::MediaBudget`] hook the cadence crate implements.
//! - [`tools`] and [`giphy::find_gif`] are the data side of the media tools.

mod candle;
pub mod chart;
mod error;
pub mod fit;
pub mod format;
pub mod giphy;
mod http;
pub mod images;
pub mod moon;
pub mod png;
pub mod reference;
pub mod store;
pub mod tools;
pub mod types;
pub mod videos;
pub mod vision;
pub mod wiring;

pub use candle::Candle;
pub use error::{Error, Result};
