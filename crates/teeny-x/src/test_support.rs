//! Helpers shared by the client tests: a client pointed at a mock server.

use teeny_core::Secret;
use wiremock::MockServer;

use crate::client::XApi;
use crate::oauth::OAuth1;

pub fn signer() -> OAuth1 {
    OAuth1::new("ck", Secret::new("cs"), "at", Secret::new("ats"))
}

/// A client with both an OAuth pair and an app bearer, as production runs.
pub fn api(server: &MockServer) -> XApi {
    XApi::new("@teenydroid", signer(), Some(Secret::new("app-bearer"))).with_base_url(server.uri())
}

/// A client with no bearer: every read signs with OAuth instead.
pub fn api_without_bearer(server: &MockServer) -> XApi {
    XApi::new("teenydroid", signer(), None).with_base_url(server.uri())
}

pub fn is_oauth(request: &wiremock::Request) -> bool {
    request
        .headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("OAuth ") && value.contains("oauth_signature="))
}
