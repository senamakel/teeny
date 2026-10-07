use std::str::FromStr;

use reqwest::dns::{Name, Resolve};
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;

const PNG: [u8; 12] = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0];

/// A fetcher that lets this one wiremock server through, and nothing else
/// the production guard would refuse.
fn fetcher_for(server: &MockServer) -> ImageFetcher {
    ImageFetcher::trusting(&server.uri())
}

fn redirect_to(location: &str) -> ResponseTemplate {
    ResponseTemplate::new(302).insert_header("location", location)
}

async fn serving_png(server: &MockServer, at: &str) {
    Mock::given(path(at))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "image/png")
                .set_body_bytes(PNG.to_vec()),
        )
        .mount(server)
        .await;
}

#[test]
fn only_addresses_on_the_public_internet_are_public() {
    for public in ["1.1.1.1", "93.184.216.34", "2606:4700::1111"] {
        assert!(is_public_address(public.parse().unwrap()), "{public}");
    }
    for private in [
        "10.0.0.1",
        "172.16.5.4",
        "192.168.1.1",
        "127.0.0.1",
        "169.254.169.254",
        "100.64.0.1",
        "100.127.255.254",
        "0.0.0.0",
        "224.0.0.1",
        "255.255.255.255",
        "240.0.0.1",
        "::1",
        "::",
        "fc00::1",
        "fd12:3456::1",
        "fe80::1",
        "ff02::1",
        "::ffff:127.0.0.1",
        "::ffff:10.1.2.3",
        "64:ff9b::a9fe:a9fe",
    ] {
        assert!(!is_public_address(private.parse().unwrap()), "{private}");
    }
}

#[tokio::test]
async fn refuses_a_name_that_resolves_to_the_box_itself() {
    let resolved = PublicOnlyResolver
        .resolve(Name::from_str("localhost").unwrap())
        .await;
    let Err(error) = resolved else {
        panic!("localhost resolved to something fetchable");
    };
    assert!(error.to_string().contains("non-public"), "{error}");
}

#[tokio::test]
async fn fetches_a_public_image() {
    let server = MockServer::start().await;
    serving_png(&server, "/a.png").await;
    let (data, mime) = fetcher_for(&server)
        .fetch(&format!("{}/a.png", server.uri()))
        .await
        .unwrap();
    assert_eq!((data, mime), (PNG.to_vec(), MimeType::Png));
}

#[tokio::test]
async fn refuses_a_redirect_to_the_box_itself() {
    let server = MockServer::start().await;
    let inside = MockServer::start().await;
    serving_png(&inside, "/secret.png").await;
    Mock::given(path("/hop"))
        .respond_with(redirect_to(&format!("{}/secret.png", inside.uri())))
        .mount(&server)
        .await;
    let error = fetcher_for(&server)
        .fetch(&format!("{}/hop", server.uri()))
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "could not fetch that image");
    assert!(inside.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn refuses_a_redirect_to_the_metadata_endpoint_or_a_bare_service_name() {
    let server = MockServer::start().await;
    Mock::given(path("/meta"))
        .respond_with(redirect_to("https://169.254.169.254/latest/meta-data"))
        .mount(&server)
        .await;
    Mock::given(path("/mongo"))
        .respond_with(redirect_to("https://mongo/a.png"))
        .mount(&server)
        .await;
    let fetcher = fetcher_for(&server);
    for hop in ["/meta", "/mongo"] {
        let error = fetcher
            .fetch(&format!("{}{hop}", server.uri()))
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), "could not fetch that image", "{hop}");
    }
}

#[tokio::test]
async fn follows_a_few_redirects_and_no_more() {
    let server = MockServer::start().await;
    serving_png(&server, "/end.png").await;
    for (from, to) in [("/1", "/2"), ("/2", "/3"), ("/3", "/end.png"), ("/0", "/1")] {
        Mock::given(path(from))
            .respond_with(redirect_to(&format!("{}{to}", server.uri())))
            .mount(&server)
            .await;
    }
    let fetcher = fetcher_for(&server);
    assert!(fetcher.fetch(&format!("{}/1", server.uri())).await.is_ok());
    let error = fetcher
        .fetch(&format!("{}/0", server.uri()))
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "could not fetch that image");
}

#[tokio::test]
async fn refuses_a_url_the_guard_refuses_without_fetching_it() {
    let server = MockServer::start().await;
    let fetcher = fetcher_for(&server);
    for refused in [
        "https://127.0.0.1/a.png",
        "https://cortexdb/a.png",
        "http://pbs.twimg.com/a.png",
    ] {
        let error = fetcher.fetch(refused).await.unwrap_err();
        assert_eq!(
            error.to_string(),
            "refusing to fetch that URL: only public https image URLs are read"
        );
    }
}

#[tokio::test]
async fn answers_the_model_without_echoing_what_the_server_said() {
    let server = MockServer::start().await;
    Mock::given(path("/page"))
        .respond_with(ResponseTemplate::new(200).set_body_raw("<html>", "text/html"))
        .mount(&server)
        .await;
    Mock::given(path("/gone"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let fetcher = fetcher_for(&server);
    let error = fetcher
        .fetch(&format!("{}/page", server.uri()))
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "that URL did not return an image");
    let error = fetcher
        .fetch(&format!("{}/gone", server.uri()))
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "could not fetch that image");
}

#[tokio::test]
async fn trusts_the_bytes_over_a_vague_header() {
    let server = MockServer::start().await;
    Mock::given(path("/vague"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/octet-stream")
                .set_body_bytes(b"RIFF....WEBPVP8 ".to_vec()),
        )
        .mount(&server)
        .await;
    let (_, mime) = fetcher_for(&server)
        .fetch(&format!("{}/vague", server.uri()))
        .await
        .unwrap();
    assert_eq!(mime, MimeType::Webp);
}

#[tokio::test]
async fn stops_reading_past_the_size_limit() {
    let server = MockServer::start().await;
    let mut big = PNG.to_vec();
    big.resize(MAX_IMAGE_BYTES + 1, 0);
    Mock::given(path("/big.png"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "image/png")
                .set_body_bytes(big),
        )
        .mount(&server)
        .await;
    let error = fetcher_for(&server)
        .fetch(&format!("{}/big.png", server.uri()))
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        format!("that image is over the {MAX_IMAGE_BYTES} byte limit")
    );
}
