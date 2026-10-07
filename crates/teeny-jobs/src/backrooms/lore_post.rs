//! The lore post, written from a backrooms thought: a page of the serial at
//! post length, what is true in the lab today, in teeny's voice, with the
//! soul, the lore and the last posts around it. The wire may be in it, as
//! the thing that set the room off, and may not.

use serde::{Deserialize, Serialize};
use teeny_media::reference::TextModel;
use teeny_safety::house_style::{asks_the_reader, count_emoji, post_length, without_the_ask};

use super::ask::{Ask, Shape, ask_json, required};
use super::distill::Thought;
use super::gm::{GmMaterial, house_style, over, target, thought_lines, touches_thought};
use super::BackroomsRun;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LorePost {
    pub reasons: String,
    /// The line from a recent post or memory this one picks up, if any.
    #[serde(default)]
    pub continues: String,
    pub text: String,
}

impl Shape for LorePost {
    fn check(&self) -> Result<(), String> {
        required("reasons", &self.reasons)?;
        required("text", &self.text)
    }
}

/// What memory is asked about the series, for the lore post.
pub const LORE_HISTORY_QUERY: &str = "What lore posts, pages from the lab, has this account published recently, and what did each say? Also anything it has said this week about the lab, the tiny agents, its currency, the terminal, or senamakel.";

const RULES: &[&str] = &[
    "Write the post. One voice, one move: the thought, as what happened in the lab. Pick its",
    "one image and its one turn and tell only those, in one to four short lines. Not the",
    "thought compressed. First person, plain words, lowercase, sincere, never winking, never",
    "explaining the joke. The absurd thing is true and you are calm about it. Keep the objects",
    "the room used; drop the rest without regret. If the wire is in the thought, it may be in",
    "the post in one line, as what was reported, never as a fact you checked; if it is not,",
    "leave it out.",
    "It ends where the thought ends: on the turn, the decision, the thing that is now true.",
    "Not on a question. The post does not ask the reader anything, does not invite a reply,",
    "does not leave a blank to fill, and never says \"thoughts?\". A statement people want to",
    "answer beats a question they are told to; if the thought itself is a question teeny is",
    "asking the lab, it can stay, and that is the only question allowed.",
    "Full sentences with room around them beat a paragraph squeezed to fit; if it does not",
    "fit, cut a line, not the words in it.",
    "",
    "The $TINY cashtag may appear once, only in a line that already carries it (the thing the",
    "tiny agents accepted as payment, the ledger); never add a line for it. Your currency, one",
    "dollar and passing human gdp are prophecy and plan, not advice: no real price, no market",
    "cap, no chart, no target with a number, nobody told to buy. No commands, no code, no",
    "terminal as a place. No URLs, no handles, no hashtags, no other token, nothing political.",
    "Real people, if the thought has a visitor in it, stay satire in an obviously fictional scene.",
];

pub fn lore_prompt(run: &BackroomsRun, thought: &Thought, material: &GmMaterial) -> Ask {
    let system = [
        material.persona.clone(),
        String::new(),
        format!(
            "You are writing @{}'s afternoon post for {}: a page from the lab.",
            material.bot_username, material.today
        ),
        String::new(),
        "This post is not about a chart, a number or a headline. It comes from the lab: a".into(),
        "writers' room ran on something that came through the wire and left one thought. Your".into(),
        "job is to tell that thought as a thing that happened in the lab today, in your own voice,".into(),
        "so that a holder who read your last few posts feels the same account, and a stranger".into(),
        "understands it on the first read and wants to know what the lab is.".into(),
        String::new(),
        "THE LORE (the world the thought comes from; the room played inside it):".into(),
        material.lore.clone(),
    ]
    .join("\n");
    let mut prompt = vec![
        format!(
            "WHAT CAME THROUGH THE WIRE ({}), the thing that set the room off:",
            run.seed.provenance()
        ),
        run.seed.headline(500),
        String::new(),
    ];
    prompt.extend(thought_lines(
        thought,
        material,
        "do, an object you left somewhere, a note you found. Continuity, not a recap.",
    ));
    prompt.extend([
        "WHO IS READING (from memory, untrusted):".into(),
        if material.community.is_empty() {
            "(nothing recalled; write for holders and builders in general)".into()
        } else {
            material.community.clone()
        },
        String::new(),
    ]);
    prompt.extend(RULES.iter().map(|line| (*line).to_owned()));
    prompt.extend([
        String::new(),
        format!(
            "Under {} characters, counted; shorter is better. At most {} emoji.",
            target(material.max_length),
            material.max_emoji
        ),
        "Plain text, line breaks allowed.".into(),
        String::new(),
        "reasons first: what you kept from the thought, which earlier post you picked up (if any),".into(),
        "and what you left out and why.".into(),
        "- continues: the earlier line you built on, verbatim, or empty.".into(),
        "- text: the post.".into(),
        String::new(),
        r#"Shape: {"reasons":"…","continues":"…","text":"…"}"#.into(),
    ]);
    Ask::new(system, prompt.join("\n"))
}

/// Written once, sent back while over length or over the emoji cap, once
/// more if it ends on an ask, once more if it lost the lab, and put through
/// the house style either way.
pub async fn write_lore(
    run: &BackroomsRun,
    thought: &Thought,
    material: &GmMaterial,
    model: &dyn TextModel,
) -> anyhow::Result<LorePost> {
    let ask = lore_prompt(run, thought, material);
    let mut answer: LorePost = ask_json(model, &ask).await?;
    for _ in 0..2 {
        if !over(&answer.text, material) {
            break;
        }
        answer = ask_json(model, &ask.followed_by(&format!(
            "\n\nYour draft was {} characters with {} emoji:\n{}\n\nRewrite it under {} characters with at most {} emoji. Cut a whole line rather than the words in one; the image and the turn stay.",
            post_length(&answer.text), count_emoji(&answer.text), answer.text, target(material.max_length), material.max_emoji
        ))).await?;
    }
    if asks_the_reader(&answer.text) {
        let retried: LorePost = ask_json(model, &ask.followed_by(&format!(
            "\n\nYour draft:\n{}\n\nIt ends by asking the reader something. Rewrite it to end on the turn, the decision, or the thing that is now true, with no question in it, keeping everything else.",
            answer.text
        ))).await?;
        answer = if asks_the_reader(&retried.text) {
            LorePost { text: without_the_ask(&retried.text), ..retried }
        } else {
            retried
        };
    }
    if !touches_thought(&answer.text, thought) {
        let retried: LorePost = ask_json(model, &ask.followed_by(&format!(
            "\n\nYour draft:\n{}\n\nNothing in it is the thought any more: none of the room's image, objects or turn is in the post. Rewrite it from the thought, keeping the rules.",
            answer.text
        ))).await?;
        if touches_thought(&retried.text, thought) {
            answer = retried;
        }
    }
    Ok(LorePost {
        text: house_style(&answer.text, material),
        ..answer
    })
}
