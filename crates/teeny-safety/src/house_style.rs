//! The account's hard formatting rules, applied to text about to be
//! published rather than only asked for in the prompt. SOUL.md states the
//! emoji cap as absolute and the humanizer skill bans em dashes; a model
//! follows both most of the time, which is not the same as always.
//! A port of `legacy/src/agent/house-style.ts`.
//!
//! Lengths are counted in UTF-16 code units, as the legacy service's
//! `String.length` did, so a draft the TypeScript service accepted is accepted
//! here. Unlike a JavaScript slice, a cut never splits a character.

use std::sync::LazyLock;

use regex::Regex;

/// X's limit on a post, as the legacy service counted it.
pub const MAX_POST_LENGTH: usize = 280;

fn regex(pattern: &str) -> Regex {
    Regex::new(pattern).expect("a valid pattern")
}

/// One emoji sequence: a pictographic base plus its variation selectors,
/// skin-tone modifiers, and any zero-width-joined parts, so a joined family
/// counts once.
static EMOJI_SEQUENCE: LazyLock<Regex> = LazyLock::new(|| {
    regex(
        r"\p{Extended_Pictographic}(?:\x{FE0F}|\p{Emoji_Modifier})*(?:\x{200D}\p{Extended_Pictographic}(?:\x{FE0F}|\p{Emoji_Modifier})*)*",
    )
});

/// "nfa", "NFA", "not financial advice", "dyor", "do your own research", with
/// the comma, dash or bracket that attached it to the sentence.
static DISCLAIMER: LazyLock<Regex> = LazyLock::new(|| {
    regex(
        r"(?i)(?:\s*[,;:(\[-]\s*|\s+)?(?:\(\s*)?\b(?:nfa|dyor|not\s+financial\s+advice|do\s+your\s+own\s+research)\b(?:\s*[)\]])?",
    )
});

static LEADING_HANDLES: LazyLock<Regex> =
    LazyLock::new(|| regex(r"^(?:\s*@[A-Za-z0-9_]{1,15}\b[\s,:]*)+"));
static DASH: LazyLock<Regex> = LazyLock::new(|| regex(r"\s*[—–]\s*"));
static DOUBLE_COMMA: LazyLock<Regex> = LazyLock::new(|| regex(r",\s*,"));
static RUNS_OF_BLANKS: LazyLock<Regex> = LazyLock::new(|| regex(r"[ \t]{2,}"));
static BLANKS_BEFORE_PUNCTUATION: LazyLock<Regex> = LazyLock::new(|| regex(r"[ \t]+([,.!?])"));
/// `([,.!?])[ \t]*\1` in the legacy pattern; `regex` has no backreferences.
static REPEATED_PUNCTUATION: LazyLock<Regex> =
    LazyLock::new(|| regex(r",[ \t]*,|\.[ \t]*\.|![ \t]*!|\?[ \t]*\?"));
static TRAILING_URL: LazyLock<Regex> = LazyLock::new(|| regex(r"\s*\S*://\S*$"));
static TRAILING_ASK: LazyLock<Regex> = LazyLock::new(|| regex(r"\s*\?\s*$"));

/// How many emoji sequences the text carries.
pub fn count_emoji(text: &str) -> usize {
    EMOJI_SEQUENCE.find_iter(text).count()
}

/// The length of a post as the legacy service measured it: UTF-16 code units.
pub fn post_length(text: &str) -> usize {
    text.encode_utf16().count()
}

/// How the house style is applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HouseStyleOptions {
    /// `MAX_EMOJI_PER_POST`, default 1.
    pub max_emoji: usize,
    /// Default [`MAX_POST_LENGTH`].
    pub max_length: usize,
    /// The handle being replied to, whose `@` X adds by itself.
    pub replying_to: Option<String>,
}

impl Default for HouseStyleOptions {
    fn default() -> Self {
        Self { max_emoji: 1, max_length: MAX_POST_LENGTH, replying_to: None }
    }
}

/// The draft as it may be published, and what was changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HouseStyleResult {
    pub text: String,
    /// What was changed, for logging. Empty when the draft was already clean.
    pub fixes: Vec<String>,
}

fn tidy_blanks(text: &str) -> String {
    let text = RUNS_OF_BLANKS.replace_all(text, " ");
    BLANKS_BEFORE_PUNCTUATION.replace_all(&text, "$1").into_owned()
}

/// Keeps the first `max_emoji` sequences and drops the rest, tidying the gaps.
fn cap_emoji(text: &str, max_emoji: usize) -> String {
    let mut seen = 0;
    let capped = EMOJI_SEQUENCE.replace_all(text, |caps: &regex::Captures<'_>| {
        seen += 1;
        if seen <= max_emoji { caps[0].to_owned() } else { String::new() }
    });
    tidy_blanks(&capped).trim().to_owned()
}

/// The longest prefix of at most `max_units` UTF-16 code units.
fn prefix_units(text: &str, max_units: usize) -> &str {
    let mut units = 0;
    for (index, c) in text.char_indices() {
        units += c.len_utf16();
        if units > max_units {
            return &text[..index];
        }
    }
    text
}

/// Cuts text to a length without cutting through a word or a URL.
///
/// A blunt slice is how "that's OpenHumans" became "that's OpenHum" in a
/// published reply, and half a URL is not a shorter receipt, it is a broken
/// one. So the cut falls back to the last whitespace, and if the tail it would
/// leave is a URL, that whole URL goes rather than being published in pieces.
pub fn truncate_at_boundary(text: &str, max_length: usize) -> String {
    if post_length(text) <= max_length {
        return text.to_owned();
    }
    let cut = prefix_units(text, max_length);
    let last_space = cut.char_indices().rev().find(|(_, c)| c.is_whitespace()).map(|(i, _)| i);
    // No whitespace at all: one very long token, and half of it is meaningless.
    let trimmed = match last_space {
        Some(index) if index > 0 => &cut[..index],
        _ => cut,
    };
    TRAILING_URL.replace(trimmed, "").trim_end().to_owned()
}

/// Applies the house style to a draft about to be published.
pub fn apply_house_style(draft: &str, options: &HouseStyleOptions) -> HouseStyleResult {
    let mut fixes = Vec::new();
    let mut text = draft.trim().to_owned();

    // X prefixes a reply with every participant's handle, so any handle a
    // draft opens with is published twice.
    if LEADING_HANDLES.is_match(&text) {
        let stripped = LEADING_HANDLES.replace(&text, "").trim().to_owned();
        // A post that is nothing but handles has no reply left once they go.
        if !stripped.is_empty() {
            text = stripped;
            fixes.push("dropped leading @handles X adds itself".to_owned());
        }
    }

    // The same handle further in is still a second tag. The name is kept and
    // only the @ goes, so the sentence still addresses them.
    if let Some(handle) = &options.replying_to {
        let handle = handle.trim_start_matches('@');
        let anywhere = regex(&format!(r"(?i)@({})\b", regex::escape(handle)));
        if anywhere.is_match(&text) {
            text = anywhere.replace_all(&text, "${1}").into_owned();
            fixes.push("unlinked a repeated @handle X already tags".to_owned());
        }
    }

    // An em or en dash reads as machine-written, and the humanizer bans it.
    if text.contains(['—', '–']) {
        let commas = DASH.replace_all(&text, ", ");
        text = DOUBLE_COMMA.replace_all(&commas, ",").into_owned();
        fixes.push("replaced em/en dashes".to_owned());
    }

    if text.contains(['“', '”', '‘', '’']) {
        text = text.replace(['“', '”'], "\"").replace(['‘', '’'], "'");
        fixes.push("straightened curly quotes".to_owned());
    }

    // The account does not disclaim: the tag undoes the conviction of the
    // line before it. It goes wherever it sits, with its punctuation.
    let undisclaimed = DISCLAIMER.replace_all(&text, "");
    if undisclaimed != text {
        let tidied = tidy_blanks(&undisclaimed);
        let tidied = REPEATED_PUNCTUATION.replace_all(&tidied, |caps: &regex::Captures<'_>| {
            caps[0][..1].to_owned()
        });
        let trimmed = tidied.trim();
        text = trimmed.strip_suffix([',', ';', ':']).unwrap_or(trimmed).to_owned();
        fixes.push("dropped a disclaimer".to_owned());
    }

    let emoji = count_emoji(&text);
    if emoji > options.max_emoji {
        text = cap_emoji(&text, options.max_emoji);
        fixes.push(format!("cut {} emoji over the cap of {}", emoji - options.max_emoji, options.max_emoji));
    }

    if post_length(&text) > options.max_length {
        text = truncate_at_boundary(&text, options.max_length);
        fixes.push(format!("trimmed to {} characters", options.max_length));
    }

    HouseStyleResult { text, fixes }
}

/// Whether the post ends by asking the reader something: a question mark on
/// its last line.
pub fn asks_the_reader(text: &str) -> bool {
    let last = text.trim().split('\n').rfind(|line| !line.trim().is_empty()).unwrap_or_default();
    last.trim_end().ends_with('?')
}

/// The last line cut when it is a question and there is a post left without
/// it; a one-line post that is all question keeps its words and loses the
/// mark, which reads as the deadpan it should have been.
pub fn without_the_ask(text: &str) -> String {
    let lines: Vec<&str> = text.trim().split('\n').collect();
    let kept = lines[..lines.len() - 1].join("\n");
    let kept = kept.trim();
    if !kept.is_empty() {
        return kept.to_owned();
    }
    TRAILING_ASK.replace(&lines.join("\n"), ".").into_owned()
}

#[cfg(test)]
#[path = "house_style_tests.rs"]
mod tests;
