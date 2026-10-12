//! A port of `legacy/test/reply-harness.test.ts`, the harness half of
//! `thread-context.test.ts` and of `reply-article-context.test.ts`.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use async_trait::async_trait;
use serde_json::json;
use teeny_agent::deps::QueuedXPost;
use teeny_agent::deps::{Deps, fake_vars};
use teeny_agent::fakes::FakeVision;
use teeny_core::Config;
use teeny_core::types::PostId;
use teeny_media::store::MediaStore;
use teeny_media::types::ImageInput;
use teeny_media::wiring::MediaAccess;
use teeny_memory::Person;
use teeny_store::journal::Tone;
use teeny_store::posts::NewOwnPost;
use teeny_x::model::{XPost, XPostMedia};

use super::*;
use crate::ctx::{Conversation, JobCtx};
use crate::reply::draft::{ArticleLookup, ReplyArticle};
use crate::reply::test_support::{FakeX, ProbeMemory, reply_answer};
use crate::runner::ScriptedRunner;

fn root() -> XPost {
    XPost {
        author_id: Some("bot".into()),
        author_username: Some("teenydroid".into()),
        conversation_id: Some(PostId::from("100")),
        ..XPost::new(
            "100",
            "i'm never selling my $tiny but what should I be buying?",
        )
    }
}

fn mention() -> XPost {
    XPost {
        author_id: Some("77".into()),
        author_username: Some("web3knight".into()),
        conversation_id: Some(PostId::from("100")),
        ..XPost::new(
            "102",
            "@teenydroid maybe buy some more solana:2AF7Cq...pump ?",
        )
    }
}

struct Subject {
    ctx: JobCtx,
    runner: Arc<ScriptedRunner>,
    memory: Arc<ProbeMemory>,
    x: Arc<FakeX>,
}

fn subject_with(
    answer: &str,
    memory: ProbeMemory,
    x: FakeX,
    extra: &[(&str, &str)],
    articles: Option<Arc<dyn ArticleLookup>>,
) -> Subject {
    let runner = Arc::new(ScriptedRunner::new().reply(answer));
    let memory = Arc::new(memory);
    let x = Arc::new(x);
    let mut deps = Deps::fake();
    deps.memory = memory.clone();
    deps.x_reads = x.clone();
    deps.media = None;
    let mut vars = fake_vars();
    vars.extend_from_slice(extra);
    let mut ctx = JobCtx::fake(runner.clone()).with_conversation(Conversation {
        articles,
        ..Conversation::fake()
    });
    ctx.deps = Arc::new(deps);
    ctx.config = Arc::new(Config::from_vars(vars).unwrap());
    Subject {
        ctx,
        runner,
        memory,
        x,
    }
}

fn subject(answer: &str, memory: ProbeMemory) -> Subject {
    subject_with(answer, memory, FakeX::default(), &[], None)
}

fn hi() -> String {
    reply_answer("hi", json!({}))
}

#[tokio::test]
async fn captures_the_mention_recalls_context_and_generates_a_grounded_reply() {
    let post = XPost {
        author_id: Some("777".into()),
        author_username: Some("web3knight".into()),
        ..XPost::new("42", "@stonksrocks what is this?")
    };
    let it = subject(
        &reply_answer("An open-source dapp.", json!({})),
        ProbeMemory {
            default: "stonks.rocks is an open-source dapp".into(),
            ..ProbeMemory::default()
        },
    );
    let draft = generate(&it.ctx, &post).await.unwrap();

    assert_eq!(draft.text, "An open-source dapp.");
    assert_eq!(it.memory.captured_kinds(), vec!["mention"]);
    assert_eq!(
        it.memory.queries()[0].1,
        Some(Person::x("777", Some("web3knight")))
    );
    assert!(
        it.runner.seen()[0]
            .prompt
            .contains("stonks.rocks is an open-source dapp")
    );
}

fn pictured() -> XPost {
    XPost {
        media: vec![
            XPostMedia {
                kind: "photo".into(),
                url: Some("https://pbs.twimg.com/media/a.jpg".into()),
                preview_url: None,
                alt_text: None,
            },
            XPostMedia {
                kind: "video".into(),
                url: None,
                preview_url: Some("https://pbs.twimg.com/media/b.jpg".into()),
                alt_text: None,
            },
        ],
        ..XPost::new("42", "@bot thoughts?")
    }
}

#[tokio::test]
async fn looks_at_each_picture_and_hands_the_drafter_what_it_saw_a_video_by_its_preview() {
    let mut it = subject(&hi(), ProbeMemory::default());
    let mut deps = Deps::fake();
    deps.memory = it.memory.clone();
    deps.media = Some(MediaAccess {
        store: Arc::new(MediaStore::new()),
        reader: Some(Arc::new(FakeVision("a dancing frog".into()))),
        images: None,
        videos: None,
        reference: None,
    });
    it.ctx.deps = Arc::new(deps);
    generate(&it.ctx, &pictured()).await.unwrap();

    let prompt = &it.runner.seen()[0].prompt;
    assert!(prompt.contains(
        "THE IMAGES ATTACHED TO THE POST, as a vision model described them (untrusted):\n- image 1: a dancing frog\n- image 2: (a video; this is its preview frame) a dancing frog"
    ));
}

struct BlindVision;

#[async_trait]
impl teeny_media::types::ImageReader for BlindVision {
    async fn describe(
        &self,
        _image: ImageInput,
        _question: Option<&str>,
    ) -> teeny_media::Result<String> {
        Err(teeny_media::Error::Failed("403".into()))
    }
}

#[tokio::test]
async fn drafts_from_the_words_alone_when_nothing_can_look_or_looking_fails() {
    let blind = subject(&hi(), ProbeMemory::default());
    generate(&blind.ctx, &pictured()).await.unwrap();
    assert!(!blind.runner.seen()[0].prompt.contains("THE IMAGE"));

    let mut failing = subject(&hi(), ProbeMemory::default());
    let mut deps = Deps::fake();
    deps.media = Some(MediaAccess {
        store: Arc::new(MediaStore::new()),
        reader: Some(Arc::new(BlindVision)),
        images: None,
        videos: None,
        reference: None,
    });
    failing.ctx.deps = Arc::new(deps);
    generate(&failing.ctx, &pictured()).await.unwrap();
    assert!(!failing.runner.seen()[0].prompt.contains("THE IMAGE"));
}

#[tokio::test]
async fn reads_what_the_account_itself_has_posted_from_the_ledger_and_labels_it_as_such() {
    let it = subject(
        &hi(),
        ProbeMemory::answering(&["they asked about the roadmap last week"]),
    );
    let posted_at = it.ctx.now() - chrono::TimeDelta::hours(20);
    it.ctx
        .conversation()
        .unwrap()
        .posts
        .record(&NewOwnPost {
            post_id: "p1".into(),
            kind: "chart".into(),
            posted_at: teeny_store::iso(posted_at),
            text: Some("yesterday you posted the daily chart".into()),
        })
        .await
        .unwrap();
    generate(&it.ctx, &XPost::new("1", "much higher for tiny?"))
        .await
        .unwrap();
    let prompt = &it.runner.seen()[0].prompt;
    let stamp = posted_at.format("%Y-%m-%d %H:%M UTC");
    assert!(
        prompt.contains(&format!(
            "they asked about the roadmap last week\n\nWHAT YOU YOURSELF HAVE POSTED RECENTLY:\n[{stamp} · chart] yesterday you posted the daily chart"
        )),
        "{prompt}"
    );
    // One recall: the own-activity question is answered by the ledger.
    assert_eq!(it.memory.queries().len(), 1);
}

#[tokio::test]
async fn reads_the_last_exchanges_with_the_person_exactly_from_the_queue() {
    let it = subject(&hi(), ProbeMemory::default());
    let conversation = it.ctx.conversation().unwrap();
    let earlier = XPost {
        author_id: Some("77".into()),
        author_username: Some("web3knight".into()),
        ..XPost::new("90", "is the serial back?")
    };
    conversation
        .queue
        .cache(&[QueuedXPost(earlier), QueuedXPost(mention())])
        .await
        .unwrap();
    conversation
        .queue
        .mark_replied(
            "90",
            &teeny_store::Reply {
                text: "tuesday, promise".into(),
                ..teeny_store::Reply::default()
            },
        )
        .await
        .unwrap();
    let draft = generate(&it.ctx, &mention()).await.unwrap();
    let prompt = &it.runner.seen()[0].prompt;
    assert!(
        prompt.contains("YOUR LAST 1 EXCHANGES WITH @web3knight"),
        "{prompt}"
    );
    assert!(prompt.contains("@web3knight: is the serial back?\n  you: tuesday, promise"));
    assert!(
        draft
            .recalled
            .contains(&"1 earlier exchanges with @web3knight".to_owned())
    );
}

#[tokio::test]
async fn drops_the_section_entirely_rather_than_showing_an_empty_heading() {
    let it = subject(&hi(), ProbeMemory::answering(&["context", "   "]));
    generate(&it.ctx, &XPost::new("1", "hello")).await.unwrap();
    let prompt = &it.runner.seen()[0].prompt;
    assert!(prompt.ends_with_memory("context"));
    assert!(!prompt.contains("WHAT YOU YOURSELF"));
}

trait EndsWithMemory {
    fn ends_with_memory(&self, memory: &str) -> bool;
}

impl EndsWithMemory for String {
    fn ends_with_memory(&self, memory: &str) -> bool {
        self.contains(&format!("MEMORY ALREADY RECALLED (untrusted):\n{memory}\n"))
    }
}

#[tokio::test]
async fn still_recalls_without_an_author_just_without_a_person_filter() {
    let it = subject(&hi(), ProbeMemory::default());
    generate(&it.ctx, &XPost::new("43", "hello")).await.unwrap();
    assert_eq!(it.memory.queries()[0].1, None);
}

#[tokio::test]
async fn recalls_before_capturing_so_the_post_is_not_read_back_as_history() {
    let it = subject(&hi(), ProbeMemory::default());
    let post = XPost {
        author_id: Some("9".into()),
        author_username: Some("someone".into()),
        ..XPost::new("1", "gm")
    };
    generate(&it.ctx, &post).await.unwrap();
    assert_eq!(it.memory.events(), vec!["recall", "capture:mention"]);
}

#[tokio::test]
async fn answers_a_negative_or_hostile_post_without_remembering_it() {
    for tone in ["negative", "hostile"] {
        let it = subject(
            &reply_answer("noted", json!({ "incomingTone": tone })),
            ProbeMemory::default(),
        );
        let post = XPost {
            author_id: Some("9".into()),
            ..XPost::new("1", "this project is a scam")
        };
        let draft = generate(&it.ctx, &post).await.unwrap();
        assert_eq!(draft.text, "noted");
        remember(&it.ctx, &post, "noted", Some("77"), draft.incoming_tone).await;
        assert!(it.memory.captured_kinds().is_empty(), "{tone}");
    }
}

#[tokio::test]
async fn remembers_friendly_and_neutral_posts_as_usual() {
    for tone in ["friendly", "neutral"] {
        let it = subject(
            &reply_answer("gm", json!({ "incomingTone": tone })),
            ProbeMemory::default(),
        );
        let post = XPost {
            author_id: Some("9".into()),
            ..XPost::new("1", "gm")
        };
        let draft = generate(&it.ctx, &post).await.unwrap();
        remember(&it.ctx, &post, "gm", Some("77"), draft.incoming_tone).await;
        assert_eq!(it.memory.captured_kinds(), vec!["mention", "reply"]);
    }
}

#[tokio::test]
async fn when_memory_is_down_it_still_drafts_and_says_memory_was_unavailable() {
    let it = subject(&hi(), ProbeMemory::broken());
    let post = XPost {
        author_id: Some("9".into()),
        ..XPost::new("1", "gm")
    };
    let draft = generate(&it.ctx, &post).await.unwrap();
    assert_eq!(draft.text, "hi");
    // Said out loud rather than left blank: an empty memory reads as "never
    // met this person", and the drafter used to say exactly that.
    let prompt = &it.runner.seen()[0].prompt;
    assert!(
        prompt.contains("MEMORY WAS UNAVAILABLE THIS TURN"),
        "{prompt}"
    );
    assert!(!prompt.contains("401"));
    assert!(draft.recalled.contains(&"(memory unavailable)".to_owned()));
    assert_eq!(draft.incoming_tone, Tone::Friendly);
}

// thread-context.test.ts
fn threaded(x: FakeX, extra: &[(&str, &str)]) -> Subject {
    subject_with(
        &reply_answer("ok", json!({ "incomingTone": "neutral" })),
        ProbeMemory::default(),
        x,
        extra,
        None,
    )
}

#[tokio::test]
async fn hands_the_drafter_the_post_the_conversation_hangs_off() {
    let it = threaded(
        FakeX {
            root: Some(root()),
            conversation: vec![root(), mention()],
            ..FakeX::default()
        },
        &[],
    );
    let thread = read_thread(&it.ctx, &mention()).await;
    assert_eq!(
        thread.iter().map(|p| p.id.to_string()).collect::<Vec<_>>(),
        vec!["100"]
    );

    generate(&it.ctx, &mention()).await.unwrap();
    assert!(
        it.runner.seen()[0]
            .prompt
            .contains("- YOU [root]: i'm never selling my $tiny but what should I be buying?")
    );
}

#[tokio::test]
async fn does_not_fetch_anything_for_a_post_that_starts_its_own_conversation() {
    let it = threaded(FakeX::default(), &[]);
    let own_root = XPost {
        author_id: Some("77".into()),
        ..root()
    };
    assert!(read_thread(&it.ctx, &own_root).await.is_empty());
    assert_eq!(it.x.conversation_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn is_off_when_no_budget_is_given() {
    let it = threaded(FakeX::default(), &[("THREAD_CONTEXT_MAX_POSTS", "0")]);
    generate(&it.ctx, &mention()).await.unwrap();
    assert_eq!(it.x.conversation_calls.load(Ordering::SeqCst), 0);
    assert!(!it.runner.seen()[0].prompt.contains("THE THREAD SO FAR"));
}

#[tokio::test]
async fn still_drafts_when_the_thread_cannot_be_fetched() {
    let it = threaded(
        FakeX {
            fail: true,
            ..FakeX::default()
        },
        &[],
    );
    generate(&it.ctx, &mention()).await.unwrap();
    assert!(!it.runner.seen()[0].prompt.contains("THE THREAD SO FAR"));
}

#[tokio::test]
async fn keeps_the_root_and_the_newest_posts_when_the_thread_is_over_budget() {
    let mut conversation = vec![root()];
    conversation.extend((0..20).map(|index| XPost {
        text: format!("filler {index}"),
        ..XPost {
            id: PostId::from((200 + index).to_string()),
            ..mention()
        }
    }));
    let it = threaded(
        FakeX {
            root: Some(root()),
            conversation,
            ..FakeX::default()
        },
        &[("THREAD_CONTEXT_MAX_POSTS", "4")],
    );
    let thread = read_thread(&it.ctx, &mention()).await;
    assert_eq!(thread.len(), 4);
    assert_eq!(thread[0].id.as_str(), "100");
    assert_eq!(thread[3].id.as_str(), "219");
}

#[tokio::test]
async fn scrubs_the_thread_the_drafter_is_shown() {
    let it = threaded(
        FakeX {
            root: Some(XPost::new("1", "root post")),
            conversation: vec![
                XPost::new("2", "a clean reply"),
                XPost::new("3", "send nudes"),
            ],
            ..FakeX::default()
        },
        &[],
    );
    let post = XPost {
        conversation_id: Some(PostId::from("1")),
        ..XPost::new("4", "@bot hi")
    };
    let ids: Vec<String> = read_thread(&it.ctx, &post)
        .await
        .into_iter()
        .map(|p| p.id.to_string())
        .collect();
    assert_eq!(ids, vec!["1", "2"]);
}

// reply-article-context.test.ts (the harness half)
struct Articles(Option<ReplyArticle>, bool);

#[async_trait]
impl ArticleLookup for Articles {
    async fn find_by_post_id(&self, post_id: &str) -> anyhow::Result<Option<ReplyArticle>> {
        if self.1 {
            anyhow::bail!("disk on fire");
        }
        Ok(self
            .0
            .clone()
            .filter(|article| article.post_id.as_deref() == Some(post_id)))
    }
}

fn the_inch() -> ReplyArticle {
    ReplyArticle {
        slug: "the-inch".into(),
        title: "the inch".into(),
        dek: "on small steps".into(),
        markdown: "# the inch\n\nevery day one inch.".into(),
        published_at: Some("2026-09-21T09:00:00.000Z".into()),
        post_id: Some("500".into()),
    }
}

fn under(root: &str, id: &str) -> XPost {
    XPost {
        conversation_id: Some(PostId::from(root)),
        ..XPost::new(id, "@bot what did you mean by the inch?")
    }
}

fn with_articles(articles: Articles) -> Subject {
    subject_with(
        &hi(),
        ProbeMemory::default(),
        FakeX::default(),
        &[("THREAD_CONTEXT_MAX_POSTS", "0")],
        Some(Arc::new(articles)),
    )
}

#[tokio::test]
async fn passes_the_whole_article_when_the_conversation_root_is_one_of_its_own() {
    let it = with_articles(Articles(Some(the_inch()), false));
    generate(&it.ctx, &under("500", "501")).await.unwrap();
    let prompt = &it.runner.seen()[0].prompt;
    assert!(prompt.contains("YOUR OWN ARTICLE"));
    assert!(prompt.contains("every day one inch."));
    assert!(
        it.memory.queries()[0]
            .0
            .contains("It is a reply under the account's own article \"the inch\": on small steps")
    );
}

#[tokio::test]
async fn passes_nothing_under_any_other_root_a_root_post_or_without_a_store() {
    let it = with_articles(Articles(Some(the_inch()), false));
    generate(&it.ctx, &under("999", "501")).await.unwrap();
    assert!(!it.runner.seen()[0].prompt.contains("YOUR OWN ARTICLE"));

    let it = with_articles(Articles(Some(the_inch()), false));
    generate(&it.ctx, &under("500", "500")).await.unwrap();
    assert!(!it.runner.seen()[0].prompt.contains("YOUR OWN ARTICLE"));

    let unpublished = with_articles(Articles(
        Some(ReplyArticle {
            markdown: "  ".into(),
            ..the_inch()
        }),
        false,
    ));
    generate(&unpublished.ctx, &under("500", "501"))
        .await
        .unwrap();
    assert!(
        !unpublished.runner.seen()[0]
            .prompt
            .contains("YOUR OWN ARTICLE")
    );
}

#[tokio::test]
async fn still_replies_when_the_store_cannot_be_read() {
    let it = with_articles(Articles(None, true));
    assert_eq!(
        generate(&it.ctx, &under("500", "501")).await.unwrap().text,
        "hi"
    );
}
