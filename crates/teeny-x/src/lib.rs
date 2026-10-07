//! The X API, owned: an OAuth 1.0a signer, a typed v2 client, a read-through
//! cache in front of it, the webhook and the activity stream, and the
//! `Publisher` seam every write goes through.

pub mod activity;
pub mod article;
pub mod cache;
mod client;
mod error;
pub mod intake;
mod media;
pub mod mention;
pub mod model;
pub mod oauth;
pub mod webhook;
mod wire;
mod writes;

#[cfg(test)]
mod test_support;

pub use client::XApi;
pub use error::{XError, XResult};
