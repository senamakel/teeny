//! Scraped text, made plain: forum posts arrive as HTML fragments with
//! entities, and the room reads plain text.

use std::sync::LazyLock;

use regex::Regex;

static BREAK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)<br\s*/?>").expect("valid"));
static PARA_END: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)</p>").expect("valid"));
static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]+>").expect("valid"));
static SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[ \t]+").expect("valid"));
static GAPS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\n{3,}").expect("valid"));
static ENTITY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)&(#x[0-9a-f]+|#\d+|[a-z]+);").expect("valid"));
static WORD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[a-z0-9$][a-z0-9'-]*").expect("valid"));

const STOP: [&str; 52] = [
    "to", "be", "or", "of", "in", "on", "at", "is", "it", "as", "an", "by", "up", "so", "if", "no",
    "we", "he", "me", "my", "do", "us", "the", "and", "for", "with", "that", "this", "from",
    "into", "are", "was", "were", "has", "have", "its", "about", "what", "when", "where", "who",
    "how", "why", "out", "not", "but", "you", "your", "our", "they", "them", "there",
];

pub fn strip_html(html: &str) -> String {
    let text = BREAK.replace_all(html, "\n");
    let text = PARA_END.replace_all(&text, "\n");
    let text = TAG.replace_all(&text, "");
    let text = unescape_entities(&text);
    let text = SPACES.replace_all(&text, " ");
    GAPS.replace_all(&text, "\n\n").trim().to_owned()
}

pub fn unescape_entities(text: &str) -> String {
    ENTITY
        .replace_all(text, |captures: &regex::Captures<'_>| {
            let whole = &captures[0];
            let code = &captures[1];
            let decoded = if let Some(hex) = code
                .strip_prefix("#x")
                .or_else(|| code.strip_prefix("#X"))
            {
                u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
            } else if let Some(decimal) = code.strip_prefix('#') {
                decimal.parse::<u32>().ok().and_then(char::from_u32)
            } else {
                match code.to_ascii_lowercase().as_str() {
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "quot" => Some('"'),
                    "apos" => Some('\''),
                    "nbsp" => Some(' '),
                    _ => None,
                }
            };
            decoded.map_or_else(|| whole.to_owned(), String::from)
        })
        .into_owned()
}

/// At most `max_chars`, the last an ellipsis when cut.
pub fn clip(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    format!(
        "{}…",
        head(text, max_chars.saturating_sub(1)).trim_end()
    )
}

/// The first `max_chars` characters.
pub fn head(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

/// Runs of whitespace as one space, trimmed.
pub fn flatten(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The words worth matching a forum post on: lowercase, no stop words, no shorties.
pub fn keywords(query: &str, max: usize) -> Vec<String> {
    let lower = query.to_lowercase();
    let mut words: Vec<String> = Vec::new();
    for found in WORD.find_iter(&lower) {
        let word = found.as_str().trim_matches('\'');
        if word.chars().count() < 2 || STOP.contains(&word) || words.iter().any(|w| w == word) {
            continue;
        }
        words.push(word.to_owned());
        if words.len() >= max {
            break;
        }
    }
    words
}

#[cfg(test)]
#[path = "text_tests.rs"]
mod tests;
