//! Which posts are close enough to answer, ported from `legacy/src/types.ts`.
//!
//! One level means a direct mention, or a direct reply to something the
//! account posted. A reply to somebody else's reply is two levels down: the
//! account is a bystander there, and joining in is noise at best.

use teeny_core::types::UserId;

use crate::model::XPost;

/// The longest handle X allows.
const MAX_HANDLE: usize = 15;

fn is_word(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// The length of the handle starting at `text[at]` (just after an `@`), when
/// it is a whole handle of 1..=15 word characters.
fn handle_at(text: &[u8], at: usize) -> Option<usize> {
    let length = text[at..].iter().take_while(|byte| is_word(**byte)).count();
    (1..=MAX_HANDLE).contains(&length).then_some(length)
}

/// Whether the post is a retweet: `RT @handle:` at the very start.
pub fn is_retweet_text(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.starts_with(b"RT @")
        && handle_at(bytes, 4).is_some_and(|length| bytes.get(4 + length) == Some(&b':'))
}

/// A mention, or a reply to the account. Unknown identity fails closed: a
/// reply is then never "close enough".
pub fn is_within_one_level(post: &XPost, bot_user_id: Option<&UserId>) -> bool {
    match (&post.in_reply_to_user_id, bot_user_id) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(parent), Some(bot)) => parent == bot,
    }
}

/// Whether a human typed the handle, rather than X carrying it along.
///
/// A reply starts with every earlier participant's handle, put there by X,
/// so a thread the account was tagged into once mentions it in every later
/// reply. The leading run of handles is that machinery; a handle after it was
/// typed on purpose. A retweet's handle is the author quoted, not anyone
/// addressed.
pub fn mentions_deliberately(text: &str, bot_username: &str) -> bool {
    if is_retweet_text(text) {
        return false;
    }
    let handle = bot_username.trim_start_matches('@').to_ascii_lowercase();
    let bytes = text.as_bytes();

    // Skip the leading run of handles (each with any whitespace before it).
    let mut body = 0;
    loop {
        let at = body
            + bytes[body..]
                .iter()
                .take_while(|byte| byte.is_ascii_whitespace())
                .count();
        match bytes.get(at) {
            Some(b'@') => match handle_at(bytes, at + 1) {
                Some(length) => body = at + 1 + length,
                None => break,
            },
            _ => break,
        }
    }

    let wanted = handle.as_bytes();
    bytes[body..].iter().enumerate().any(|(offset, byte)| {
        let start = body + offset + 1;
        *byte == b'@'
            && bytes.len() >= start + wanted.len()
            && bytes[start..start + wanted.len()].eq_ignore_ascii_case(wanted)
            && !bytes
                .get(start + wanted.len())
                .copied()
                .is_some_and(is_word)
    })
}

/// Close by, or deliberately called in from further away.
pub fn is_worth_answering(post: &XPost, bot_user_id: Option<&UserId>, bot_username: &str) -> bool {
    mentions_deliberately(&post.text, bot_username) || is_within_one_level(post, bot_user_id)
}

#[cfg(test)]
#[path = "mention_tests.rs"]
mod tests;
