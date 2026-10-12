use serde_json::json;
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use super::*;
use crate::XError;
use crate::test_support::{api, is_oauth};

fn draft() -> ArticleDraft {
    ArticleDraft {
        title: "The tiny ledger".into(),
        content_state: ContentState {
            blocks: vec![
                ArticleBlock {
                    text: "Chapter one".into(),
                    kind: BlockType::HeaderOne,
                    inline_style_ranges: vec![],
                    entity_ranges: vec![],
                },
                ArticleBlock {
                    text: "bold start".into(),
                    kind: BlockType::Unstyled,
                    inline_style_ranges: vec![InlineStyle {
                        offset: 0,
                        length: 4,
                        style: Style::Bold,
                    }],
                    entity_ranges: vec![],
                },
                ArticleBlock {
                    text: " ".into(),
                    kind: BlockType::Atomic,
                    inline_style_ranges: vec![],
                    entity_ranges: vec![EntityRange {
                        key: 0,
                        offset: 0,
                        length: 1,
                    }],
                },
            ],
            entities: vec![ArticleEntity {
                key: "0".into(),
                value: EntityValue {
                    kind: EntityKind::Image,
                    mutability: Mutability::Immutable,
                    data: EntityData {
                        media_items: vec![MediaItem {
                            media_category: "tweet_image".into(),
                            media_id: "m9".into(),
                        }],
                        caption: None,
                        url: None,
                    },
                },
            }],
        },
        cover_media_id: Some("m1".into()),
    }
}

#[tokio::test]
async fn drafts_then_publishes_an_article_in_x_wire_format() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/2/articles/draft"))
        .and(|request: &Request| is_oauth(request))
        .and(body_json(json!({
            "title": "The tiny ledger",
            "content_state": {
                "blocks": [
                    { "text": "Chapter one", "type": "header-one" },
                    { "text": "bold start", "type": "unstyled", "inline_style_ranges": [{ "offset": 0, "length": 4, "style": "bold" }] },
                    { "text": " ", "type": "atomic", "entity_ranges": [{ "key": 0, "offset": 0, "length": 1 }] }
                ],
                "entities": [{
                    "key": "0",
                    "value": { "type": "image", "mutability": "immutable", "data": { "media_items": [{ "media_category": "tweet_image", "media_id": "m9" }] } }
                }]
            },
            "cover_media": { "media_category": "tweet_image", "media_id": "m1" }
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": { "id": "a1" } })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/2/articles/a1/publish"))
        .and(|request: &Request| is_oauth(request))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "data": { "post_id": "p1" } })),
        )
        .expect(1)
        .mount(&server)
        .await;

    let published = api(&server).publish_article(&draft()).await.unwrap();
    assert_eq!(
        published,
        PublishedArticle {
            article_id: "a1".into(),
            post_id: "p1".into()
        }
    );
}

#[tokio::test]
async fn a_refusal_names_its_status() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/2/articles/draft"))
        .respond_with(ResponseTemplate::new(403).set_body_string("{\"title\":\"Forbidden\"}"))
        .mount(&server)
        .await;
    let error = api(&server).publish_article(&draft()).await.unwrap_err();
    assert!(
        matches!(error, XError::ArticleRefused { status: 403, .. }),
        "{error:?}"
    );
    assert!(error.to_string().starts_with("X refused the article (403)"));
}

#[tokio::test]
async fn a_draft_without_a_post_back_is_an_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/2/articles/draft"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": { "id": "a2" } })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/2/articles/a2/publish"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": {} })))
        .mount(&server)
        .await;
    let error = api(&server).publish_article(&draft()).await.unwrap_err();
    assert!(
        error
            .to_string()
            .contains("did not return a post for article a2"),
        "{error}"
    );
}
