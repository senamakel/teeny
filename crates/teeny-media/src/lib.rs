//! Giphy, vision, image and video generation, the media store and the chart
//! renderer (SVG to PNG via resvg).

mod candle;
pub mod chart;
pub mod fit;
mod error;
mod http;
pub mod format;
pub mod images;
pub mod moon;
pub mod png;
pub mod reference;
pub mod store;
pub mod types;
pub mod videos;
pub mod vision;

pub use candle::Candle;
pub use error::{Error, Result};
