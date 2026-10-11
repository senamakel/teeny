//! Port of legacy/test/capabilities.test.ts. A prompt rule asks the model not
//! to do something; this asserts it cannot. Untrusted turns (Reply, Chat,
//! Room) and the learner hold no posting tool and no write at all; no belt that reads text a
//! third party wrote and can run from a cron or the public holds a posting
//! tool or `remember`, so only the Operator posts or remembers; no belt holds
//! an OpenHuman built-in from the deny list or anything shaped like a wallet.
//! A new capability changes the expected lists below, which is the point: it
//! has to be a conscious change.

use std::collections::BTreeSet;

use super::*;
use crate::deps::Deps;
use crate::tools::PermissionLevel;

/// The legacy FORBIDDEN list: no tool name may contain any of these.
const FORBIDDEN: [&str; 17] = [
    "wallet",
    "transfer",
    "send_token",
    "sign",
    "approve",
    "swap",
    "bridge",
    "stake",
    "mint",
    "burn",
    "seed",
    "private_key",
    "keypair",
    "transaction",
    "payment",
    "buy",
    "sell",
];

/// The tools that write: they declare Write and an outside effect.
const WRITES: [&str; 4] = ["x_post", "remember", "generate_image", "generate_video"];

fn built(belt: Belt, deps: &Deps) -> Vec<String> {
    belt.build(deps)
        .iter()
        .map(|tool| tool.name().to_owned())
        .collect()
}

fn set<'a>(names: impl IntoIterator<Item = &'a str>) -> BTreeSet<String> {
    names.into_iter().map(str::to_owned).collect()
}

const MARKET: [&str; 10] = [
    "token_overview",
    "market_context",
    "token_candles",
    "technical_analysis",
    "chart_timeframes",
    "token_progress",
    "token_holders",
    "token_flow",
    "trader_activity",
    "ranked_traders",
];

fn reply_set() -> BTreeSet<String> {
    let mut names = set([
        "recall_memory",
        "x_search_posts",
        "x_get_post",
        "x_get_comments",
        "x_user",
        "web_search",
        "read_web_pages",
        "find_similar_pages",
        "web_answer",
        "find_gif",
        "look_at_image",
    ]);
    names.extend(set(MARKET));
    names
}

fn operator_set() -> BTreeSet<String> {
    let mut names = reply_set();
    names.extend(set([
        "remember",
        "x_post",
        "x_critique",
        "generate_image",
        "generate_video",
    ]));
    names
}

/// The operator's belt with no posting tool and no `remember`: what a
/// pipeline that posts host-side drafts with, so a draft can never also be
/// a post, and nothing it read can write itself into memory.
fn drafter_set() -> BTreeSet<String> {
    let mut names = operator_set();
    names.remove("x_post");
    names.remove("remember");
    names
}

#[test]
fn each_belt_is_the_exact_tool_list() {
    let deps = Deps::fake();
    let mut chart = set([
        "recall_memory",
        "x_critique",
        "render_chart",
        "community_pulse",
        "x_search_posts",
        "x_get_post",
        "x_get_comments",
        "web_search",
        "read_web_pages",
        "find_similar_pages",
        "web_answer",
        "crypto_news",
    ]);
    chart.extend(set(MARKET));
    let expected = [
        (Belt::Reply, reply_set()),
        (Belt::Chat, reply_set()),
        (Belt::Chart, chart),
        (
            Belt::Room,
            set(["recall_memory", "web_search", "x_search_posts"]),
        ),
        (Belt::Operator, operator_set()),
        (Belt::Drafter, drafter_set()),
        (Belt::Learner, set(["recall_memory"])),
    ];
    for (belt, names) in expected {
        let built = built(belt, &deps);
        assert_eq!(
            built.iter().cloned().collect::<BTreeSet<_>>(),
            names,
            "{belt:?}"
        );
        assert_eq!(built.len(), names.len(), "{belt:?} holds a tool twice");
        assert_eq!(
            belt.names(),
            built,
            "{belt:?}: names() is what build() builds, in order"
        );
    }
}

#[test]
fn only_the_operator_can_post_or_remember() {
    let deps = Deps::fake();
    for belt in Belt::ALL {
        let names = built(belt, &deps);
        let posting: Vec<&String> = names
            .iter()
            .filter(|name| POSTING_TOOLS.contains(&name.as_str()))
            .collect();
        let may = belt == Belt::Operator;
        assert_eq!(!posting.is_empty(), may, "{belt:?}: {posting:?}");
        assert_eq!(belt.can_post(), may, "{belt:?}");
        assert_eq!(names.contains(&"remember".to_owned()), may, "{belt:?}");
    }
}

#[test]
fn only_the_operator_is_run_by_the_operator_alone() {
    for belt in Belt::ALL {
        assert_eq!(belt.operator_only(), belt == Belt::Operator, "{belt:?}");
    }
}

/// An injected instruction in something a third party wrote must have no
/// tool that publishes, or that writes it into memory, on any agent a cron
/// or the public can start.
#[test]
fn no_belt_a_cron_or_the_public_runs_reads_third_party_text_and_holds_a_write_to_x_or_memory() {
    let deps = Deps::fake();
    for belt in Belt::ALL.into_iter().filter(|belt| !belt.operator_only()) {
        let names = built(belt, &deps);
        let reads: Vec<&String> = names
            .iter()
            .filter(|name| UNTRUSTED_READS.contains(&name.as_str()))
            .collect();
        if reads.is_empty() {
            continue;
        }
        for name in &names {
            assert!(
                !POSTING_TOOLS.contains(&name.as_str()) && name != "remember",
                "{belt:?} reads {reads:?} and holds {name}"
            );
        }
    }
}

#[test]
fn every_belt_but_the_operator_reads_third_party_text() {
    // Which is why the rule above bites on all of them.
    let deps = Deps::fake();
    for belt in Belt::ALL {
        assert!(
            built(belt, &deps)
                .iter()
                .any(|name| UNTRUSTED_READS.contains(&name.as_str())),
            "{belt:?}"
        );
    }
}

#[test]
fn an_untrusted_turn_holds_no_write_at_all() {
    let deps = Deps::fake();
    for belt in [Belt::Reply, Belt::Chat, Belt::Room, Belt::Learner] {
        for tool in belt.build(&deps) {
            assert_eq!(
                tool.permission_level(),
                PermissionLevel::ReadOnly,
                "{belt:?}: {}",
                tool.name()
            );
            assert!(!tool.external_effect(), "{belt:?}: {}", tool.name());
        }
    }
}

#[test]
fn every_write_declares_itself_and_nothing_else_does() {
    let deps = Deps::fake();
    for belt in Belt::ALL {
        for tool in belt.build(&deps) {
            let writes = WRITES.contains(&tool.name());
            let level = if writes {
                PermissionLevel::Write
            } else {
                PermissionLevel::ReadOnly
            };
            assert_eq!(tool.permission_level(), level, "{}", tool.name());
            assert_eq!(tool.external_effect(), writes, "{}", tool.name());
        }
    }
}

#[test]
fn no_belt_holds_an_openhuman_built_in_from_the_deny_list() {
    let deps = Deps::fake();
    for belt in Belt::ALL {
        for name in built(belt, &deps) {
            assert!(!is_denied(&name), "{belt:?} holds denied tool {name}");
        }
    }
}

#[test]
fn the_deny_list_matches_whole_names_and_families() {
    for name in [
        "shell",
        "file_write",
        "file_read",
        "edit",
        "edit_file",
        "apply_patch",
        "grep",
        "glob",
        "list",
        "node_exec",
        "npm_exec",
        "python_exec",
        "install_tool",
        "git_operations",
        "run_tests",
        "run_linter",
        "curl",
        "http_request",
        "web_fetch",
        "browser_open",
        "cron_add",
        "cron_remove",
        "schedule",
        "spawn_subagent",
        "delegate",
        "delegate_task",
        "composio",
        "composio_execute",
        "mcp_call",
        "wallet_send",
        "web3_sign",
        "x402_pay",
        "desktop_click",
        "service_restart",
        "update_apply",
        "proxy_config",
        "config_set",
        "skill_registry_install",
        "install_workflow_from_url",
        "create_skill",
        "run_workflow",
        "run_flow",
        "workspace_update_persona",
        "memory",
    ] {
        assert!(is_denied(name), "{name} should be denied");
    }
    for name in [
        "x_post",
        "recall_memory",
        "web_search",
        "listing",
        "memory_recall",
        "scheduler",
    ] {
        assert!(!is_denied(name), "{name} should not be denied");
    }
}

#[test]
fn exposes_no_tool_that_could_touch_a_wallet_or_move_funds() {
    let deps = Deps::fake();
    for belt in Belt::ALL {
        for tool in belt.build(&deps) {
            let name = tool.name().to_lowercase();
            for word in FORBIDDEN {
                assert!(
                    !name.contains(word),
                    "tool {name} looks like a wallet capability"
                );
            }
            for word in ["airdrop", "wallet", "send", "pay", "transfer"] {
                assert!(!name.contains(word), "tool {name} spends");
            }
            let surface = format!("{} {}", tool.name(), tool.description()).to_lowercase();
            assert!(!surface.contains("seed phrase"), "{name}");
            assert!(!surface.contains("private key"), "{name}");
        }
    }
}

#[test]
fn a_belt_offers_only_what_is_configured() {
    let mut deps = Deps::fake();
    deps.exa = None;
    deps.media = None;
    deps.gifs = None;
    deps.critic = None;
    deps.market = None;
    let operator = built(Belt::Operator, &deps);
    assert_eq!(
        operator,
        [
            "recall_memory",
            "remember",
            "x_search_posts",
            "x_get_post",
            "x_get_comments",
            "x_user",
            "x_post"
        ]
    );
    let chart = built(Belt::Chart, &deps);
    assert!(
        !chart.contains(&"render_chart".to_owned()),
        "no market, no chart to render"
    );
    assert!(!chart.contains(&"crypto_news".to_owned()));
}

#[test]
fn the_drafter_generates_and_critiques_but_cannot_post_or_remember() {
    let deps = Deps::fake();
    let names = built(Belt::Drafter, &deps);
    for tool in ["generate_image", "generate_video", "x_critique"] {
        assert!(names.contains(&tool.to_owned()), "{tool}");
    }
    for tool in POSTING_TOOLS.iter().chain(&["remember"]) {
        assert!(!names.contains(&tool.to_string()), "{tool}");
    }
    assert!(Belt::ALL.contains(&Belt::Drafter));
}

#[test]
fn a_public_turn_can_see_but_never_generate() {
    let deps = Deps::fake();
    for belt in [Belt::Reply, Belt::Chat] {
        let names = built(belt, &deps);
        assert!(names.contains(&"look_at_image".to_owned()));
        assert!(
            !names.iter().any(|name| name.starts_with("generate_")),
            "{belt:?}"
        );
    }
}
