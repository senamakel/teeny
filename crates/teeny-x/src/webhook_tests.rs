use base64::Engine as _;
use hmac::{Hmac, Mac};
use sha2::Sha256;

use super::*;

fn reference(message: &[u8], secret: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(message);
    format!(
        "sha256={}",
        base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
    )
}

#[test]
fn answers_a_challenge_with_the_hmac_of_the_token() {
    assert_eq!(
        crc_response("challenge", "secret"),
        reference(b"challenge", "secret")
    );
    // A fixed vector, so a change to the encoding cannot pass silently.
    assert_eq!(
        crc_response("challenge", "secret"),
        "sha256=oeUF6Wxqoezggrue+wbIDxKRPSF6esKwizR2MHh9HaA="
    );
}

#[test]
fn accepts_only_a_delivery_x_signed() {
    let body = br#"{"tweet_create_events":[{"id_str":"123","text":"hello"}],"favorite_events":[{"id":"456"}]}"#;
    let signature = reference(body, "secret");
    assert!(is_valid_signature(body, Some(&signature), "secret"));
    assert!(!is_valid_signature(body, None, "secret"));
    assert!(!is_valid_signature(body, Some("sha256=nope"), "secret"));
    assert!(!is_valid_signature(body, Some(&signature), "other secret"));
    assert!(!is_valid_signature(b"{}", Some(&signature), "secret"));
    assert!(!is_valid_signature(
        body,
        Some(&signature[..signature.len() - 1]),
        "secret"
    ));
}
