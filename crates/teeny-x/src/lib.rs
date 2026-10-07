//! The X API, owned: an OAuth 1.0a signer, a typed v2 client, a read-through
//! cache in front of it, the webhook and the activity stream, and the
//! `Publisher` seam every write goes through.
//!
//! Everything returned is a plain serde value, so the X tools (built in
//! teeny-agent) are thin wrappers. Reads: [`XApi`], or any [`cache::XReads`]
//! (cached or not). Writes: only [`XPublisher`].

pub mod activity;
pub mod auth;
pub mod article;
pub mod cache;
mod client;
mod error;
pub mod intake;
mod media;
pub mod mention;
pub mod model;
pub mod oauth;
mod publisher;
pub mod stream;
pub mod webhook;
pub mod webhook_admin;
mod wire;
mod writes;

#[cfg(test)]
mod test_support;

pub use client::{X_API_BASE, XApi};
pub use error::{XError, XResult};
pub use publisher::{
    Draft, MAX_POST_LENGTH, NoopGuard, PublishError, PublishGuard, Published, Refusal, XPublisher,
};
