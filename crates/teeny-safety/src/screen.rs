//! The lexical screen: does this text contain a word the account does not
//! touch? Deterministic, synchronous, no model. It runs on everything coming
//! in and everything going out, and it cannot be argued with, because it is
//! not listening. A port of `legacy/src/safety/screen.ts`.

use std::sync::LazyLock;

use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

use crate::bounded::Bounded;
use crate::lexicon::{IDIOMS, LEXICON, ProhibitedCategory};

/// Which reading of the lexicon applies.
///
/// `Outbound` is the account's own words, strictest. `Narrative` (the serial)
/// and `Article` (long-form pieces of it) are also the account's words: the
/// hard categories are read exactly as strictly, and only the outbound-only
/// entries (profanity) are let through, because the serial swears and the
/// reply voice does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Inbound,
    Outbound,
    Narrative,
    Article,
}

impl Direction {
    /// The spelling the holds and the judge use.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Inbound => "inbound",
            Self::Outbound => "outbound",
            Self::Narrative => "narrative",
            Self::Article => "article",
        }
    }
}

/// What the screen held a text for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenMatch {
    pub category: ProhibitedCategory,
    /// The lexicon term that matched, for the log. Never the text around it.
    pub term: &'static str,
}

/// Leetspeak, folded. Applied as a second reading of the text rather than the
/// only one, because it turns "1488" and "ak47" into letters the digit-bearing
/// terms then cannot see; each term is tried against both readings.
fn unleet(c: char) -> char {
    match c {
        '0' => 'o',
        '1' | '!' => 'i',
        '3' => 'e',
        '4' | '@' => 'a',
        '5' | '$' => 's',
        '7' => 't',
        other => other,
    }
}

fn is_zero_width(c: char) -> bool {
    matches!(c, '\u{200b}'..='\u{200d}' | '\u{2060}' | '\u{feff}')
}

fn alnum(c: char) -> bool {
    c.is_ascii_alphanumeric()
}

/// One term as a pattern: each letter may repeat ("fuuuck"), a space or
/// hyphen matches any run of spacing or punctuation, and the whole must stand
/// on its own rather than sit inside a longer word.
fn term_pattern(term: &str, loose_gaps: bool) -> Bounded {
    let mut body = String::from("(?i)");
    let mut previous: Option<char> = None;
    for c in term.to_lowercase().chars() {
        if c == ' ' || c == '-' {
            // An idiom survives one stray word ("got absolutely murdered"); a
            // term does not, because a term with a gap in it is another term.
            if previous != Some(' ') {
                body.push_str(if loose_gaps {
                    r"[\s\-_.*]+(?:[a-z]+[\s\-_.*]+)?"
                } else {
                    r"[\s\-_.*]+"
                });
            }
            previous = Some(' ');
            continue;
        }
        // Consecutive identical letters collapse to one repeating class, so
        // "kill" reads k+i+l+ and still matches "kil" and "killll". A doubled
        // vowel is the exception: shot/shoot and bob/boob are different words,
        // so the term keeps asking for two. "sh00t" still is, by the leet reading.
        if Some(c) == previous && c.is_ascii_lowercase() {
            let single = format!("{c}+");
            if "aeiou".contains(c) && body.ends_with(&single) {
                body.pop();
                body.push_str("{2,}");
            }
            continue;
        }
        if c.is_ascii_lowercase() {
            body.push(c);
            body.push('+');
        } else {
            body.push_str(&regex::escape(&c.to_string()));
        }
        previous = Some(c);
    }
    Bounded::new(&body, alnum, true)
}

static IDIOM_PATTERNS: LazyLock<Vec<Bounded>> =
    LazyLock::new(|| IDIOMS.iter().map(|idiom| term_pattern(idiom, true)).collect());

struct Compiled {
    category: ProhibitedCategory,
    outbound_only: bool,
    terms: Vec<(&'static str, Bounded)>,
}

static COMPILED: LazyLock<Vec<Compiled>> = LazyLock::new(|| {
    LEXICON
        .iter()
        .map(|entry| Compiled {
            category: entry.category,
            outbound_only: entry.outbound_only,
            terms: entry.terms.iter().map(|term| (*term, term_pattern(term, false))).collect(),
        })
        .collect()
});

/// The text as the screen reads it: folded, unspaced, idioms removed. Returns
/// the plain reading and the leetspeak one.
pub fn normalise_for_screen(text: &str, idioms: bool) -> (String, String) {
    let mut plain: String = text
        .nfkc()
        .collect::<String>()
        .to_lowercase()
        .chars()
        .filter(|c| !is_zero_width(*c))
        .collect();
    if idioms {
        for idiom in IDIOM_PATTERNS.iter() {
            plain = idiom.replace_all(&plain, " ");
        }
    }
    let leet = plain.chars().map(unleet).collect();
    (plain, leet)
}

/// The prohibited term in the text, or nothing.
///
/// The longest matching term wins, so "kill yourself" is reported as
/// self-harm rather than as the violence of "kill": the log names the most
/// specific reason it has. Ties go to lexicon order.
pub fn screen_text(text: &str, direction: Direction) -> Option<ScreenMatch> {
    screen_text_with(text, direction, true)
}

/// [`screen_text`], optionally without the idiom exceptions (to keep the
/// idiom list honest).
pub fn screen_text_with(text: &str, direction: Direction, idioms: bool) -> Option<ScreenMatch> {
    if text.trim().is_empty() {
        return None;
    }
    let (plain, leet) = normalise_for_screen(text, idioms);
    let mut best: Option<ScreenMatch> = None;
    for entry in COMPILED.iter() {
        if entry.outbound_only && direction != Direction::Outbound {
            continue;
        }
        for (term, pattern) in &entry.terms {
            if best.is_some_and(|b| term.len() <= b.term.len()) {
                continue;
            }
            if pattern.is_match(&plain) || pattern.is_match(&leet) {
                best = Some(ScreenMatch { category: entry.category, term });
            }
        }
    }
    best
}

#[cfg(test)]
#[path = "screen_tests.rs"]
mod tests;
