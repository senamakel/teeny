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

#[tokio::test]
async fn a_failed_download_never_names_the_token_and_only_reads_the_file_api() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!("/bot{TOKEN}/getFile")))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "ok": true, "result": {
                "file_id": "f", "file_size": 3, "file_path": "photos/missing.jpg"
            }})),
        )
        .mount(&server)
        .await;
    let api = TelegramApi::new(TOKEN).with_base_url(server.uri());
    let error = api.download_image("f").await.unwrap_err();
    assert!(!error.to_string().contains(TOKEN), "{error}");
    let fetched: Vec<String> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|request| request.url.path().replace(TOKEN, "<token>"))
        .collect();
    assert_eq!(
        fetched,
        ["/bot<token>/getFile", "/file/bot<token>/photos/missing.jpg"]
    );
}

#[test]
fn a_file_path_stays_under_the_file_api() {
    assert!(is_file_path("photos/file_1.jpg"));
    for path in [
        "",
        "/etc/passwd",
        "../bot/getMe",
        "photos/../../x",
        "https://evil.example/x.png",
        "photos/x.jpg?y",
        "photos//x.jpg",
    ] {
        assert!(!is_file_path(path), "{path}");
    }
}
