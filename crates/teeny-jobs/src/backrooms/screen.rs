//! What scraped text has to pass before it is allowed into an agent's
//! context or a tool result. The narrative direction of the lexicon lets
//! swearing through and keeps every hard category; the political filter is
//! the morning post's. 4chan is where the slurs are, and a slur that gets
//! into the room gets said back.

use std::sync::LazyLock;

use regex::Regex;
use teeny_safety::screen::{Direction, screen_text};

use super::{ForumPost, Site};

/// The morning's political terms (`POLITICAL_TERMS` in `legacy/src/morning/news.ts`).
pub const POLITICAL_TERMS: &[&str] = &[
    "election", "elections", "electoral", "ballot", "president", "presidential", "prime minister",
    "senate", "senator", "lawmaker", "lawmakers", "house speaker", "policymaker", "policymakers",
    "minister", "ministry", "federal", "feds", "treasury", "justice department", "north korea",
    "north korean", "congress", "congressional", "parliament", "legislation", "legislature",
    "governor", "white house", "kremlin", "republican", "republicans", "democrat", "democrats",
    "gop", "labour party", "tory", "tories", "trump", "biden", "harris", "putin", "zelensky",
    "netanyahu", "xi jinping", "modi", "war", "warfare", "invasion", "ceasefire", "missile",
    "missiles", "airstrike", "airstrikes", "troops", "military", "gaza", "israel", "israeli",
    "palestine", "palestinian", "ukraine", "ukrainian", "russia", "russian", "iran", "iranian",
    "taiwan", "nato", "tariff", "tariffs", "sanction", "sanctions", "immigration", "immigrants",
    "deportation", "abortion", "protest", "protests", "protesters", "riot", "riots", "impeach",
    "impeachment", "indictment", "indicted", "lawsuit", "supreme court", "regulator",
    "regulators", "regulation", "regulatory", "doj", "fbi", "cia", "shooting", "terror",
    "terrorist", "genocide",
];

static POLITICAL: LazyLock<Regex> = LazyLock::new(|| {
    let terms = POLITICAL_TERMS
        .iter()
        .map(|term| regex::escape(term.trim()))
        .collect::<Vec<_>>()
        .join("|");
    Regex::new(&format!("(?i)(?:^|[^a-z])(?:{terms})(?:$|[^a-z])")).expect("valid")
});

/// Groups of people the room has no business riffing on. The lexicon catches
/// slurs; this catches the thread that is about a religion or an ethnicity
/// without using one, which on the boards is most of them.
static SENSITIVE: LazyLock<Regex> = LazyLock::new(|| {
    let terms = [
        "jew", "jews", "jewish", "zionist", "zionism", "israel", "israeli", r"palestin\w*", "gaza",
        "holocaust", "muslim", "muslims", "islam", "islamic", "islamist", "quran", "koran",
        "sharia", r"jihad\w*", "christian", "christians", "christianity", "catholic",
        "protestant", "evangelical", "hindu", "hindus", "hinduism", "sikh", "buddhist", "atheist",
        "bible", "torah", "talmud", r"satan\w*", "antichrist", "race", "racial", "racist",
        "racism", r"ethnic\w*", "white people", "black people", "whites", "blacks",
        r"immigrant\w*", r"migrant\w*", r"refugee\w*", "genocide", "ethnostate",
        "great replacement", r"trans\b", "transgender", "gay", r"lgbt\w*", r"homosexual\w*",
    ]
    .join("|");
    Regex::new(&format!("(?i)(?:^|[^a-z])(?:{terms})(?:$|[^a-z])")).expect("valid")
});

static SYSTEM_LINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)<\s*/?\s*system\s+message\s*>").expect("valid"));

/// The morning's political filter, over a title and an excerpt.
pub fn is_political(text: &str) -> bool {
    POLITICAL.is_match(text)
}

/// Whether scraped text may reach a room.
pub fn admissible(text: &str) -> bool {
    if text.trim().is_empty() {
        return false;
    }
    if screen_text(text, Direction::Narrative).is_some() {
        return false;
    }
    if SENSITIVE.is_match(text) {
        return false;
    }
    !is_political(&format!(" {text}"))
}

pub fn admissible_post(post: &ForumPost) -> bool {
    admissible(&format!(
        "{}\n{}",
        post.title.as_deref().unwrap_or_default(),
        post.text
    ))
}

/// `[r/singularity ↑12] title — text`, on one line.
pub fn render_post(post: &ForumPost) -> String {
    let score = match (post.score, post.site) {
        (None, _) => String::new(),
        (Some(score), Site::Reddit) => format!(" ↑{score}"),
        (Some(score), Site::Fourchan) => format!(" {score} replies"),
    };
    let title = match &post.title {
        Some(title) if *title != post.text => format!("{title} — "),
        _ => String::new(),
    };
    let text = post
        .text
        .split('\n')
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" / ");
    format!("[{}{score}] {title}{text}", post.place)
}

/// Untrusted text as quoted material: anything posing as the room's own
/// `<SYSTEM MESSAGE>` line is defused, so a headline cannot speak as the
/// room. The rest is left as written; the framing around it says what it is.
pub fn quote_untrusted(text: &str) -> String {
    SYSTEM_LINE
        .replace_all(text, "(quoted: system message)")
        .into_owned()
}

#[cfg(test)]
#[path = "screen_tests.rs"]
mod tests;
