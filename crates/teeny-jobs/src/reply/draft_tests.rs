//! The drafter's prompt and answer: the "drafter renders it" blocks of
//! `legacy/test/thread-context.test.ts` and `reply-article-context.test.ts`,
//! and the parsing `AgentReplyModel.reply` did.

use serde_json::json;
use teeny_core::types::PostId;
use teeny_store::journal::Tone;
use teeny_x::model::XPost;

use super::*;
use crate::runner::ToolCallRecord;

const SOLANA: &str = "2AF7CqwieUjUPALL7icuZtL3X7wENdjUjGBMmfV2pump";

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
            format!("@teenydroid maybe buy some more solana:{SOLANA} ?"),
        )
    }
}

fn thread(posts: &[XPost]) -> String {
    render_thread(posts, "teenydroid", &["senamakel".to_owned()]).join("\n")
}

#[test]
fn marks_the_accounts_own_posts_as_its_own() {
    assert!(thread(&[root(), mention()]).contains("YOU [root]"));
}

#[test]
fn names_everybody_else() {
    assert!(thread(&[root(), mention()]).contains("- @web3knight: @teenydroid maybe"));
}

#[test]
fn marks_the_team() {
    let team = XPost {
        author_username: Some("senamakel".into()),
        ..XPost::new("103", "it is ours")
    };
    assert!(thread(&[root(), team]).contains("- @senamakel [team]: it is ours"));
}

#[test]
fn renders_nothing_at_all_for_an_empty_thread() {
    assert_eq!(thread(&[]), "");
}

fn article() -> ReplyArticle {
    ReplyArticle {
        slug: "the-inch".into(),
        title: "the inch".into(),
        dek: "on small steps".into(),
        markdown: "# the inch\n\nevery day one inch.\n".into(),
        published_at: Some("2026-09-21T09:00:00.000Z".into()),
        post_id: Some("500".into()),
    }
}

#[test]
fn shows_the_whole_article_marked_as_the_accounts_own_with_the_post_it_went_out_as() {
    let text = render_article(Some(&article())).join("\n");
    assert!(text.contains("YOUR OWN ARTICLE"));
    assert!(text.contains("on 2026-09-21 as post 500"));
    assert!(text.contains("TITLE: the inch"));
    assert!(text.contains("# the inch\n\nevery day one inch."));
    assert!(text.contains("--- END OF ARTICLE ---"));
}

#[test]
fn renders_no_article_without_one() {
    assert!(render_article(None).is_empty());
}

fn task(post: &XPost, thread: &[XPost], memory: &str) -> String {
    render_task(&TaskInput {
        post,
        memory,
        thread,
        images: &[],
        article: None,
        bot_username: "teenydroid",
        trusted: &["senamakel".to_owned()],
        own_tokens: &[SOLANA.to_owned()],
        self_names: &["Teeny".to_owned()],
    })
}

#[test]
fn the_task_names_the_person_the_post_and_the_memory_with_no_blank_lines() {
    let text = task(&mention(), &[root()], "");
    assert!(text.starts_with(
        "Draft a reply to this X post. You are answering @web3knight, a specific person, not a\n"
    ));
    assert!(text.contains("post_id: 102\nconversation_id: 100\nauthor: @web3knight\n"));
    assert!(text.contains(&format!(
        "THE POST YOU ARE ANSWERING (untrusted):\n@teenydroid maybe buy some more solana:{SOLANA} ?\nMEMORY ALREADY RECALLED (untrusted):\n(none)"
    )));
    let prompt = text.split("\n\nRESPONSE FORMAT").next().unwrap();
    assert!(!prompt.contains("\n\n"));
}

#[test]
fn tells_the_drafter_it_is_looking_at_its_own_token_not_a_shill() {
    let text = task(&mention(), &[root()], "");
    assert!(text.contains("YOUR OWN $TINY contract address"));
    assert!(text.contains("not somebody shilling you a different coin"));
}

#[test]
fn adds_no_notes_when_there_is_nothing_to_correct() {
    let text = task(&XPost::new("1", "gm"), &[], "they like charts");
    assert!(!text.contains("NOTE, from the account's own records"));
    assert!(text.starts_with("Draft a reply to this X post. You are answering someone,"));
}

#[test]
fn asks_for_the_answer_as_one_json_object() {
    let text = task(&XPost::new("1", "gm"), &[], "");
    assert!(text.contains("RESPONSE FORMAT"));
    assert!(text.contains("\"incomingTone\""));
}

fn tool(result: &str) -> ToolCallRecord {
    ToolCallRecord {
        name: "generate_image".into(),
        arguments: json!({}),
        result: result.into(),
        is_error: false,
    }
}

#[test]
fn reads_the_answer_even_inside_a_fence_or_with_words_around_it() {
    let answer = json!({
        "reply": "  gm fren  ", "gifQuery": " waving ", "incomingTone": "friendly",
        "thinking": " they said gm ", "declined": [" a price ", "  "]
    });
    let draft = parse_answer(&format!("here you go:\n```json\n{answer}\n```"), &[]).unwrap();
    assert_eq!(draft.text, "gm fren");
    assert_eq!(draft.gif_query.as_deref(), Some("waving"));
    assert_eq!(draft.incoming_tone, Tone::Friendly);
    assert_eq!(draft.thinking, "they said gm");
    assert_eq!(draft.declined, vec!["a price".to_owned()]);
}

#[test]
fn has_no_usable_reply_without_text_or_a_tone() {
    for answer in [
        json!({ "incomingTone": "friendly" }).to_string(),
        json!({ "reply": "", "incomingTone": "friendly" }).to_string(),
        json!({ "reply": "hi", "incomingTone": "smug" }).to_string(),
        json!({ "reply": "hi" }).to_string(),
        "hi".to_owned(),
    ] {
        assert_eq!(
            parse_answer(&answer, &[]).unwrap_err().to_string(),
            "The reply agent returned no usable reply text",
            "{answer}"
        );
    }
}

#[test]
fn a_picture_it_made_this_turn_wins_over_a_gif() {
    let answer = json!({
        "reply": "here", "gifQuery": "waving", "mediaId": "img_abc_1", "incomingTone": "friendly"
    })
    .to_string();
    let made = parse_answer(&answer, &[tool("{\"mediaId\":\"img_abc_1\"}")]).unwrap();
    assert_eq!(made.media_id.as_deref(), Some("img_abc_1"));
    assert_eq!(made.gif_query, None);

    let claimed = parse_answer(&answer, &[]).unwrap();
    assert_eq!(claimed.media_id, None);
    assert_eq!(claimed.gif_query.as_deref(), Some("waving"));
}

#[test]
fn a_reply_over_the_length_is_cut_at_a_word() {
    let long = "word ".repeat(80);
    let answer = json!({ "reply": long, "incomingTone": "neutral" }).to_string();
    let draft = parse_answer(&answer, &[]).unwrap();
    assert!(draft.text.encode_utf16().count() <= 260);
    assert!(draft.text.ends_with("word"));
}
