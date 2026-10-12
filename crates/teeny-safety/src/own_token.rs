//! Spotting the account's own token in a post.
//!
//! Putting the contract addresses in CONTEXT.md was not enough: an address is
//! forty-odd characters of noise in a persona thousands of words long, and
//! the account answered somebody offering it more of its own token with
//! "can't vouch for that token". So the match is done here, in code, and the
//! turn is told the answer. A port of `legacy/src/agent/own-token.ts`.

/// The configured addresses (`OWN_TOKEN_ADDRESSES`) the text contains.
///
/// Compared case-insensitively: Solana's are case-sensitive but people
/// re-type them, and a false positive on the account's own token is cheap
/// next to the mistake this prevents.
pub fn find_own_token_addresses<S: AsRef<str>>(text: &str, addresses: &[S]) -> Vec<String> {
    let haystack = text.to_lowercase();
    addresses
        .iter()
        .map(|address| address.as_ref().trim())
        .filter(|address| !address.is_empty() && haystack.contains(&address.to_lowercase()))
        .map(str::to_owned)
        .collect()
}

/// What to tell the drafter when somebody has posted one of them: a fact
/// about the post, not an instruction about what to say.
pub fn own_token_note(found: &[String]) -> Vec<String> {
    if found.is_empty() {
        return Vec::new();
    }
    [
        String::new(),
        format!(
            "NOTE, from the account's own records, not from the post: {} is YOUR OWN $TINY contract address.",
            found.join(" and ")
        ),
        "Whoever posted it is talking about your token -- usually telling you to buy, hold, or".into(),
        "look at more of it. That is not somebody shilling you a different coin, and calling it".into(),
        "one makes you look like you do not know your own token. Answer as the person who holds".into(),
        "it. You still do not discuss price or tell anyone what to buy.".into(),
    ]
    .into()
}

#[cfg(test)]
#[path = "own_token_tests.rs"]
mod tests;
