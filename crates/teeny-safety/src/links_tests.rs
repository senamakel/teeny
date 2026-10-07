//! Port of `legacy/test/safety-links.test.ts`, case for case.

use super::*;

fn allowlist() -> Vec<AllowlistEntry> {
    parse_allowlist(DEFAULT_LINK_ALLOWLIST)
}

fn verdict(url: &str) -> LinkVerdict {
    let links = find_links(url);
    let link = links.first().unwrap_or_else(|| panic!("no link found in {url}"));
    judge_link(link, &allowlist())
}

fn raws(text: &str) -> Vec<String> {
    find_links(text).into_iter().map(|link| link.raw).collect()
}

mod find_links {
    use super::*;

    #[test]
    fn finds_explicit_www_and_bare_links() {
        assert_eq!(
            raws("see https://tinyhumans.ai/about and www.example.com and tiny.place"),
            ["https://tinyhumans.ai/about", "www.example.com", "tiny.place"]
        );
    }

    #[test]
    fn does_not_mistake_words_with_dots_for_links() {
        assert!(raws("node.js and next.js, e.g. v1.2 vs 0.5x, U.S. based").is_empty());
    }

    #[test]
    fn leaves_email_addresses_and_handles_alone() {
        assert!(raws("mail hello@tinyhumans.ai or ping @tiny.place").is_empty());
    }

    #[test]
    fn sees_through_obfuscation() {
        assert_eq!(raws("claim at tinyhumans[.]ai/claim")[0], "tinyhumans.ai/claim");
        assert_eq!(raws("claim at evil (dot) xyz")[0], "evil.xyz");
        let found = find_links("hxxps://evil.example/x");
        assert_eq!(found[0].url.as_ref().and_then(|url| url.host_str()), Some("evil.example"));
    }

    #[test]
    fn drops_trailing_punctuation() {
        assert_eq!(raws("read tinyhumans.ai.")[0], "tinyhumans.ai");
        assert_eq!(
            raws("(github.com/tinyhumansai/openhuman)")[0],
            "github.com/tinyhumansai/openhuman"
        );
    }
}

mod judge_link {
    use super::*;

    #[test]
    fn allows_the_accounts_own_sites_and_subdomains() {
        assert_eq!(verdict("https://tinyhumans.ai"), LinkVerdict::Allowed);
        assert_eq!(verdict("https://docs.tinyhumans.ai/x"), LinkVerdict::Allowed);
        assert_eq!(verdict("https://github.com/tinyhumansai/openhuman"), LinkVerdict::Allowed);
        assert_eq!(verdict("https://x.com/TinyMascot/status/1"), LinkVerdict::Allowed);
        assert_eq!(verdict("https://t.me/TinyHumanAi"), LinkVerdict::Allowed);
    }

    #[test]
    fn refuses_anything_not_on_the_list() {
        assert!(!verdict("https://claim-tiny.xyz/airdrop").is_allowed());
        assert!(!verdict("https://github.com/someone-else/repo").is_allowed());
        assert!(!verdict("https://t.me/other_channel").is_allowed());
    }

    #[test]
    fn is_not_fooled_by_lookalike_hosts() {
        assert!(!verdict("https://tinyhumans.ai.evil.example/").is_allowed());
        assert!(!verdict("https://tinyhumans.ai@evil.example/").is_allowed());
        assert!(!verdict("https://evil.example/tinyhumans.ai").is_allowed());
        assert!(!verdict("https://tinyhum\u{430}ns.ai/").is_allowed()); // Cyrillic а
        assert!(!verdict("https://xn--tinyhumns-x3a.ai/").is_allowed());
        assert!(!verdict("https://1.2.3.4/").is_allowed());
        assert!(!verdict("https://tinyhumans.ai:8443/").is_allowed());
    }

    #[test]
    fn honours_a_root_only_entry() {
        assert_eq!(verdict("pump.fun"), LinkVerdict::Allowed);
        assert_eq!(verdict("https://pump.fun/"), LinkVerdict::Allowed);
        assert!(!verdict("https://pump.fun/coin/abc").is_allowed());
        assert!(!verdict("https://pump.fun/?ref=x").is_allowed());
    }

    #[test]
    fn honours_a_path_prefix_as_a_whole_segment() {
        assert!(verdict("https://github.com/tinyhumansai").is_allowed());
        assert!(!verdict("https://github.com/tinyhumansai-fake/x").is_allowed());
    }

    #[test]
    fn accepts_extra_entries_in_the_same_spelling() {
        let extended = parse_allowlist(
            DEFAULT_LINK_ALLOWLIST.iter().copied().chain(["https://dexscreener.com/solana"]),
        );
        let link = &find_links("https://dexscreener.com/solana/abc")[0];
        assert_eq!(judge_link(link, &extended), LinkVerdict::Allowed);
        let other = &find_links("https://dexscreener.com/ethereum/abc")[0];
        assert!(!judge_link(other, &extended).is_allowed());
    }

    #[test]
    fn names_the_host_when_it_refuses() {
        assert_eq!(
            verdict("https://claim-tiny.xyz/go"),
            LinkVerdict::Refused("claim-tiny.xyz is not on the link allowlist".into())
        );
    }
}

mod filter_links {
    use super::*;

    #[test]
    fn leaves_a_clean_draft_untouched() {
        let result = filter_links("the code is at github.com/tinyhumansai, come look", &allowlist());
        assert_eq!(result.text, "the code is at github.com/tinyhumansai, come look");
        assert!(result.removed.is_empty());
        assert_eq!(result.kept, ["github.com/tinyhumansai"]);
    }

    #[test]
    fn cuts_a_disallowed_link_and_tidies_the_sentence() {
        let result =
            filter_links("claim your tokens at https://claim-tiny.xyz/go , quick!", &allowlist());
        assert_eq!(result.text, "claim your tokens at, quick!");
        assert_eq!(result.removed.len(), 1);
        assert_eq!(result.removed[0].link, "https://claim-tiny.xyz/go");
    }

    #[test]
    fn cuts_an_obfuscated_link_too() {
        let result = filter_links("go to evil[.]example/claim now", &allowlist());
        assert_eq!(result.text, "go to now");
        assert_eq!(result.removed[0].link, "evil.example/claim");
    }

    #[test]
    fn keeps_the_allowed_and_cuts_the_rest_in_one_draft() {
        let result = filter_links("real: tinyhumans.ai fake: tinyhumans.ai.evil.xyz", &allowlist());
        assert_eq!(result.text, "real: tinyhumans.ai fake:");
        assert_eq!(result.kept, ["tinyhumans.ai"]);
    }
}
