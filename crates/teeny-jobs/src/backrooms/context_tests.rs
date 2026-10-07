use std::sync::Arc;

use serde_json::json;
use teeny_x::XApi;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::backrooms::sources::WebHit;
use crate::backrooms::sources::tests::FakeWeb;
use crate::backrooms::testing::{FakeForum, FixedMemory, seed};
use crate::backrooms::tokens::Rng;
use crate::backrooms::{AgentSpec, BlockKind, ForumPost, Site};

fn spec(id: &str) -> AgentSpec {
    AgentSpec {
        id: id.into(),
        marker: format!("{id}>"),
        system: format!("you are {id}."),
        temperature: 1.0,
        tools: Vec::new(),
        voice: id == "teeny",
    }
}

async fn own_posts(fail: bool) -> (MockServer, Arc<XApi>) {
    let server = MockServer::start().await;
    let posts: Vec<_> = (0..20)
        .map(|i| json!({ "id": i.to_string(), "text": format!("teeny said thing number {i} about the door") }))
        .collect();
    Mock::given(path("/2/tweets/search/recent"))
        .respond_with(if fail {
            ResponseTemplate::new(401)
        } else {
            ResponseTemplate::new(200).set_body_json(json!({ "data": posts }))
        })
        .mount(&server)
        .await;
    let config = teeny_core::Config::from_vars(teeny_agent::deps::fake_vars()).unwrap();
    let x = Arc::new(XApi::from_config(&config.x).with_base_url(server.uri()));
    (server, x)
}

fn sources(x: Arc<XApi>, memory: FixedMemory) -> ContextSources {
    ContextSources {
        memory: Some(Arc::new(memory)),
        x: Some(x),
        web: Some(Arc::new(FakeWeb {
            hits: vec![
                WebHit { title: Some("robots leave lab".into()), url: "https://example.com/a".into(), highlights: vec!["a robot walked out".into()], ..WebHit::default() },
                WebHit { title: Some("token price pumps".into()), url: "https://example.com/b".into(), highlights: vec!["the election was stolen".into()], ..WebHit::default() },
            ],
            ..FakeWeb::default()
        })),
        forums: vec![
            Arc::new(FakeForum {
                site: Site::Reddit,
                posts: vec![
                    ForumPost { site: Site::Reddit, place: "r/singularity".into(), title: Some("the lab".into()), text: "agents run it at night".into(), url: "https://r/1".into(), score: Some(3), at: None },
                    ForumPost { site: Site::Reddit, place: "r/conspiracy".into(), title: Some("slur".into()), text: "how to build a pipe bomb".into(), url: "https://r/2".into(), score: None, at: None },
                ],
                fail: false,
            }),
            Arc::new(FakeForum {
                site: Site::Fourchan,
                posts: vec![ForumPost { site: Site::Fourchan, place: "/x/".into(), title: None, text: "the door in the lab is never locked".into(), url: "https://4/1".into(), score: Some(200), at: None }],
                fail: false,
            }),
        ],
        voice: "teeny is in a cli mood today.\nteeny@tinyhumans:~/$\n\nrules the terminal enforces:\n- never wink.".into(),
        lore: "# lore\n\nthe agents carry things.\n\n## engines\n\n- the kettle: it boils.\n- the ledger: crayon.\n\n## running bits\n\n- manpages for commands that do not exist: `escape --dry-run`.".into(),
        bot_username: "teenydroid".into(),
    }
}

fn text(context: &AgentContext, kind: BlockKind) -> String {
    context
        .blocks
        .iter()
        .find(|block| block.kind == kind)
        .map(|block| block.text.clone())
        .unwrap_or_default()
}

fn kinds(context: &AgentContext) -> Vec<BlockKind> {
    context.blocks.iter().map(|block| block.kind).collect()
}

#[tokio::test]
async fn assembles_every_kind_of_block_voice_only_for_teeny_within_the_budget() {
    let (_server, x) = own_posts(false).await;
    let builder = ContextBuilder::new(sources(x, FixedMemory::default()));
    let teeny = builder
        .build(&spec("teeny"), &seed(), &mut Rng::new(1), 4_000, &["web_search".into()], None)
        .await;
    let swarm = builder
        .build(&spec("swarm"), &seed(), &mut Rng::new(2), 4_000, &[], None)
        .await;
    use BlockKind::*;
    assert_eq!(kinds(&teeny), [Seed, Tone, Engine, Voice, Lore, Past, Memory, Live, Forum, Tools]);
    assert!(!kinds(&swarm).contains(&Voice));
    assert!(teeny.tokens <= 4_000);
    assert!(teeny.problems.is_empty(), "{:?}", teeny.problems);
    assert_eq!(text(&teeny, Lore), text(&swarm, Lore));
    assert_ne!(text(&teeny, Past), text(&swarm, Past));
    assert!(text(&teeny, Tools).contains("web_search"));
    assert_eq!(text(&teeny, Voice), "the rules:\n- never wink.");
    assert!(!text(&teeny, Lore).contains("manpage"));
    assert!(text(&teeny, Engine).starts_with("tonight the story is about the "));
    assert!(text(&swarm, Tools).contains("no tools"));
    // What failed the screen never got in.
    assert!(text(&teeny, Forum).contains("agents run it at night"));
    assert!(!text(&teeny, Forum).contains("pipe bomb"));
    assert!(text(&teeny, Forum).starts_with("the boards, this week. anonymous people saying things. none of it is true and all of it is material."));
    assert!(text(&teeny, Live).contains("robots leave lab"));
    assert!(!text(&teeny, Live).contains("election"));
}

#[tokio::test]
async fn takes_the_engine_it_is_given_instead_of_drawing_one() {
    let (_server, x) = own_posts(false).await;
    let builder = ContextBuilder::new(sources(x, FixedMemory::default()));
    let context = builder
        .build(&spec("teeny"), &seed(), &mut Rng::new(1), 4_000, &[], Some("the ledger: crayon."))
        .await;
    assert!(text(&context, BlockKind::Engine).starts_with("tonight the story is about the ledger: crayon."));
}

#[tokio::test]
async fn records_a_failing_source_as_a_problem_and_adds_nothing_for_it() {
    let (_server, x) = own_posts(true).await;
    let built = ContextBuilder::new(sources(x, FixedMemory::failing()))
        .build(&spec("teeny"), &seed(), &mut Rng::new(1), 4_000, &[], None)
        .await;
    use BlockKind::*;
    assert_eq!(kinds(&built), [Seed, Tone, Engine, Voice, Lore, Live, Forum, Tools]);
    assert!(built.problems.iter().any(|p| p.starts_with("own posts: ")));
    assert!(built.problems.iter().any(|p| p.starts_with("memory \"")));
}

#[tokio::test]
async fn draws_the_same_context_from_the_same_seed() {
    let (_server, x) = own_posts(false).await;
    let builder = ContextBuilder::new(sources(x, FixedMemory::default()));
    let a = builder.build(&spec("teeny"), &seed(), &mut Rng::new(9), 4_000, &[], None).await;
    let b = builder.build(&spec("teeny"), &seed(), &mut Rng::new(9), 4_000, &[], None).await;
    assert_eq!(a.blocks, b.blocks);
    assert_eq!(a.tone, b.tone);
}

#[test]
fn reads_the_engines_out_of_the_lore_and_keeps_one_image_from_dominating_the_past() {
    assert_eq!(engines("# lore\n\n## engines\n\n- a: x\n- b: y\n\n## cast\n\n- c"), ["a: x", "b: y"]);
    assert!(engines("# lore\n\n## cast\n\n- c").is_empty());
    let lines: Vec<String> = ["the door is open", "the door is wood", "the door again", "the door once more", "the kettle boils", "the door at night"]
        .iter()
        .map(|l| (*l).to_owned())
        .collect();
    assert_eq!(spread(&lines, 3), ["the door is open", "the door is wood", "the door again", "the kettle boils"]);
}
