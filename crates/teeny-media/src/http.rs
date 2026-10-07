//! The bits every OpenAI-shaped client here shares.

use serde_json::Value;

/// `baseUrl.replace(/\/$/, '') + path`.
pub(crate) fn endpoint(base_url: &str, path: &str) -> String {
    format!("{}{path}", base_url.strip_suffix('/').unwrap_or(base_url))
}

/// The response body as JSON, or `{}` when it is not JSON: a provider's error
/// page should surface as its status, not as a parse error.
pub(crate) async fn json_body(response: reqwest::Response) -> Value {
    response
        .json::<Value>()
        .await
        .unwrap_or_else(|_| Value::Object(Default::default()))
}

/// `body.error.message ?? 'no detail'`.
pub(crate) fn error_detail(body: &Value) -> &str {
    body.pointer("/error/message")
        .and_then(Value::as_str)
        .unwrap_or("no detail")
}

/// `encodeURIComponent`.
pub(crate) fn encode_component(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&byte) {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// `Buffer.from(text, 'base64')`: forgiving the way Node is. URL-safe letters
/// read as standard ones, anything outside the alphabet is skipped, padding
/// is optional, and a dangling final character is dropped.
pub(crate) fn decode_base64_lenient(text: &str) -> Vec<u8> {
    use base64::Engine as _;
    use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};

    const LENIENT: GeneralPurpose = GeneralPurpose::new(
        &base64::alphabet::STANDARD,
        GeneralPurposeConfig::new()
            .with_decode_padding_mode(DecodePaddingMode::RequireNone)
            .with_decode_allow_trailing_bits(true),
    );
    let mut clean: String = text
        .chars()
        .filter_map(|character| match character {
            '-' => Some('+'),
            '_' => Some('/'),
            '=' => None,
            other if other.is_ascii_alphanumeric() || other == '+' || other == '/' => Some(other),
            _ => None,
        })
        .collect();
    if clean.len() % 4 == 1 {
        clean.pop();
    }
    LENIENT.decode(clean).unwrap_or_default()
}
