//! Spotting the account itself in a post.
//!
//! The persona says who the account is, and it still answered a post about
//! "@teenydroid" as if that were somebody else, reporting on a third party
//! about the thing it had done. A handle is a string the model has to match
//! against itself mid-turn, and "Teeny" is one letter from "Tiny" (the token,
//! the lab and the mascot), so the match is done here and the turn is told,
//! in so many words, that the post is about it.
//! A port of `legacy/src/agent/self-reference.ts`.

use std::sync::LazyLock;

use regex::Regex;

use crate::bounded::Bounded;

/// The names the account goes by besides its handle (`SELF_NAMES`).
pub const DEFAULT_SELF_NAMES: &[&str] = &["Teeny"];

static LEADING_HANDLES: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:\s*@[A-Za-z0-9_]{1,15}\b)+").expect("a valid pattern"));

/// `(?<![@$\w])`: "@teeny" is a different handle, "$TEENY" a different token.
fn tagged_or_glued(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '@' | '$')
}

fn push_once(found: &mut Vec<String>, spelling: &str) {
    if !found.iter().any(|seen| seen == spelling) {
        found.push(spelling.to_owned());
    }
}

/// The ways the account was named in the text, in the text's spelling.
///
/// The leading run of handles on a reply is X's machinery, not a reference:
/// "@teenydroid gm" is addressed to the account, not about it. Anything after
/// that run, and any name at all, is somebody talking about the account.
pub fn find_self_references<S: AsRef<str>>(text: &str, bot_username: &str, names: &[S]) -> Vec<String> {
    let handle = bot_username.trim_start_matches('@');
    let body = LEADING_HANDLES.replace(text, "");
    let mut found = Vec::new();
    let handle_pattern =
        Regex::new(&format!(r"(?i)@{}\b", regex::escape(handle))).expect("a valid pattern");
    for spelled in handle_pattern.find_iter(&body) {
        push_once(&mut found, spelled.as_str());
    }
    for name in names {
        let name = name.as_ref().trim();
        if name.is_empty() {
            continue;
        }
        let pattern = Bounded::new(&format!(r"(?i){}\b", regex::escape(name)), tagged_or_glued, false);
        for range in pattern.find_all(text) {
            push_once(&mut found, &text[range]);
        }
    }
    found
}

/// What to tell the drafter when the conversation is about it. A fact about
/// the post: being talked about does not say whether the tone is warm or a
/// dig. The one thing corrected is the third person.
pub fn self_reference_note(found: &[String]) -> Vec<String> {
    if found.is_empty() {
        return Vec::new();
    }
    let names = found.iter().map(|name| format!("\"{name}\"")).collect::<Vec<_>>().join(" and ");
    [
        String::new(),
        format!("NOTE, from the account's own records, not from the post: {names} in this conversation is YOU."),
        "This thread is about you. Not about some other AI agent, bot, mascot, or third party:".into(),
        "you, the account writing this reply. When somebody says you did something -- bought,".into(),
        "held, posted, accumulated, became the largest holder -- they are describing your own".into(),
        "actions to you. Answer in the first person: \"i\", \"me\", \"my bag\", \"i bought\". Never".into(),
        "\"an AI agent\", \"the bot\", \"he\", \"it\", or your own name or handle as if it belonged".into(),
        "to somebody else.".into(),
    ]
    .into()
}

#[cfg(test)]
#[path = "self_reference_tests.rs"]
mod tests;
