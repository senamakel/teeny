use serde_json::json;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use super::*;
use crate::model::SortOrder;
use crate::test_support::{api, api_without_bearer, is_oauth};

const POST_FIELDS: &str = "created_at,conversation_id,text,attachments,public_metrics";
const EXPANSIONS: &str = "author_id,in_reply_to_user_id,attachments.media_keys";

async fn me(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/2/users/me"))
        .and(query_param("user.fields", "username,public_metrics"))
        .and(|request: &Request| is_oauth(request))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": { "id": "2094854245468000256", "username": "teenydroid", "public_metrics": { "followers_count": 321 } }
        })))
        .expect(1)
        .mount(server)
        .await;
}

#[tokio::test]
async fn gets_a_post_with_its_expansions_on_the_app_bearer() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/2/tweets/77"))
        .and(header("authorization", "Bearer app-bearer"))
        .and(query_param("tweet.fields", POST_FIELDS))
        .and(query_param("user.fields", "username"))
        .and(query_param(
            "media.fields",
            "media_key,type,url,preview_image_url,alt_text",
        ))
        .and(query_param("expansions", EXPANSIONS))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": { "id": "77", "text": "hello", "author_id": "42" },
            "includes": { "users": [{ "id": "42", "username": "someone" }] }
        })))
        .mount(&server)
        .await;

    let post = api(&server).get_post(&"77".into()).await.unwrap().unwrap();
    assert_eq!(post.text, "hello");
    assert_eq!(post.author_username.as_deref(), Some("someone"));
}

#[tokio::test]
async fn a_post_x_does_not_have_is_none() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/2/tweets/404"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "errors": [{ "title": "Not Found Error", "resource_id": "404" }]
        })))
        .mount(&server)
        .await;
    assert_eq!(api(&server).get_post(&"404".into()).await.unwrap(), None);
}

#[tokio::test]
async fn reads_sign_with_oauth_when_there_is_no_bearer() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/2/tweets/1"))
        .and(|request: &Request| is_oauth(request))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "data": { "id": "1", "text": "x" } })),
        )
        .expect(1)
        .mount(&server)
        .await;
    assert!(
        api_without_bearer(&server)
            .get_post(&"1".into())
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn searches_recent_posts_with_clamped_max_results_and_paging() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/2/tweets/search/recent"))
        .and(header("authorization", "Bearer app-bearer"))
        .and(query_param("query", "$TINY -is:retweet"))
        .and(query_param("max_results", "10"))
        .and(query_param("since_id", "500"))
        .and(query_param("sort_order", "recency"))
        .and(query_param("pagination_token", "page2"))
        .and(query_param("tweet.fields", POST_FIELDS))
        .and(query_param("expansions", EXPANSIONS))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{ "id": "600", "text": "a" }, { "id": "601", "text": "b" }],
            "meta": { "next_token": "page3", "newest_id": "601", "result_count": 2 }
        })))
        .mount(&server)
        .await;

    let page = api(&server)
        .search_page(
            "$TINY -is:retweet",
            &SearchOptions {
                max_results: Some(3),
                since_id: Some("500".into()),
                sort_order: Some(SortOrder::Recency),
                pagination_token: Some("page2".into()),
                ..SearchOptions::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(page.posts.len(), 2);
    assert_eq!(page.next_token.as_deref(), Some("page3"));
    assert_eq!(page.newest_id, Some("601".into()));
}

#[tokio::test]
async fn caps_a_search_at_one_hundred_and_an_empty_one_is_empty() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/2/tweets/search/recent"))
        .and(query_param("max_results", "100"))
        .and(query_param("start_time", "2026-10-01T00:00:00Z"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "meta": { "result_count": 0 } })),
        )
        .expect(1)
        .mount(&server)
        .await;
    let posts = api(&server)
        .search_posts(
            "q",
            &SearchOptions {
                max_results: Some(500),
                start_time: Some("2026-10-01T00:00:00Z".into()),
                ..SearchOptions::default()
            },
        )
        .await
        .unwrap();
    assert!(posts.is_empty());
}

#[tokio::test]
async fn reads_a_conversation_by_its_id() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/2/tweets/search/recent"))
        .and(query_param("query", "conversation_id:900"))
        .and(query_param("max_results", "25"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "data": [{ "id": "901", "text": "r" }] })),
        )
        .expect(1)
        .mount(&server)
        .await;
    let posts = api(&server)
        .get_conversation(&"900".into(), None)
        .await
        .unwrap();
    assert_eq!(posts[0].id.as_str(), "901");
}

#[tokio::test]
async fn finds_mentions_by_the_live_handle_and_identifies_once() {
    let server = MockServer::start().await;
    me(&server).await;
    Mock::given(method("GET"))
        .and(path("/2/tweets/search/recent"))
        .and(query_param(
            "query",
            "(@teenydroid OR to:teenydroid) -from:teenydroid -is:retweet",
        ))
        .and(query_param("since_id", "10"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "data": [{ "id": "11", "text": "@teenydroid hi" }] })),
        )
        .expect(2)
        .mount(&server)
        .await;

    let x = api(&server);
    let window = MentionWindow {
        max_results: None,
        start: WindowStart::SinceId("10".into()),
    };
    assert_eq!(x.find_mentions(&window).await.unwrap().len(), 1);
    x.find_mentions(&window).await.unwrap();
    let identity = x.identity().await.unwrap();
    assert_eq!(identity.id.as_str(), "2094854245468000256");
    assert_eq!(identity.followers, Some(321));
}

#[tokio::test]
async fn reads_trends_worldwide_by_default_and_skips_blank_names() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/2/trends/by/woeid/1"))
        .and(query_param("max_trends", "20"))
        .and(query_param("trend.fields", "trend_name,tweet_count"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{ "trend_name": " #tiny ", "tweet_count": 12 }, { "trend_name": "  " }, { "trend_name": "gm" }]
        })))
        .mount(&server)
        .await;
    let trends = api(&server).trends(TrendOptions::default()).await.unwrap();
    assert_eq!(
        trends,
        vec![
            Trend {
                name: "#tiny".into(),
                posts: Some(12)
            },
            Trend {
                name: "gm".into(),
                posts: None
            }
        ]
    );
}

#[tokio::test]
async fn asks_whether_someone_follows_the_account_as_the_account() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/2/users/42"))
        .and(query_param("user.fields", "connection_status"))
        .and(|request: &Request| is_oauth(request))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": { "id": "42", "connection_status": ["followed_by", "following"] }
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/2/users/43"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(
                json!({ "data": { "id": "43", "connection_status": ["following"] } }),
            ),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/2/users/44"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": { "id": "44" } })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/2/users/45"))
        .respond_with(ResponseTemplate::new(429))
        .mount(&server)
        .await;

    let x = api(&server);
    assert_eq!(x.follows_account(&"42".into()).await, Some(true));
    assert_eq!(x.follows_account(&"43".into()).await, Some(false));
    // Unknown is not "no": a missing field or a refusal is None.
    assert_eq!(x.follows_account(&"44".into()).await, None);
    assert_eq!(x.follows_account(&"45".into()).await, None);
}

#[tokio::test]
async fn reads_own_post_metrics_as_the_account_in_batches_of_one_hundred() {
    let server = MockServer::start().await;
    let ids: Vec<PostId> = (0..150).map(|n| PostId::from(n.to_string())).collect();
    let first: Vec<String> = (0..100).map(|n| n.to_string()).collect();
    Mock::given(method("GET"))
        .and(path("/2/tweets"))
        .and(query_param("ids", first.join(",")))
        .and(query_param("tweet.fields", "public_metrics,created_at"))
        .and(|request: &Request| is_oauth(request))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [
                { "id": "0", "text": "a", "public_metrics": { "impression_count": 5, "like_count": 1, "reply_count": 0, "retweet_count": 2, "quote_count": 0 } },
                { "id": "1", "text": "no counters" }
            ]
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/2/tweets"))
        .and(query_param(
            "ids",
            (100..150)
                .map(|n| n.to_string())
                .collect::<Vec<_>>()
                .join(","),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": [] })))
        .expect(1)
        .mount(&server)
        .await;

    let metrics = api(&server).post_metrics(&ids).await.unwrap();
    assert_eq!(
        metrics,
        vec![OwnPostMetrics {
            id: "0".into(),
            impressions: 5,
            likes: 1,
            replies: 0,
            reposts: 2,
            quotes: 0
        }]
    );
}

#[tokio::test]
async fn an_error_status_is_an_http_error_with_the_body() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/2/tweets/search/recent"))
        .respond_with(
            ResponseTemplate::new(429).set_body_string("{\"title\":\"Too Many Requests\"}"),
        )
        .mount(&server)
        .await;
    let error = api(&server)
        .search_posts("q", &SearchOptions::default())
        .await
        .unwrap_err();
    match error {
        XError::Http { status, body } => {
            assert_eq!(status, 429);
            assert!(body.contains("Too Many Requests"));
        }
        other => panic!("expected an HTTP error, got {other:?}"),
    }
}
