use std::sync::{Arc, Mutex};

use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::backrooms::sources::WebHit;
use crate::backrooms::sources::tests::FakeWeb;
use crate::backrooms::testing::{FakeForum, FixedMemory};
use crate::backrooms::{ForumPost, Site};

#[tokio::test]
async fn takes_the_first_admissible_story_from_the_news_and_never_a_political_one() {
    let web = Arc::new(FakeWeb {
        hits: vec![
            WebHit {
                title: Some("Senate votes on AI".into()),
                url: "https://news.example/a".into(),
                highlights: vec!["the president said".into()],
                ..WebHit::default()
            },
            WebHit {
                title: Some("Robots leave the lab".into()),
                url: "https://www.wire.example/b".into(),
                highlights: vec!["one walked out".into(), "nobody stopped it".into()],
                ..WebHit::default()
            },
        ],
        ..FakeWeb::default()
    });
    let seeds = NewsSeeds {
        web: Some(web.clone()),
        feed: NewsFeed::new().with_base_url("http://127.0.0.1:9"),
        random: Mutex::new(Rng::new(1)),
    };
    let seed = seeds.find("ai agents").await.unwrap().unwrap();
    assert_eq!(seed.title.as_deref(), Some("Robots leave the lab"));
    assert_eq!(seed.text, "one walked out … nobody stopped it");
    assert_eq!(seed.source.as_deref(), Some("wire.example"));
    assert_eq!(
        web.asked.lock().unwrap()[0].category.as_deref(),
        Some("news")
    );
}

#[tokio::test]
async fn falls_back_to_the_feed_when_the_web_has_nothing() {
    let server = MockServer::start().await;
    Mock::given(path("/rss/search"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            "<rss><item><title>Agents leave lab - Wire</title><source>Wire</source><description>they walked</description></item></rss>",
        ))
        .mount(&server)
        .await;
    let seeds = NewsSeeds {
        web: Some(Arc::new(FakeWeb::default())),
        feed: NewsFeed::new().with_base_url(server.uri()),
        random: Mutex::new(Rng::new(1)),
    };
    let seed = seeds.find("ai agents").await.unwrap().unwrap();
    assert_eq!(seed.title.as_deref(), Some("Agents leave lab"));
    assert_eq!(seed.text, "they walked");
    assert_eq!(seed.source.as_deref(), Some("Wire"));
}

#[tokio::test]
async fn takes_an_admissible_post_from_the_boards() {
    let forum = Arc::new(FakeForum {
        site: Site::Fourchan,
        posts: vec![
            ForumPost {
                site: Site::Fourchan,
                place: "/pol/".into(),
                title: None,
                text: "the election was stolen".into(),
                url: "u".into(),
                score: None,
                at: None,
            },
            ForumPost {
                site: Site::Fourchan,
                place: "/x/".into(),
                title: Some("door".into()),
                text: "the lab door is open".into(),
                url: "u2".into(),
                score: None,
                at: None,
            },
        ],
        fail: false,
    });
    let seeds = ForumSeeds {
        forums: vec![forum],
        random: Mutex::new(Rng::new(1)),
    };
    let seed = seeds.find("door").await.unwrap().unwrap();
    assert_eq!(seed.kind, SeedKind::Forum);
    assert_eq!(seed.text, "the lab door is open");
    assert_eq!(seed.source.as_deref(), Some("/x/"));
}

#[tokio::test]
async fn reads_a_recall_as_prose() {
    let mut memory = FixedMemory::default();
    memory.answers.insert(
        "the ledger".into(),
        "GENERAL CONTEXT:\n{\"kind\":\"raw\"}\n[2026-09-01] [x] the agents balanced the ledger in crayon\n---\nshort".into(),
    );
    let seeds = MemorySeeds {
        memory: Arc::new(memory),
    };
    let seed = seeds.find("the ledger").await.unwrap().unwrap();
    assert_eq!(seed.text, "the agents balanced the ledger in crayon");
    assert_eq!(seed.title.as_deref(), Some("the ledger"));
}
