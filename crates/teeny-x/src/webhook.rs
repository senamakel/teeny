//! The webhook's two checks: X's challenge-response (CRC), and the signature
//! on every delivery. Both are an HMAC-SHA256 under the app's consumer secret.

use base64::Engine as _;
use hmac::{Hmac, Mac};
use sha2::Sha256;

/// The CRC answer for a `GET ...?crc_token=...`: `sha256=` and the base64
/// HMAC of the token. Served as `{"response_token": ...}`.
pub fn crc_response(token: &str, consumer_secret: &str) -> String {
    signature_of(token.as_bytes(), consumer_secret)
}

/// Whether `x-twitter-webhooks-signature` is X's signature of the raw body.
/// Compared in constant time; an absent header is never valid.
pub fn is_valid_signature(raw_body: &[u8], signature: Option<&str>, consumer_secret: &str) -> bool {
    let Some(received) = signature else {
        return false;
    };
    let expected = signature_of(raw_body, consumer_secret);
    constant_time_eq(expected.as_bytes(), received.as_bytes())
}

fn signature_of(message: &[u8], consumer_secret: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(consumer_secret.as_bytes())
        .expect("HMAC takes any key length");
    mac.update(message);
    format!(
        "sha256={}",
        base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
    )
}

/// Equal lengths and equal bytes, without stopping at the first difference.
/// The length is not secret: every valid signature has the same one.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b)
        .fold(0u8, |difference, (x, y)| difference | (x ^ y))
        == 0
}

#[cfg(test)]
#[path = "webhook_tests.rs"]
mod tests;
