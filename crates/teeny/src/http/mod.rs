//! The HTTP server, a port of `legacy/src/server.ts`.
//!
//! One axum router over [`Services`], a bag of trait objects so every route
//! can be exercised against in-memory fakes. A dependency that is absent
//! answers its routes with the same 503 the TypeScript service gave.
//!
//! What is served:
//! - `/health`, `/queue`, and the site's feeds `/mind`, `/ledger`, `/brain`
//!   and `/articles` (read by `web/` over the compose network);
//! - `/webhooks/x`, X's challenge and its signed deliveries;
//! - the operator's routes behind `AGENT_API_TOKEN`: `/safety` and its holds,
//!   `/critique` and `/agent/run`.
//!
//! There is no CORS layer, as there was none in the TypeScript: the site
//! reads these server-side, and a browser only ever opens `/safety`.

mod auth;
pub mod body;
pub mod brain;
mod feeds;
mod reply;

#[cfg(test)]
mod test_support;

use std::sync::Arc;

use axum::Router;
use axum::routing::get;
use teeny_store::{InferenceLedger, Journal};
use teeny_x::intake::ActivityIntake;

use crate::mentions::Mentions;

pub use brain::BrainSource;
pub use feeds::{BrowserStatus, Mind, MindStatus};

/// Everything the routes can reach. Only the consumer secret and the intake
/// are required; everything else switches its routes on when present.
pub struct Services {
    /// X's consumer secret: the key for the CRC answer and the signatures.
    pub consumer_secret: String,
    /// Where a verified delivery goes.
    pub intake: Arc<ActivityIntake>,
    /// The operator's token. Absent, every operator route answers 503.
    pub agent_token: Option<String>,
    pub queue: Option<Arc<Mentions>>,
    pub mind: Option<Mind>,
    pub ledger: Option<Arc<dyn InferenceLedger>>,
    pub brain: Option<Arc<dyn BrainSource>>,
    /// The browser's state for `/health`; `null` until the browser is wired.
    pub browser: Option<Arc<dyn BrowserStatus>>,
}

impl Services {
    /// A server with nothing but the webhook.
    pub fn new(consumer_secret: impl Into<String>, intake: Arc<ActivityIntake>) -> Self {
        Self {
            consumer_secret: consumer_secret.into(),
            intake,
            agent_token: None,
            queue: None,
            mind: None,
            ledger: None,
            brain: None,
            browser: None,
        }
    }
}

/// Shared by every handler.
pub(crate) type State = axum::extract::State<Arc<Services>>;

/// The whole router.
pub fn router(services: Services) -> Router {
    Router::new()
        .route("/health", get(feeds::health))
        .route("/queue", get(feeds::queue))
        .route("/mind", get(feeds::mind))
        .route("/ledger", get(feeds::ledger))
        .route("/brain", get(feeds::brain))
        .fallback(reply::not_found)
        .method_not_allowed_fallback(reply::not_found)
        .with_state(Arc::new(services))
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
