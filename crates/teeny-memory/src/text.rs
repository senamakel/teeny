//! Cutting text for a prompt without cutting a character in half.
//!
//! The legacy counted UTF-16 units, as JavaScript's `slice` does, and an
//! emoji is two of them: cut between the two and the string carried a lone
//! surrogate, which JSON can only escape and the model router rejected
//! outright (the first learnings brief failed exactly that way). A Rust
//! string cannot hold half a character, but the limits here are still
//! counted in UTF-16 units so a prompt is the length the legacy's was, and a
//! character that would straddle the limit is left out whole.

/// How long `text` is as JavaScript counts it, in UTF-16 units.
pub fn units(text: &str) -> usize {
    text.encode_utf16().count()
}

/// The first `max` UTF-16 units of `text`, never ending inside a character.
pub fn cut_at(text: &str, max: usize) -> &str {
    let mut used = 0;
    for (index, character) in text.char_indices() {
        used += character.len_utf16();
        if used > max {
            return &text[..index];
        }
    }
    text
}

/// `text` on one line, whitespace runs collapsed, and at most `max` units
/// long: cut one short and ended with `…` when it does not fit.
pub fn clip(text: &str, max: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if units(&flat) > max {
        format!("{}…", cut_at(&flat, max.saturating_sub(1)))
    } else {
        flat
    }
}

#[cfg(test)]
#[path = "text_tests.rs"]
mod tests;
