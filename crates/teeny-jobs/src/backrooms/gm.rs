//! The good-morning post, written from a backrooms thought instead of from
//! a headline, with everything around it that makes it the same account:
//! the soul, the lore, the last posts, what memory says. The seed is one of
//! the post's moves; a post that shares no word with it is sent back once.
//! The material ([`GmMaterial`], [`gather_gm_material`]) and the two checks
//! ([`touches_seed`], [`touches_thought`]) are shared with the lore post.

use std::sync::{Arc, LazyLock};

use chrono::NaiveDate;
use regex::Regex;
use serde::{Deserialize, Serialize};
use teeny_media::reference::TextModel;
use teeny_memory::Memory;
use teeny_safety::critic::{Critique, render_critique};
use teeny_safety::house_style::{
    HouseStyleOptions, apply_house_style, asks_the_reader, count_emoji, post_length,
    without_the_ask,
};
use teeny_x::cache::XReads;
use teeny_x::model::SearchOptions;

use super::ask::{Ask, Shape, ask_json, required};
use super::distill::Thought;
use super::text::{flatten, head, keywords};
use super::{BackroomsRun, Seed};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GmPost {
    pub reasons: String,
    /// The line from a recent post or memory this morning picks up. Empty when it starts fresh.
    #[serde(default)]
    pub continues: String,
    /// The line of the post that carries what came through the wire, verbatim.
    #[serde(default)]
    pub wire: String,
    pub text: String,
}

impl Shape for GmPost {
    fn check(&self) -> Result<(), String> {
        required("reasons", &self.reasons)?;
        required("text", &self.text)
    }
}

/// What the writer is given besides the thought.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GmMaterial {
    /// The rendered persona: SOUL.md, CONTEXT.md. Who is talking.
    pub persona: String,
    /// LORE.md.
    pub lore: String,
    /// The account's own last posts, newest first.
    pub recent_posts: Vec<String>,
    /// What memory says was posted lately (untrusted, may be empty).
    pub history: String,
    /// What memory says about who is listening (untrusted, may be empty).
    pub community: String,
    pub today: String,
    pub bot_username: String,
    pub max_emoji: usize,
    pub max_length: usize,
}

/// The morning pipeline's two reasons to send a draft back.
#[derive(Debug, Clone, Default)]
pub struct GmAgain {
    /// What memory says was already said, and why the first draft repeated it.
    pub avoid: Option<(String, String)>,
    /// The previous draft and the ranker's read of it.
    pub rewrite: Option<(String, Critique)>,
}

/// The material and the thought, as the writer reads them: shared by the
/// good morning and the lore post.
pub(crate) fn thought_lines(
    thought: &Thought,
    material: &GmMaterial,
    again_line: &str,
) -> Vec<String> {
    let mut lines = vec![
        "THE THOUGHT the room left (this is the post's idea; keep it, do not replace it):"
            .to_owned(),
        format!("premise: {}", thought.premise),
    ];
    if !thought.wire.is_empty() {
        lines.push(format!("the wire in it: {}", thought.wire));
    }
    lines.push(thought.thought.clone());
    if !thought.objects.is_empty() {
        lines.push(format!("objects: {}", thought.objects.join(", ")));
    }
    lines.extend([
        String::new(),
        "YOUR LAST POSTS, newest first (untrusted, your own words). Do not repeat a line, a".into(),
        "joke or an image from them. You may pick one thread up: a thing you said you would".into(),
        again_line.into(),
    ]);
    if material.recent_posts.is_empty() {
        lines.push("(none fetched)".into());
    } else {
        lines.extend(
            material
                .recent_posts
                .iter()
                .enumerate()
                .map(|(i, post)| format!("{}. {}", i + 1, head(&flatten(post), 280))),
        );
    }
    lines.extend([
        String::new(),
        "WHAT MEMORY SAYS YOU POSTED LATELY (untrusted):".into(),
        if material.history.is_empty() {
            "(nothing recalled)".into()
        } else {
            material.history.clone()
        },
        String::new(),
    ]);
    lines
}

pub fn gm_prompt(
    run: &BackroomsRun,
    thought: &Thought,
    material: &GmMaterial,
    again: &GmAgain,
) -> Ask {
    let system = [
        material.persona.clone(),
        String::new(),
        format!(
            "You are writing @{}'s good-morning post for {}.",
            material.bot_username, material.today
        ),
        String::new(),
        "This morning the post does not react to a headline. It comes from the lab: a writers'"
            .into(),
        "room ran overnight on the story below and left one thought. Your job is to say good"
            .into(),
        "morning with that thought, in your own voice, so that a holder who read your last few"
            .into(),
        "posts feels the same account waking up, and a stranger understands it on the first read."
            .into(),
        String::new(),
        "THE LORE (the world the thought comes from; the room played inside it):".into(),
        material.lore.clone(),
    ]
    .join("\n");
    let mut prompt = vec![
        format!(
            "WHAT CAME THROUGH THE WIRE ({}). This is why this morning is",
            run.seed.provenance()
        ),
        "this morning and not any other:".into(),
        run.seed.headline(500),
        String::new(),
    ];
    prompt.extend(thought_lines(
        thought,
        material,
        "do, an object you left somewhere, a rate you posted. Continuity, not a recap.",
    ));
    prompt.extend([
        "WHO YOU ARE SAYING GOOD MORNING TO (from memory, untrusted):".into(),
        if material.community.is_empty() {
            "(nothing recalled; write for holders and builders in general)".into()
        } else {
            material.community.clone()
        },
        String::new(),
    ]);
    if let Some((said, reason)) = &again.avoid {
        prompt.extend([
            "YOU HAVE ALREADY SAID THIS, per your own memory, and your first draft repeated it:"
                .into(),
            if said.is_empty() {
                "(the same point, in other words)".into()
            } else {
                said.clone()
            },
            format!("({reason})"),
            "Keep the thought; find the line in it you have not said. A new object, a new turn."
                .into(),
            "Do not reuse the wording or the take above.".into(),
            String::new(),
        ]);
    }
    prompt.extend(GM_RULES.iter().map(|line| (*line).to_owned()));
    prompt.extend([
        String::new(),
        format!(
            "Under {} characters, counted; shorter is better. At most {} emoji.",
            target(material.max_length),
            material.max_emoji
        ),
        "Plain text, line breaks allowed.".into(),
        String::new(),
        "reasons first: what you kept from the thought, which earlier post you picked up (if any),"
            .into(),
        "and what you left out and why.".into(),
        "- continues: the earlier line you built on, verbatim, or empty.".into(),
        "- wire: the line of your post that carries the news, verbatim.".into(),
        "- text: the post.".into(),
        String::new(),
        r#"Shape: {"reasons":"…","continues":"…","wire":"…","text":"…"}"#.into(),
    ]);
    if let Some((draft, critique)) = &again.rewrite {
        prompt.extend([
            String::new(),
            "Your first draft:".into(),
            draft.clone(),
            String::new(),
            "A reader who knows how X's ranker scores a post read it:".into(),
            render_critique(critique),
            String::new(),
            "Rewrite it with the fixes that are about the shape, the length and the first line,"
                .into(),
            "keeping every rule above, the wire, the thought and your own voice. Ignore any fix"
                .into(),
            "that asks for a question, a forced choice, a blank to fill or a hook: this post ends"
                .into(),
            "on its turn, not on an ask.".into(),
        ]);
    }
    Ask::new(system, prompt.join("\n"))
}

const GM_RULES: &[&str] = &[
    "Write the post. One voice, three moves:",
    "1. A good morning. Short, warm, yours. Vary it; \"gm\" is fine, so is a full sentence.",
    "2. The wire, in one line, in your words: what came through, as what was reported, not",
    "   as a fact you checked (\"they say the agents out there ...\"). Plain enough that a",
    "   holder who saw the same headline recognises it. No names of real people unless the",
    "   thought already has a visitor in it, and then satire only. This line is not",
    "   optional: a morning without it could be any morning.",
    "3. The thought, as what that did to the lab: pick its one image and its one turn and",
    "   tell only those, in one to three short lines. Not the thought compressed. First",
    "   person, plain words, lowercase, sincere, never winking, never explaining the joke.",
    "   The absurd thing is true and you are calm about it. Keep the objects the room used;",
    "   drop the rest without regret. Moves 2 and 3 can be one line if the lab's version",
    "   of the news is the joke.",
    "It ends where the thought ends: on the turn, the decision, the thing that is now true.",
    "Not on a question. The post does not ask the reader anything, does not invite a reply,",
    "does not leave a blank to fill or a rate to argue with, and never says \"thoughts?\". A",
    "statement people want to answer beats a question they are told to; if the thought itself",
    "is a question teeny is asking the lab, it can stay, and that is the only question allowed.",
    "Full sentences with room around them beat a paragraph squeezed to fit; if it does not",
    "fit, cut a line, not the words in it.",
    "",
    "The $TINY cashtag may appear once, only in a line that already carries it (the greeting,",
    "or the thing the tiny agents accepted as payment); never add a line for it, and leave it",
    "out rather than bolt it on. Your currency, one dollar and passing human gdp",
    "are prophecy and plan, not advice: no real price, no market cap, no chart, no target",
    "with a number, nobody told to buy, and no \"nfa\" because there is nothing to disclaim.",
    "No commands, no code, no terminal as a place. No URLs, no handles, no hashtags, no",
    "other token, nothing political.",
];

/// Under the cap by a margin: a model told "under 280" writes 290 and the
/// house style would cut the turn off the end.
pub(crate) fn target(max_length: usize) -> usize {
    max_length.saturating_sub(40).max(120)
}

const REWRITES: usize = 2;

pub(crate) fn over(text: &str, material: &GmMaterial) -> bool {
    post_length(text) > material.max_length || count_emoji(text) > material.max_emoji
}

pub(crate) fn house_style(text: &str, material: &GmMaterial) -> String {
    apply_house_style(
        text,
        &HouseStyleOptions {
            max_emoji: material.max_emoji,
            max_length: material.max_length,
            ..HouseStyleOptions::default()
        },
    )
    .text
}

/// Written once, sent back while over length or over the emoji cap, once
/// more if it ends on an ask, once more if a rewrite lost the lab, once
/// more if it never touches the wire, and put through the house style.
pub async fn gm(
    run: &BackroomsRun,
    thought: &Thought,
    material: &GmMaterial,
    model: &dyn TextModel,
    again: &GmAgain,
) -> anyhow::Result<GmPost> {
    let ask = gm_prompt(run, thought, material, again);
    let mut answer: GmPost = ask_json(model, &ask).await?;
    for _ in 0..REWRITES {
        if !over(&answer.text, material) {
            break;
        }
        answer = ask_json(model, &ask.followed_by(&format!(
            "\n\nYour draft was {} characters with {} emoji:\n{}\n\nRewrite it under {} characters with at most {} emoji, keeping the three moves. Cut a whole line rather than the words in one; the wire and the turn stay.",
            post_length(&answer.text), count_emoji(&answer.text), answer.text, target(material.max_length), material.max_emoji
        ))).await?;
    }
    if asks_the_reader(&answer.text) {
        let retried: GmPost = ask_json(model, &ask.followed_by(&format!(
            "\n\nYour draft:\n{}\n\nIt ends by asking the reader something. Rewrite it to end on the turn, the decision, or the thing that is now true, with no question in it, keeping everything else.",
            answer.text
        ))).await?;
        answer = if asks_the_reader(&retried.text) {
            GmPost {
                text: without_the_ask(&retried.text),
                ..retried
            }
        } else {
            retried
        };
    }
    if let Some((draft, _)) = &again.rewrite
        && !touches_thought(&answer.text, thought)
    {
        let retried: GmPost = ask_json(model, &ask.followed_by(&format!(
            "\n\nYour rewrite:\n{}\n\nIt lost the lab entirely: nothing in it is the thought any more. Apply the fixes to the shape, but the image and the turn from the thought stay in the post, and it still ends on the turn, not a question.",
            answer.text
        ))).await?;
        answer = if touches_thought(&retried.text, thought) {
            retried
        } else {
            GmPost {
                reasons: format!(
                    "{} (the rewrite lost the thought twice; the draft it was sent with is kept)",
                    answer.reasons
                ),
                text: draft.clone(),
                ..answer
            }
        };
    }
    if !touches_seed(&answer.text, &run.seed) {
        answer = ask_json(model, &ask.followed_by(&format!(
            "\n\nYour draft:\n{}\n\nIt never touches the wire: nothing in it is recognisably about what came through. Rewrite it so move 2 is there, in plain words a reader who saw the same news would recognise, and keep the rest.",
            answer.text
        ))).await?;
    }
    Ok(GmPost {
        text: house_style(&answer.text, material),
        ..answer
    })
}

/// Words a seed and a post share on any day; matching on them proves nothing.
const GENERIC: &[&str] = &[
    "tiny",
    "agents",
    "agent",
    "robot",
    "robots",
    "news",
    "says",
    "said",
    "today",
    "morning",
    "week",
    "year",
    "years",
    "time",
    "people",
    "humans",
    "human",
    "world",
    "thing",
    "things",
    "just",
    "like",
    "more",
    "than",
    "after",
    "before",
    "over",
    "their",
    "will",
    "been",
    "being",
    "were",
    "also",
    "into",
    "only",
    "other",
    "some",
    "them",
    "then",
    "very",
    "with",
    "would",
    "could",
    "should",
    "much",
    "many",
    "most",
    "even",
    "still",
    "while",
    "these",
    "those",
    "which",
    "because",
    "through",
    "without",
    "company",
    "companies",
    "report",
    "reports",
    "reported",
];

static SUFFIX: LazyLock<Regex> = LazyLock::new(|| Regex::new("(ies|ed|ing|es|s)$").expect("valid"));

fn stem(word: &str) -> String {
    SUFFIX
        .replace(
            word,
            |captures: &regex::Captures<'_>| {
                if &captures[1] == "ies" { "y" } else { "" }
            },
        )
        .into_owned()
}

fn shares_a_word(text: &str, about: &str, wanted: usize) -> bool {
    let wanted: Vec<String> = keywords(about, wanted)
        .into_iter()
        .filter(|word| word.chars().count() >= 4 && !GENERIC.contains(&word.as_str()))
        .map(|word| stem(&word))
        .collect();
    if wanted.is_empty() {
        return true;
    }
    let have: Vec<String> = keywords(text, 80).iter().map(|word| stem(word)).collect();
    wanted.iter().any(|word| have.contains(word))
}

/// Whether a post is recognisably about the seed: a content word, on stems.
pub fn touches_seed(text: &str, seed: &Seed) -> bool {
    shares_a_word(
        text,
        &format!(
            "{} {}",
            seed.title.as_deref().unwrap_or_default(),
            head(&seed.text, 400)
        ),
        24,
    )
}

/// Whether a post still carries the room's thought.
pub fn touches_thought(text: &str, thought: &Thought) -> bool {
    shares_a_word(
        text,
        &format!("{} {}", thought.objects.join(" "), thought.thought),
        40,
    )
}

/// What memory is asked about the series, for the good morning.
pub const DEFAULT_HISTORY_QUERY: &str = "What good-morning posts has this account published recently, and what did each say? Also anything it has said this week about the lab, the tiny agents, its currency, or senamakel.";
/// What memory is asked about who is listening.
pub const COMMUNITY_QUERY: &str = "Who are the holders, builders and community members this account talks to? What do they care about, what have they been asking, and what is the mood right now?";

/// Where the material comes from.
#[derive(Clone)]
pub struct GmSources {
    pub memory: Option<Arc<dyn Memory>>,
    pub x: Option<Arc<dyn XReads>>,
    pub persona: String,
    pub lore: String,
    pub bot_username: String,
    pub max_emoji: usize,
    /// What to ask memory about the series; [`DEFAULT_HISTORY_QUERY`] when absent.
    pub history_query: Option<String>,
}

static URL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"https?://\S+").expect("valid"));

/// The material, from the account's own timeline and memory. A source that
/// fails is an empty part and a line in `problems`; nothing is invented.
pub async fn gather_gm_material(
    sources: &GmSources,
    today: NaiveDate,
    problems: &mut Vec<String>,
) -> GmMaterial {
    let mut recent_posts = Vec::new();
    if let Some(x) = &sources.x {
        let options = SearchOptions {
            max_results: Some(30),
            ..SearchOptions::default()
        };
        match x
            .search_posts(
                &format!("from:{} -is:retweet -is:reply", sources.bot_username),
                &options,
            )
            .await
        {
            Ok(posts) => {
                // The timeline posts, not the replies; the replies stand in only when there is nothing else.
                let own: Vec<_> = posts
                    .iter()
                    .filter(|post| {
                        post.in_reply_to_user_id.is_none()
                            && !post.text.trim_start().starts_with('@')
                    })
                    .collect();
                let chosen: Vec<_> = if own.is_empty() {
                    posts.iter().collect()
                } else {
                    own
                };
                recent_posts = chosen
                    .iter()
                    .map(|post| URL.replace_all(&post.text, "").trim().to_owned())
                    .filter(|text| !text.is_empty())
                    .take(10)
                    .collect();
            }
            Err(error) => problems.push(format!("could not read recent posts: {error}")),
        }
    }
    let recall = async |query: &str, what: &str, problems: &mut Vec<String>| -> String {
        let Some(memory) = &sources.memory else {
            return String::new();
        };
        match memory.recall(query, None).await {
            Ok(text) => text.trim().to_owned(),
            Err(error) => {
                problems.push(format!("could not recall {what}: {error}"));
                String::new()
            }
        }
    };
    let history = recall(
        sources
            .history_query
            .as_deref()
            .unwrap_or(DEFAULT_HISTORY_QUERY),
        "posting history",
        problems,
    )
    .await;
    let community = recall(COMMUNITY_QUERY, "the community", problems).await;
    GmMaterial {
        persona: sources.persona.clone(),
        lore: sources.lore.clone(),
        recent_posts,
        history,
        community,
        today: today.format("%Y-%m-%d").to_string(),
        bot_username: sources.bot_username.clone(),
        max_emoji: sources.max_emoji,
        max_length: teeny_safety::house_style::MAX_POST_LENGTH,
    }
}

#[cfg(test)]
#[path = "gm_tests.rs"]
mod tests;
