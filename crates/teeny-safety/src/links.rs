//! Links the account is allowed to publish.
//!
//! A link in a reply is the one thing on a timeline that does something when
//! tapped, and a drainer is a page that looks like the project's own site
//! until a wallet connects to it. Nothing in the drafter's context is trusted,
//! so the rule is not "do not repeat suspicious links", which asks the model
//! to spot them. The rule is a list: a host is on it or the link does not go
//! out. Everything else here is about not being fooled into thinking a link
//! is on the list. A port of `legacy/src/safety/links.ts`.

use std::borrow::Cow;
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::bounded::Bounded;

/// One allowlist entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllowlistEntry {
    /// Lower-case host. Matches itself and any subdomain.
    pub host: String,
    /// When set, the URL's path must start with this, as whole segments.
    pub path_prefix: Option<String>,
    /// Only the bare host with no path is allowed: `pump.fun/` in config.
    pub root_only: bool,
}

/// What the account may link to without anyone adding to the list: its own
/// sites, code and community, and the platform it is on. The launchpad is
/// root-only, so the origin story can name it and a stranger's coin page on
/// it still cannot go out.
pub const DEFAULT_LINK_ALLOWLIST: &[&str] = &[
    "tinyhumans.ai",
    "tinyhumansworld.com",
    "tiny.place",
    "github.com/tinyhumansai",
    "x.com",
    "twitter.com",
    "t.me/TinyHumanAi",
    "pump.fun/",
];

/// Reads `host[/path]` entries, as `LINK_ALLOWLIST` and the operator write them.
pub fn parse_allowlist<I, S>(entries: I) -> Vec<AllowlistEntry>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    entries
        .into_iter()
        .filter_map(|raw| {
            let lower = raw.as_ref().trim().to_lowercase();
            let entry = lower
                .strip_prefix("https://")
                .or_else(|| lower.strip_prefix("http://"))
                .unwrap_or(&lower);
            if entry.is_empty() {
                return None;
            }
            let Some(slash) = entry.find('/') else {
                return Some(AllowlistEntry { host: entry.to_owned(), path_prefix: None, root_only: false });
            };
            let (host, path) = entry.split_at(slash);
            let host = host.to_owned();
            if path == "/" {
                return Some(AllowlistEntry { host, path_prefix: None, root_only: true });
            }
            let prefix = path.trim_end_matches('/').to_owned();
            Some(AllowlistEntry { host, path_prefix: Some(prefix), root_only: false })
        })
        .collect()
}

/// Common enough that a bare "name.tld" in a post is a link everybody taps.
const LINKABLE_TLDS: &[&str] = &[
    "com", "net", "org", "io", "ai", "co", "me", "app", "dev", "xyz", "fun", "place", "so",
    "gg", "to", "ly", "sh", "link", "site", "online", "info", "biz", "tv", "cc", "finance",
    "exchange", "cash", "money", "crypto", "sol", "eth", "wtf", "lol", "club", "live", "life",
    "world", "zone", "network", "tech", "digital", "cloud", "page", "run", "one", "top",
    "pro", "vip", "win", "bet", "best", "store", "shop", "space", "website", "today", "news",
    "art", "design", "us", "uk", "de", "fr", "ru", "cn", "in", "jp", "br", "ca", "au", "nl",
    "ch", "es", "it", "se", "no", "fi", "pl", "kr", "tw", "hk", "sg", "id", "ph", "vn", "th",
    "tr", "ua", "cz", "be", "at", "dk", "ie", "pt", "gr", "nz", "za", "mx", "ar", "cl", "ng",
    "ke", "eu", "is", "im", "ac", "st", "fm", "am", "ws", "tk", "ml", "ga", "cf", "gq", "pw",
    "icu", "click", "monster", "rest", "bond", "cyou", "buzz", "cam", "quest", "sbs", "lat",
    "mov", "zip", "work", "guru", "ninja", "email", "team", "trade", "markets", "capital",
    "fund", "ventures", "partners", "solutions", "services", "agency", "studio", "games",
    "game", "social", "chat", "wiki", "blog", "tools", "systems", "foundation", "dao",
    "nft", "coin", "token", "wallet", "swap", "dex", "defi",
];

fn regex(pattern: &str) -> Regex {
    Regex::new(pattern).expect("a valid pattern")
}

static DOTS: LazyLock<Regex> = LazyLock::new(|| {
    regex(r"(?i)\[\s*\.\s*\]|\(\s*\.\s*\)|\{\s*\.\s*\}|\s*\(dot\)\s*|\s*\[dot\]\s*|\s+dot\s+")
});
static HXXP: LazyLock<Regex> = LazyLock::new(|| regex(r"(?i)hxxps?://"));
static SPACED_SCHEME: LazyLock<Regex> =
    LazyLock::new(|| regex(r"(?i)h\s*t\s*t\s*p\s*(s?)\s*:\s*/\s*/"));

/// Ways people write a link so a filter will not see one, folded back into a
/// plain URL: "tinyhumans[.]ai", "tinyhumans(dot)ai", "hxxps://".
fn deobfuscate(text: &str) -> String {
    let text = DOTS.replace_all(text, ".");
    let text = HXXP.replace_all(&text, |caps: &regex::Captures<'_>| caps[0].replace(['x', 'X'], "t"));
    SPACED_SCHEME.replace_all(&text, "http${1}://").into_owned()
}

/// `(?<![\w@.-])` in the legacy pattern, `\w` being ASCII there.
fn glued(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '@' | '.' | '-')
}

static URL_PATTERN: LazyLock<Bounded> = LazyLock::new(|| {
    Bounded::new(
        r#"(?:(?:[hH][tT][tT][pP][sS]?://|[wW][wW][wW]\.)[^\s<>"'()\[\]{}]+|(?:[a-zA-Z0-9-]+\.)+[a-zA-Z]{2,24}(?::[0-9]{2,5})?(?:/[^\s<>"'()\[\]{}]*)?)"#,
        glued,
        false,
    )
});

fn has_scheme(raw: &str) -> bool {
    let lower = raw.get(..8).unwrap_or(raw).to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

/// One link found in a text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundLink {
    /// As it appeared (de-obfuscated), so it can be cut out again.
    pub raw: String,
    /// Absolute, with a scheme; `None` when it did not parse as a URL.
    pub url: Option<Url>,
}

/// Every link in the text, obfuscated ones included. A bare "name.tld" counts
/// only when the tld is one people actually get links on, so "node.js" and
/// "v1.2" are words; anything with a scheme, a www, or a path is a link.
pub fn find_links(text: &str) -> Vec<FoundLink> {
    let cleaned = deobfuscate(text);
    let mut found = Vec::new();
    for range in URL_PATTERN.find_all(&cleaned) {
        let raw = cleaned[range].trim_end_matches(['.', ',', ';', ':', '!', '?']);
        if raw.is_empty() {
            continue;
        }
        let explicit = has_scheme(raw) || raw.get(..4).is_some_and(|w| w.eq_ignore_ascii_case("www."));
        if !explicit {
            let host = raw.split(['/', ':', '?', '#']).next().unwrap_or_default();
            let tld = host.rsplit('.').next().unwrap_or_default().to_lowercase();
            // With a path, a made-up tld is still worth stripping: it costs a
            // word and it cannot be tapped anyway.
            if !raw.contains('/') && !LINKABLE_TLDS.contains(&tld.as_str()) {
                continue;
            }
        }
        let absolute: Cow<'_, str> =
            if has_scheme(raw) { raw.into() } else { format!("https://{raw}").into() };
        found.push(FoundLink { raw: raw.to_owned(), url: Url::parse(&absolute).ok() });
    }
    found
}

/// Whether one link may go out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkVerdict {
    Allowed,
    /// Why not, for the log and the hold.
    Refused(String),
}

impl LinkVerdict {
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allowed)
    }
}

static ASCII_HOST: LazyLock<Regex> = LazyLock::new(|| regex(r"^[a-z0-9.-]+$"));
static IPV4: LazyLock<Regex> = LazyLock::new(|| regex(r"^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$"));

/// Whether one link may go out. The shape of the URL is checked before the
/// host, because every trick for making a bad host look like a good one lives
/// in the shape: "tinyhumans.ai@evil.example", "tinyhumans.ai.evil.example",
/// a Cyrillic "а" in the name, a raw IP, a port.
pub fn judge_link(link: &FoundLink, allowlist: &[AllowlistEntry]) -> LinkVerdict {
    let refused = |reason: String| LinkVerdict::Refused(reason);
    let Some(url) = &link.url else { return refused("not a parseable URL".into()) };
    if url.scheme() != "https" && url.scheme() != "http" {
        return refused(format!("scheme {}: is not http(s)", url.scheme()));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return refused("credentials in the URL".into());
    }
    let host = url.host_str().unwrap_or_default().to_lowercase();
    if !ASCII_HOST.is_match(&host) {
        return refused("non-ascii host".into());
    }
    if host.split('.').any(|label| label.starts_with("xn--")) {
        return refused("punycode host".into());
    }
    if IPV4.is_match(&host) {
        return refused("raw IP address".into());
    }
    if url.port().is_some() {
        return refused("explicit port".into());
    }

    let path = url.path().trim_end_matches('/');
    let has_query = url.query().is_some_and(|q| !q.is_empty());
    let has_fragment = url.fragment().is_some_and(|f| !f.is_empty());
    for entry in allowlist {
        let host_matches = host == entry.host || host.ends_with(&format!(".{}", entry.host));
        if !host_matches {
            continue;
        }
        if entry.root_only {
            if path.is_empty() && !has_query && !has_fragment {
                return LinkVerdict::Allowed;
            }
            continue;
        }
        if let Some(prefix) = &entry.path_prefix {
            let prefix = prefix.to_lowercase();
            let lower = path.to_lowercase();
            if lower == prefix || lower.starts_with(&format!("{prefix}/")) {
                return LinkVerdict::Allowed;
            }
            continue;
        }
        return LinkVerdict::Allowed;
    }
    refused(format!("{host} is not on the link allowlist"))
}

/// A link that was cut, with why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemovedLink {
    pub link: String,
    pub reason: String,
}

/// A text with its disallowed links cut.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkFilterResult {
    pub text: String,
    /// Each link that was cut. Empty when the text was already clean.
    pub removed: Vec<RemovedLink>,
    /// Links that were checked and left in.
    pub kept: Vec<String>,
}

static RUNS_OF_BLANKS: LazyLock<Regex> = LazyLock::new(|| regex(r"[ \t]{2,}"));
static BLANKS_BEFORE_PUNCTUATION: LazyLock<Regex> = LazyLock::new(|| regex(r"[ \t]+([,.!?])"));
static EMPTY_BRACKETS: LazyLock<Regex> = LazyLock::new(|| regex(r"\(\s*\)"));

/// The text with every disallowed link cut out. Cutting rather than refusing,
/// because a reply is almost never about its link: "check the repo:
/// github.com/evil" is still a reply without the tail. The caller decides
/// whether what is left is worth sending.
pub fn filter_links(text: &str, allowlist: &[AllowlistEntry]) -> LinkFilterResult {
    let mut removed = Vec::new();
    let mut kept = Vec::new();
    let mut cleaned = deobfuscate(text);
    for link in find_links(text) {
        match judge_link(&link, allowlist) {
            LinkVerdict::Allowed => kept.push(link.raw),
            LinkVerdict::Refused(reason) => {
                cleaned = cleaned.replace(&link.raw, "");
                removed.push(RemovedLink { link: link.raw, reason });
            }
        }
    }
    if removed.is_empty() {
        return LinkFilterResult { text: text.to_owned(), removed, kept };
    }
    let tidied = RUNS_OF_BLANKS.replace_all(&cleaned, " ");
    let tidied = BLANKS_BEFORE_PUNCTUATION.replace_all(&tidied, "$1");
    let tidied = EMPTY_BRACKETS.replace_all(&tidied, "");
    LinkFilterResult { text: tidied.trim().to_owned(), removed, kept }
}

#[cfg(test)]
#[path = "links_tests.rs"]
mod tests;
