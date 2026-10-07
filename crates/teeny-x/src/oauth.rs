//! OAuth 1.0a request signing (RFC 5849, HMAC-SHA1), as X signs user-context
//! calls: every write, `/2/users/me`, and the reads that only the account
//! itself may make (its own impressions, a follow relationship).
//!
//! The signature covers the method, the URL without its query, the query
//! parameters, any form-encoded body parameters and the `oauth_*` set. A JSON
//! body is not a parameter and is not signed, which is what X expects.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use hmac::{Hmac, Mac};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use reqwest::Url;
use sha1::Sha1;
use sha2::{Digest, Sha256};
use teeny_core::Secret;

/// RFC 5849 section 3.6: everything but ALPHA, DIGIT, `-`, `.`, `_` and `~`.
const OAUTH_RESERVED: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// Percent-encodes a value the way RFC 5849 requires: UTF-8, upper-case hex,
/// and only the unreserved set left alone. (Not `encodeURIComponent`, which
/// leaves `!*'()` bare.)
pub fn percent_encode(value: &str) -> String {
    utf8_percent_encode(value, OAUTH_RESERVED).to_string()
}

/// The account's user-context credentials, able to sign a request.
#[derive(Debug, Clone)]
pub struct OAuth1 {
    consumer_key: String,
    consumer_secret: Secret,
    token: String,
    token_secret: Secret,
}

impl OAuth1 {
    /// The app's consumer pair and the account's access-token pair.
    pub fn new(
        consumer_key: impl Into<String>,
        consumer_secret: Secret,
        token: impl Into<String>,
        token_secret: Secret,
    ) -> Self {
        Self {
            consumer_key: consumer_key.into(),
            consumer_secret,
            token: token.into(),
            token_secret,
        }
    }

    /// The `Authorization` header for one request, with a fresh nonce and the
    /// current time. `url` carries the query; `form` is a form-encoded body,
    /// empty for a JSON body or none.
    pub fn header(&self, method: &str, url: &str, form: &[(&str, &str)]) -> String {
        self.header_with(method, url, form, &nonce(), unix_now())
    }

    /// [`OAuth1::header`] with the nonce and timestamp given, so a signature
    /// can be checked against a known one.
    pub fn header_with(
        &self,
        method: &str,
        url: &str,
        form: &[(&str, &str)],
        nonce: &str,
        timestamp: u64,
    ) -> String {
        let base = self.base_string(method, url, form, nonce, timestamp);
        let key = format!(
            "{}&{}",
            percent_encode(self.consumer_secret.expose()),
            percent_encode(self.token_secret.expose())
        );
        let mut mac =
            Hmac::<Sha1>::new_from_slice(key.as_bytes()).expect("HMAC takes any key length");
        mac.update(base.as_bytes());
        let signature =
            base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());

        let mut oauth = self.oauth_params(nonce, timestamp);
        oauth.push(("oauth_signature", signature));
        oauth.sort();
        let fields: Vec<String> = oauth
            .iter()
            .map(|(name, value)| format!("{}=\"{}\"", percent_encode(name), percent_encode(value)))
            .collect();
        format!("OAuth {}", fields.join(", "))
    }

    /// The signature base string (RFC 5849 section 3.4.1).
    pub fn base_string(
        &self,
        method: &str,
        url: &str,
        form: &[(&str, &str)],
        nonce: &str,
        timestamp: u64,
    ) -> String {
        let (base_url, query) = match Url::parse(url) {
            Ok(parsed) => {
                let query: Vec<(String, String)> = parsed
                    .query_pairs()
                    .map(|(name, value)| (name.into_owned(), value.into_owned()))
                    .collect();
                let mut bare = parsed;
                bare.set_query(None);
                bare.set_fragment(None);
                (bare.to_string(), query)
            }
            Err(_) => (url.to_owned(), Vec::new()),
        };

        let mut params: Vec<(String, String)> = query
            .iter()
            .map(|(name, value)| (percent_encode(name), percent_encode(value)))
            .chain(
                form.iter()
                    .map(|(name, value)| (percent_encode(name), percent_encode(value))),
            )
            .chain(
                self.oauth_params(nonce, timestamp)
                    .into_iter()
                    .map(|(name, value)| (percent_encode(name), percent_encode(&value))),
            )
            .collect();
        params.sort();
        let normalized: Vec<String> = params
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect();

        format!(
            "{}&{}&{}",
            method.to_ascii_uppercase(),
            percent_encode(&base_url),
            percent_encode(&normalized.join("&"))
        )
    }

    /// The `oauth_*` set. A signer with no token (the app alone, asking for
    /// a request token) sends no `oauth_token` at all, as RFC 5849 has it.
    fn oauth_params(&self, nonce: &str, timestamp: u64) -> Vec<(&'static str, String)> {
        let mut params = vec![
            ("oauth_consumer_key", self.consumer_key.clone()),
            ("oauth_nonce", nonce.to_owned()),
            ("oauth_signature_method", "HMAC-SHA1".to_owned()),
            ("oauth_timestamp", timestamp.to_string()),
        ];
        if !self.token.is_empty() {
            params.push(("oauth_token", self.token.clone()));
        }
        params.push(("oauth_version", "1.0".to_owned()));
        params
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

/// A nonce only has to be unique per timestamp, not secret: the clock, a
/// process-wide counter and the pid, hashed.
fn nonce() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let mut hasher = Sha256::new();
    hasher.update(nanos.to_le_bytes());
    hasher.update(COUNTER.fetch_add(1, Ordering::Relaxed).to_le_bytes());
    hasher.update(std::process::id().to_le_bytes());
    hasher
        .finalize()
        .iter()
        .take(16)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
#[path = "oauth_tests.rs"]
mod tests;
