use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::json;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;

/// A web search that answers from a list and keeps every query.
#[derive(Default)]
pub struct FakeWeb {
    pub hits: Vec<WebHit>,
    pub asked: std::sync::Mutex<Vec<WebQuery>>,
}

#[async_trait]
impl WebSearch for FakeWeb {
    async fn search(&self, query: &WebQuery) -> anyhow::Result<Vec<WebHit>> {
        self.asked.lock().unwrap().push(query.clone());
        Ok(self.hits.clone())
    }
}

fn no_pace() -> Arc<Pacer> {
    Arc::new(Pacer::new(Duration::ZERO))
}

#[tokio::test]
async fn fourchan_matches_the_query_most_hits_then_most_replies_first_and_reads_the_best_thread() {
    let server = MockServer::start().await;
    Mock::given(path("/x/catalog.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([{ "page": 1, "threads": [
            { "no": 1, "sub": "AI escaped the lab", "com": "anons it happened &gt; the <b>door</b> was open", "replies": 40, "time": 1_700_000_000 },
            { "no": 2, "com": "unrelated thread about gpus", "replies": 400, "time": 1_700_000_000 },
            { "no": 3, "sub": "lab door", "com": "someone left it open", "replies": 3, "time": 1_700_000_000 }
        ]}])))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path("/x/thread/1.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "posts": [
            { "no": 1, "com": "op" },
            { "no": 4, "com": "&gt;&gt;1 i was there. the door was one inch open and nobody said anything." },
            { "no": 5, "com": "short" }
        ]})))
        .mount(&server)
        .await;
    let forum = FourchanForum::new(vec!["x".into()], no_pace())
        .with_base_url(server.uri())
        .with_replies(2);
    let posts = forum.search("the lab door was open", 5).await.unwrap();
    assert_eq!(
        posts.iter().map(|p| p.url.as_str()).collect::<Vec<_>>(),
        [
            "https://boards.4chan.org/x/thread/1",
            "https://boards.4chan.org/x/thread/3",
            "https://boards.4chan.org/x/thread/1"
        ]
    );
    assert_eq!(posts[0].place, "/x/");
    assert_eq!(posts[0].title.as_deref(), Some("AI escaped the lab"));
    assert_eq!(posts[0].score, Some(40));
    assert_eq!(posts[0].text, "anons it happened > the door was open");
    assert_eq!(posts[2].text, "i was there. the door was one inch open and nobody said anything.");
    assert_eq!(posts[2].place, "/x/ reply");
    // The catalog is read once per board, not once per search.
    forum.search("door", 5).await.unwrap();
}

#[tokio::test]
async fn fourchan_answers_nothing_when_the_board_cannot_be_read() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    let forum = FourchanForum::new(vec!["g".into()], no_pace()).with_base_url(server.uri());
    assert!(forum.search("door", 3).await.unwrap().is_empty());
}

#[tokio::test]
async fn reddit_reads_the_public_json_unescapes_and_sorts_by_score() {
    let server = MockServer::start().await;
    Mock::given(path("/r/singularity/search.json"))
        .and(query_param("q", "door"))
        .and(query_param("restrict_sr", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": { "children": [
            { "data": { "title": "the door &amp; the lab", "selftext": "it was open", "permalink": "/r/singularity/comments/1/x/", "score": 12, "created_utc": 1_700_000_000, "subreddit": "singularity" } },
            { "data": { "title": "only a title", "selftext": "", "permalink": "/r/singularity/comments/2/y/", "score": 99 } }
        ]}})))
        .mount(&server)
        .await;
    let forum = RedditForum::new(vec!["singularity".into()], None, no_pace()).with_base_url(server.uri());
    let posts = forum.search("door", 5).await.unwrap();
    assert_eq!(posts.iter().map(|p| p.score).collect::<Vec<_>>(), [Some(99), Some(12)]);
    assert_eq!(posts[1].title.as_deref(), Some("the door & the lab"));
    assert_eq!(posts[1].text, "it was open");
    assert_eq!(posts[1].place, "r/singularity");
    assert_eq!(posts[1].url, "https://www.reddit.com/r/singularity/comments/1/x/");
    assert_eq!(posts[0].text, "only a title");
}

#[tokio::test]
async fn reddit_falls_back_to_the_web_on_reddit_com_when_reddit_says_403() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;
    let web = Arc::new(FakeWeb {
        hits: vec![WebHit {
            title: Some("from exa".into()),
            url: "https://www.reddit.com/r/artificial/comments/9/z/".into(),
            highlights: vec!["the lab door".into()],
            published_date: Some("2026-09-20".into()),
            ..WebHit::default()
        }],
        ..FakeWeb::default()
    });
    let forum = RedditForum::new(vec!["artificial".into()], Some(web.clone()), no_pace())
        .with_base_url(server.uri());
    let posts = forum.search("door", 5).await.unwrap();
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].place, "r/artificial");
    assert_eq!(posts[0].text, "the lab door");
    assert_eq!(posts[0].at.as_deref(), Some("2026-09-20"));
    assert_eq!(web.asked.lock().unwrap()[0].include_domains, ["reddit.com"]);
}

#[test]
fn reads_titles_sources_and_dates_out_of_the_rss_and_drops_the_publisher_suffix() {
    let xml = r#"<rss><channel><item><title>Robots leave lab - GeekWire</title><link>x</link><pubDate>Mon, 21 Sep 2026 14:21:10 GMT</pubDate><description>&lt;a href="x"&gt;Robots leave lab&lt;/a&gt;&amp;nbsp;GeekWire</description><source url="https://geekwire.com">GeekWire</source></item><item><title>Untitled &amp; odd</title></item></channel></rss>"#;
    let items = parse_rss(xml);
    assert_eq!(
        items,
        [
            RssItem {
                title: "Robots leave lab".into(),
                source: Some("GeekWire".into()),
                at: Some("Mon, 21 Sep 2026 14:21:10 GMT".into()),
                description: Some("Robots leave lab GeekWire".into()),
            },
            RssItem { title: "Untitled & odd".into(), source: None, at: None, description: None },
        ]
    );
}

#[tokio::test]
async fn reads_the_google_news_feed_for_a_query() {
    let server = MockServer::start().await;
    Mock::given(path("/rss/search"))
        .and(query_param("q", "ai agents"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            "<rss><item><title>Agents leave lab - Wire</title><source>Wire</source></item></rss>",
        ))
        .mount(&server)
        .await;
    let feed = NewsFeed::new().with_base_url(server.uri());
    let items = feed.search("ai agents").await.unwrap();
    assert_eq!(items[0].title, "Agents leave lab");
}

#[tokio::test]
async fn keeps_consecutive_requests_a_gap_apart() {
    let pacer = Pacer::new(Duration::from_millis(40));
    let started = Instant::now();
    for _ in 0..3 {
        pacer.wait().await;
    }
    assert!(started.elapsed() >= Duration::from_millis(80));
}
