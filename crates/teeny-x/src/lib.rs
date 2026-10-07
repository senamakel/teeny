//! The X API, owned: an OAuth 1.0a signer, a typed v2 client, a read-through
//! cache in front of it, the webhook and the activity stream, and the
//! `Publisher` seam every write goes through.

pub mod oauth;
