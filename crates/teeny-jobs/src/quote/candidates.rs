//! Which posts may be quoted: two searches, the rules, the best few. A port
//! of `legacy/src/quote/candidates.ts`.

use std::sync::{Arc, LazyLock};

use chrono::{DateTime, TimeDelta, Utc};
use regex::Regex;
use teeny_safety::trusted::is_trusted_handle;
use teeny_x::cache::XReads;
use teeny_x::model::{SearchOptions, SortOrder, XPost};

use super::log::QuotedPost;

/// Where a candidate came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateSource {
    /// The ticker search.
    Ticker,
    /// The account's mentions.
    Mention,
}

/// A post that survived the rules.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub post: XPost,
    pub source: CandidateSource,
    /// The author is one of the lab's handles.
    pub trusted: bool,
    /// How much of a conversation it already is.
    pub score: u64,
}

/// The content filter's inbound lexicon: true when the text is clean.
pub type Screen = Arc<dyn Fn(&str) -> bool + Send + Sync>;

/// The operator's rules.
#[derive(Clone)]
pub struct CandidateRules {
    /// The account's own handle; its posts are never candidates.
    pub bot_username: String,
    /// The lab's handles, flagged rather than filtered.
    pub trusted_handles: Vec<String>,
    /// Any other contract address in a post is somebody else's shill.
    pub own_token_addresses: Vec<String>,
    /// The account's ticker, without the `$`.
    pub own_symbol: String,
    pub quoted: Vec<QuotedPost>,
    pub max_age: TimeDelta,
    pub author_cooldown: TimeDelta,
    pub screen: Option<Screen>,
    pub now: DateTime<Utc>,
}

/// A post that was not kept, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    pub id: String,
    pub author: Option<String>,
    pub reason: String,
}

/// What the searches found.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Found {
    pub candidates: Vec<Candidate>,
    pub skipped: Vec<Skipped>,
    pub searched: usize,
}

static ADDRESS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?:0x[a-fA-F0-9]{40}|[1-9A-HJ-NP-Za-km-z]{32,44})\b").expect("valid")
});
static PROMOTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(giveaway|airdrop|presale|pre-sale|whitelist|launchpad|dm me|link in bio|position yourself|alpha list|join (?:my|our) (?:telegram|discord)|claim (?:your|their) (?:share|allocation|reward))\b").expect("valid")
});
static CASHTAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\$([A-Za-z][A-Za-z0-9]{1,9})\b").expect("valid"));
static URL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"https?://\S+").expect("valid"));
static HANDLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"@\w+").expect("valid"));
static SOLANA: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\bsolana:\s*").expect("valid"));

fn parse(at: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(at).ok().map(|at| at.to_utc())
}

/// Why a post is not quoted, or `None` when it may be.
pub fn reason_to_skip(post: &XPost, rules: &CandidateRules) -> Option<String> {
    let handle = rules.bot_username.trim_start_matches('@').to_lowercase();
    let author = post
        .author_username
        .as_deref()
        .map(|author| author.trim_start_matches('@').to_lowercase());
    if author.as_deref() == Some(handle.as_str()) {
        return Some("our own post".into());
    }
    if rules.quoted.iter().any(|entry| entry.post_id == post.id.as_str()) {
        return Some("already quoted".into());
    }
    if let Some(author) = &author {
        let recent = rules.quoted.iter().find(|entry| {
            entry
                .author_username
                .as_deref()
                .is_some_and(|quoted| quoted.to_lowercase() == *author)
                && parse(&entry.quoted_at)
                    .is_some_and(|at| rules.now - at < rules.author_cooldown)
        });
        if let Some(recent) = recent {
            return Some(format!(
                "@{} was quoted on {}",
                post.author_username.as_deref().unwrap_or_default(),
                recent.quoted_at.get(..10).unwrap_or(&recent.quoted_at)
            ));
        }
    }
    if let Some(created) = post.created_at.as_deref().and_then(parse)
        && rules.now - created > rules.max_age
    {
        return Some("older than the window".into());
    }

    let text = &post.text;
    let own: Vec<String> = rules
        .own_token_addresses
        .iter()
        .map(|address| address.to_lowercase())
        .collect();
    if ADDRESS
        .find_iter(text)
        .any(|found| !own.contains(&found.as_str().to_lowercase()))
    {
        return Some("carries a contract address that is not ours".into());
    }
    if let Some(foreign) = CASHTAG
        .captures_iter(text)
        .map(|caps| caps[1].to_uppercase())
        .find(|symbol| *symbol != rules.own_symbol.to_uppercase())
    {
        return Some(format!("names another token (${foreign})"));
    }
    if PROMOTION.is_match(text) {
        return Some("reads as promotion".into());
    }
    if rules.screen.as_ref().is_some_and(|screen| !screen(text)) {
        return Some("the content filter".into());
    }
    let stripped = URL.replace_all(text, " ");
    let stripped = HANDLE.replace_all(&stripped, " ");
    let stripped = ADDRESS.replace_all(&stripped, " ");
    let stripped = SOLANA.replace_all(&stripped, " ");
    let words = stripped
        .split_whitespace()
        .filter(|word| word.chars().any(|c| c.is_ascii_alphanumeric()))
        .count();
    if words < 3 {
        return Some("says nothing beyond the ticker".into());
    }
    None
}

/// Likes, replies twice, reposts and quotes three times.
pub fn conversation_score(post: &XPost) -> u64 {
    post.metrics.as_ref().map_or(0, |m| {
        m.likes + 2 * m.replies + 3 * (m.reposts + m.quotes)
    })
}

/// How many of one author's posts may be in the list.
const PER_AUTHOR: usize = 2;

/// Two searches, one list: the operator's ticker query (replies kept,
/// reposts and own posts cut) and everybody who talked to the account, both
/// inside the window and by relevancy; the rules; the lab's own posts first,
/// then the busiest; two per author; `keep` of them.
pub async fn find_candidates(
    x: &dyn XReads,
    query: &str,
    max_results: u32,
    keep: usize,
    rules: &CandidateRules,
) -> Found {
    let handle = rules.bot_username.trim_start_matches('@');
    // X wants end_time at least ten seconds in the past; a minute is safe.
    let end = rules.now - TimeDelta::minutes(1);
    let window = SearchOptions {
        max_results: Some(max_results),
        start_time: Some(teeny_store::iso(end - rules.max_age)),
        end_time: Some(teeny_store::iso(end)),
        sort_order: Some(SortOrder::Relevancy),
        ..SearchOptions::default()
    };
    let searches = [
        (
            CandidateSource::Ticker,
            format!("({query}) -is:retweet -from:{handle}"),
        ),
        (
            CandidateSource::Mention,
            format!("(@{handle} OR to:{handle}) -from:{handle} -is:retweet"),
        ),
    ];
    let mut seen: Vec<String> = Vec::new();
    let mut found = Found::default();
    for (source, search) in searches {
        let posts = match x.search_posts(&search, &window).await {
            Ok(posts) => posts,
            Err(error) => {
                tracing::info!(?source, %error, "quote: a search failed");
                Vec::new()
            }
        };
        found.searched += posts.len();
        for post in posts {
            if seen.contains(&post.id.to_string()) {
                continue;
            }
            seen.push(post.id.to_string());
            if let Some(reason) = reason_to_skip(&post, rules) {
                found.skipped.push(Skipped {
                    id: post.id.to_string(),
                    author: post.author_username.clone(),
                    reason,
                });
                continue;
            }
            found.candidates.push(Candidate {
                trusted: is_trusted_handle(post.author_username.as_deref(), &rules.trusted_handles),
                score: conversation_score(&post),
                source,
                post,
            });
        }
    }
    found
        .candidates
        .sort_by(|a, b| b.trusted.cmp(&a.trusted).then(b.score.cmp(&a.score)));
    let mut counts: Vec<(String, usize)> = Vec::new();
    let mut kept = Vec::new();
    for candidate in std::mem::take(&mut found.candidates) {
        let author = candidate
            .post
            .author_username
            .as_deref()
            .map_or_else(|| candidate.post.id.to_string(), str::to_lowercase);
        let count = match counts.iter_mut().find(|(name, _)| *name == author) {
            Some((_, count)) => count,
            None => {
                counts.push((author, 0));
                &mut counts.last_mut().expect("just pushed").1
            }
        };
        if *count >= PER_AUTHOR {
            found.skipped.push(Skipped {
                id: candidate.post.id.to_string(),
                author: candidate.post.author_username.clone(),
                reason: "the author already has two in the list".into(),
            });
            continue;
        }
        *count += 1;
        kept.push(candidate);
    }
    kept.truncate(keep);
    found.candidates = kept;
    found
}

#[cfg(test)]
#[path = "candidates_tests.rs"]
pub(crate) mod tests;
