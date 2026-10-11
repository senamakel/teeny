use std::sync::Arc;

use async_trait::async_trait;

use super::*;
use crate::backrooms::{Seed, SeedKind};
use crate::morning::news::Story;
use crate::runner::{AgentKey, Origin, ScriptedRunner};

struct Exa;

#[async_trait]
impl NewsSource for Exa {
    async fn search(&self, _: &str, _: u32, _: u32) -> Result<Vec<Story>, String> {
        Ok(Vec::new())
    }
    async fn read(&self, url: &str, _: usize) -> String {
        format!("exa read {url}")
    }
}

struct OneSeed(Seed);

#[async_trait]
impl SeedFinder for OneSeed {
    async fn find(&self, _: &str) -> anyhow::Result<Option<Seed>> {
        Ok(Some(self.0.clone()))
    }
}

fn research(runner: &Arc<ScriptedRunner>, label: &str) -> BrowserResearch {
    BrowserResearch::new(Arc::clone(runner) as Arc<dyn AgentRunner>, label)
}

#[tokio::test]
async fn a_page_is_read_by_the_researcher_room_on_a_scheduled_turn() {
    let runner = Arc::new(ScriptedRunner::new().reply("The page says the model shipped."));
    let reading = research(&runner, "morning.research")
        .read("https://techcrunch.com/a", 1000)
        .await;
    assert_eq!(reading.as_deref(), Some("The page says the model shipped."));
    let turn = &runner.seen()[0];
    match &turn.agent {
        AgentKey::Room {
            persona,
            system_prompt,
        } => {
            assert_eq!(persona, PERSONA);
            assert!(system_prompt.contains("Never log in"), "{system_prompt}");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(turn.origin, Origin::Scheduled { once: true });
    assert_eq!(turn.label, "morning.research");
    assert!(turn.prompt.contains("https://techcrunch.com/a"));
    assert!(turn.prompt.contains("browser"));
}

#[tokio::test]
async fn an_unreadable_page_a_failure_or_a_non_https_url_is_no_reading() {
    let runner = Arc::new(
        ScriptedRunner::new()
            .reply(UNREADABLE)
            .reply("  ")
            .fail("down"),
    );
    let research = research(&runner, "lore.research");
    for _ in 0..3 {
        assert_eq!(research.read("https://arxiv.org/abs/1", 1000).await, None);
    }
    assert_eq!(research.read("http://arxiv.org/abs/1", 1000).await, None);
    assert_eq!(research.read("javascript:alert(1)", 1000).await, None);
    assert_eq!(
        runner.seen().len(),
        3,
        "no turn for a url the browser would refuse"
    );
}

#[tokio::test]
async fn a_reading_is_cut_to_the_asked_length() {
    let runner = Arc::new(ScriptedRunner::new().reply("x".repeat(500)));
    let reading = research(&runner, "r")
        .read("https://a.com", 100)
        .await
        .unwrap();
    assert_eq!(reading.chars().count(), 100);
}

#[tokio::test]
async fn the_morning_reads_its_article_in_the_browser_and_falls_back_to_exa() {
    let runner = Arc::new(
        ScriptedRunner::new()
            .reply("browser reading")
            .reply(UNREADABLE),
    );
    let news = BrowsingNews {
        inner: Arc::new(Exa),
        research: research(&runner, "morning.research"),
    };
    assert_eq!(news.read("https://a.com/1", 1000).await, "browser reading");
    assert_eq!(
        news.read("https://a.com/2", 1000).await,
        "exa read https://a.com/2"
    );
}

#[tokio::test]
async fn a_lore_seed_from_a_page_carries_what_the_browser_read() {
    let mut seed = Seed::text(SeedKind::News, "headline text");
    seed.source = Some("theverge.com".into());
    seed.url = Some("https://theverge.com/story".into());
    let runner = Arc::new(ScriptedRunner::new().reply("the article in full"));
    let seeds = BrowsingSeeds {
        inner: Arc::new(OneSeed(seed.clone())),
        research: research(&runner, "lore.research"),
    };
    let found = seeds.find("ai").await.unwrap().unwrap();
    assert!(found.text.starts_with("headline text"));
    assert!(found.text.contains("the article in full"));

    // A seed with no page is untouched, and no turn is spent on it.
    let runner = Arc::new(ScriptedRunner::new());
    let seeds = BrowsingSeeds {
        inner: Arc::new(OneSeed(Seed::text(SeedKind::Memory, "a memory"))),
        research: research(&runner, "lore.research"),
    };
    assert_eq!(seeds.find("ai").await.unwrap().unwrap().text, "a memory");
    assert!(runner.seen().is_empty());
}
