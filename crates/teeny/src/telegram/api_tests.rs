//! The Bot API client: what a failed call says, and the file download.

use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;

const TOKEN: &str = "123456:SECRET-token_value";

#[tokio::test]
async fn a_failed_call_never_names_the_token() {
    // Nothing listens on the discard port: the request itself fails.
    let api = TelegramApi::new(TOKEN).with_base_url("http://127.0.0.1:9");
    let error = api.get_me().await.unwrap_err();
    assert!(!error.to_string().contains(TOKEN), "{error}");
    assert!(!format!("{error:?}").contains(TOKEN), "{error:?}");

    // An answer that is not JSON fails in decoding, which names the URL too.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(502).set_body_string("<html>bad gateway"))
        .mount(&server)
        .await;
    let api = TelegramApi::new(TOKEN).with_base_url(server.uri());
    let error = api.send_message(1, "hi", None, None).await.unwrap_err();
    assert!(!error.to_string().contains(TOKEN), "{error}");
}
