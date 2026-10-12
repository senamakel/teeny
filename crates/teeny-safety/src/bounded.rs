//! A regex with the one-character lookarounds the legacy patterns lean on.
//!
//! The `regex` crate has no lookaround, and the TypeScript filter needs
//! exactly two shapes of it: "not preceded by one of these characters"
//! (`(?<![a-z0-9])`, `(?<![\w@.-])`) and "not followed by a letter or digit"
//! (`(?![a-z0-9])`). Both are emulated here exactly:
//!
//! - the lookahead is compiled into the pattern as "followed by a non-
//!   alphanumeric or the end", inside which the real match is group 1, so the
//!   engine's leftmost-first choice is the one a backtracker would make;
//! - the lookbehind is checked on the candidate's start, and a candidate that
//!   fails it is retried one character on, as a backtracker's next start is.

use std::ops::Range;

use regex::Regex;

pub(crate) struct Bounded {
    regex: Regex,
    /// The character before a match must not satisfy this.
    not_after: fn(char) -> bool,
}

impl Bounded {
    /// `body` must stand after a character `not_after` refuses (or the start);
    /// with `alnum_end`, it must also not be followed by an ASCII letter or digit.
    pub(crate) fn new(body: &str, not_after: fn(char) -> bool, alnum_end: bool) -> Self {
        let tail = if alnum_end { "(?:[^a-zA-Z0-9]|$)" } else { "" };
        let regex = Regex::new(&format!("({body}){tail}")).expect("a valid pattern");
        Self { regex, not_after }
    }

    /// The first match starting at or after `pos`.
    pub(crate) fn find_at(&self, text: &str, mut pos: usize) -> Option<Range<usize>> {
        loop {
            let found = self.regex.captures_at(text, pos)?.get(1)?.range();
            let refused = text[..found.start]
                .chars()
                .next_back()
                .is_some_and(self.not_after);
            if !refused {
                return Some(found);
            }
            pos = found.start + text[found.start..].chars().next().map_or(1, char::len_utf8);
            if pos > text.len() {
                return None;
            }
        }
    }

    pub(crate) fn is_match(&self, text: &str) -> bool {
        self.find_at(text, 0).is_some()
    }

    /// Every non-overlapping match, left to right, as a global JS regex walks.
    pub(crate) fn find_all(&self, text: &str) -> Vec<Range<usize>> {
        let mut found = Vec::new();
        let mut pos = 0;
        while pos <= text.len() {
            let Some(range) = self.find_at(text, pos) else {
                break;
            };
            pos = if range.is_empty() {
                range.end + 1
            } else {
                range.end
            };
            found.push(range);
        }
        found
    }

    /// Every match replaced with `with`.
    pub(crate) fn replace_all(&self, text: &str, with: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut last = 0;
        for range in self.find_all(text) {
            out.push_str(&text[last..range.start]);
            out.push_str(with);
            last = range.end;
        }
        out.push_str(&text[last..]);
        out
    }
}
