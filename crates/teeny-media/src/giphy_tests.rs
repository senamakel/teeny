use serde_json::json;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;

fn client(server: &MockServer) -> GiphyClient {
    GiphyClient::new("key", "@TinyHumansAI").with_base_url(server.uri())
}

fn waving(server: &MockServer) -> serde_json::Value {
    json!({
        "id": "abc123",
        "title": "  tiny waving ",
        "url": "https://giphy.com/gifs/tiny-waving-abc123",
        "images": {
            "downsized_medium": { "url": format!("{}/media/huge.gif", server.uri()), "size": "16000000" },
            "downsized": { "url": format!("{}/media/abc123.gif", server.uri()), "size": "2048" },
            "original": { "url": format!("{}/media/original.gif", server.uri()), "size": "4096" }
        }
    })
}

#[tokio::test]
async fn searches_the_account_channel_by_default_and_picks_the_largest_rendition_x_takes() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/gifs/search"))
        .and(query_param("api_key", "key"))
        .and(query_param("q", "@TinyHumansAI waving"))
        .and(query_param("limit", "10"))
        .and(query_param("rating", "g"))
        .and(query_param("bundle", "messaging_non_clips"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": [
            waving(&server),
            { "id": "nothing-usable", "images": { "original": { "url": "https://x/y.gif", "size": "0" } } },
            { "id": "bare", "images": { "fixed_height": { "url": "https://x/bare.gif", "size": 10 } } }
        ] })))
        .expect(1)
        .mount(&server)
        .await;

    let gifs = client(&server).search("waving", true).await.unwrap();
    assert_eq!(
        gifs,
        vec![
            Gif {
                id: "abc123".into(),
                title: "tiny waving".into(),
                page_url: "https://giphy.com/gifs/tiny-waving-abc123".into(),
                media_url: format!("{}/media/abc123.gif", server.uri()),
                mime_type: MimeType::Gif,
            },
            Gif {
                id: "bare".into(),
                title: "(untitled)".into(),
                page_url: "https://giphy.com/gifs/bare".into(),
                media_url: "https://x/bare.gif".into(),
                mime_type: MimeType::Gif,
            },
        ]
    );
}

#[tokio::test]
async fn can_search_beyond_the_channel() {
    let server = MockServer::start().await;
    Mock::given(path("/v1/gifs/search"))
        .and(query_param("q", "waving"))
        .and(query_param("limit", "3"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": [] })))
        .expect(1)
        .mount(&server)
        .await;
    assert!(
        client(&server)
            .with_limit(3)
            .search("waving", false)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn looks_one_up_by_id_and_knows_a_missing_one() {
    let server = MockServer::start().await;
    Mock::given(path("/v1/gifs/abc123"))
        .and(query_param("api_key", "key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": waving(&server) })))
        .mount(&server)
        .await;
    Mock::given(path("/v1/gifs/gone"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    Mock::given(path("/v1/gifs/broken"))
        .respond_with(ResponseTemplate::new(500).set_body_string("down"))
        .mount(&server)
        .await;
    let giphy = client(&server);
    assert_eq!(giphy.get("abc123").await.unwrap().unwrap().id, "abc123");
    assert_eq!(giphy.get("gone").await.unwrap(), None);
    assert_eq!(
        giphy.get("broken").await.unwrap_err().to_string(),
        "Giphy lookup failed (500): down"
    );
}

#[tokio::test]
async fn reports_a_failed_search_with_giphys_own_words() {
    let server = MockServer::start().await;
    Mock::given(path("/v1/gifs/search"))
        .respond_with(ResponseTemplate::new(403).set_body_string("bad key"))
        .mount(&server)
        .await;
    assert_eq!(
        client(&server)
            .search("x", true)
            .await
            .unwrap_err()
            .to_string(),
        "Giphy search failed (403): bad key"
    );
}

#[tokio::test]
async fn downloads_the_rendition_and_refuses_one_over_the_upload_limit() {
    let server = MockServer::start().await;
    Mock::given(path("/media/abc123.gif"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![1, 2, 3]))
        .mount(&server)
        .await;
    Mock::given(path("/media/huge.gif"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0; MAX_UPLOAD_BYTES + 1]))
        .mount(&server)
        .await;
    let giphy = client(&server);
    let mut gif = Gif {
        id: "abc123".into(),
        title: "t".into(),
        page_url: "p".into(),
        media_url: format!("{}/media/abc123.gif", server.uri()),
        mime_type: MimeType::Gif,
    };
    assert_eq!(
        giphy.download(&gif).await.unwrap(),
        (vec![1, 2, 3], MimeType::Gif)
    );
    gif.media_url = format!("{}/media/huge.gif", server.uri());
    let error = giphy.download(&gif).await.unwrap_err();
    assert_eq!(
        error.to_string(),
        format!(
            "GIF abc123 is {} bytes, over the X upload limit",
            MAX_UPLOAD_BYTES + 1
        )
    );
    gif.media_url = format!("{}/media/missing.gif", server.uri());
    let error = giphy.download(&gif).await.unwrap_err();
    assert!(
        error
            .to_string()
            .starts_with("Giphy download failed (404) for "),
        "{error}"
    );
}

#[tokio::test]
async fn find_gif_lists_what_it_found_for_the_model() {
    let server = MockServer::start().await;
    Mock::given(path("/v1/gifs/search"))
        .and(query_param("q", "@TinyHumansAI waving"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "data": [waving(&server), waving(&server)] })),
        )
        .mount(&server)
        .await;
    let output = find_gif(&client(&server), "TinyHumansAI", "waving", false)
        .await
        .unwrap();
    assert_eq!(
        output,
        "id: abc123\ntitle: tiny waving\npage: https://giphy.com/gifs/tiny-waving-abc123\n---\nid: abc123\ntitle: tiny waving\npage: https://giphy.com/gifs/tiny-waving-abc123"
    );
}

#[tokio::test]
async fn find_gif_names_the_channel_when_nothing_matched_inside_it() {
    let server = MockServer::start().await;
    Mock::given(path("/v1/gifs/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": [] })))
        .mount(&server)
        .await;
    let giphy = client(&server);
    let output = find_gif(&giphy, "TinyHumansAI", "waving", false)
        .await
        .unwrap();
    assert_eq!(
        output,
        "No GIF in the @TinyHumansAI channel matched \"waving\". Try another wording, or set searchAllOfGiphy to look beyond the channel."
    );
    assert_eq!(
        find_gif(&giphy, "TinyHumansAI", "waving", true)
            .await
            .unwrap(),
        "No GIF matched \"waving\"."
    );
    let requests = server.received_requests().await.unwrap();
    assert!(
        requests[1]
            .url
            .query_pairs()
            .any(|(key, value)| key == "q" && value == "waving")
    );
}
