//! Clients pointed at a mock server, for the command tests.

use std::sync::Arc;

use teeny_core::Secret;
use teeny_x::oauth::OAuth1;
use teeny_x::{NoopGuard, XApi, XPublisher};
use wiremock::MockServer;

/// The account's client, with an app bearer, as production runs.
pub fn x(server: &MockServer) -> XApi {
    XApi::new(
        "teenydroid",
        OAuth1::new("ck", Secret::new("cs"), "at", Secret::new("ats")),
        Some(Secret::new("app-bearer")),
    )
    .with_base_url(server.uri())
}

pub fn publisher(server: &MockServer, dry_run: bool) -> XPublisher {
    XPublisher::new(Arc::new(x(server)), Arc::new(NoopGuard), dry_run)
}

pub fn printed(out: Vec<u8>) -> String {
    String::from_utf8(out).expect("utf-8 output")
}
