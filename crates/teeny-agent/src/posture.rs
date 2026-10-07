//! Which tools each agent holds: the belts of the plan's agent table, and
//! the OpenHuman built-ins no Teeny agent may hold.
//!
//! OpenHuman advertises every host tool it is handed, so a belt is decided
//! here, by construction: [`Belt::build`] makes only that belt's tools.
//!
//! | Belt | Turns | Holds |
//! |---|---|---|
//! | Reply | X mentions, public | memory recall, X reads, the web, the market, GIFs, eyes |
//! | Chat | Telegram, public | the same as Reply |
//! | Publisher | rota pipelines, trusted | Reply's, plus `x_post`, `remember`, `x_critique` and the generators |
//! | Chart | the chart post, trusted | the legacy chart agent's: market, chart, `post_chart`, community, news |
//! | Room | backrooms personas, trusted, no posting | `recall_memory`, `web_search`, `x_search_posts` |
//! | Operator | `/agent/run`, bearer | the same as Publisher |
//!
//! Untrusted turns (Reply, Chat) hold no write at all: replies and chat
//! produce text, and host code posts it after the safety gate. A tool whose
//! provider is not configured is left off rather than offered broken.

use std::sync::Arc;

use teeny_store::holds::HoldSurface;
use teeny_x::XPublisher;

use crate::deps::Deps;
use crate::tools::chart::{ChartBook, ChartSource, PostChartTool, post_chart, render_chart};
use crate::tools::market::{XPostSearch, community_pulse, crypto_news, market_tools};
use crate::tools::media::{find_gif, media_tools, x_critique};
use crate::tools::memory::{recall_memory, remember};
use crate::tools::post::{BudgetAllowance, Pacing, PostingGuard, XPostTool, x_post};
use crate::tools::web::{web_search, web_tools};
use crate::tools::{Tool, x};

/// The tools that publish.
pub const POSTING_TOOLS: [&str; 2] = ["x_post", "post_chart"];

/// OpenHuman built-ins no Teeny agent may hold. An entry ending in `*` is a
/// family: every name starting with what comes before it.
pub const DENY: &[&str] = &[
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
    "cron_*",
    "schedule",
    "spawn_*",
    "delegate*",
    "composio*",
    "mcp_*",
    "wallet_*",
    "web3_*",
    "x402_*",
    "desktop_*",
    "service_*",
    "update_apply",
    "proxy_config",
    "config_*",
    "skill_registry_*",
    "install_workflow_from_url",
    "create_skill",
    "run_workflow",
    "run_flow",
    "workspace_update_persona",
    "memory",
];

/// Whether `name` is on the [`DENY`] list.
pub fn is_denied(name: &str) -> bool {
    DENY.iter().any(|entry| match entry.strip_suffix('*') {
        Some(prefix) => name.starts_with(prefix),
        None => name == *entry,
    })
}

/// One agent's tool belt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Belt {
    /// `teeny-reply`: X mentions. Public.
    Reply,
    /// `teeny-chat`: Telegram. Public.
    Chat,
    /// `teeny-publisher`: the rota pipelines. Trusted.
    Publisher,
    /// `teeny-chart`: the daily chart post. Trusted.
    Chart,
    /// `teeny-room-*`: the backrooms personas. Trusted, never posts.
    Room,
    /// `teeny-operator`: `/agent/run`, behind the bearer. Trusted.
    Operator,
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

const WEB: [&str; 4] = [
    "web_search",
    "read_web_pages",
    "find_similar_pages",
    "web_answer",
];

impl Belt {
    pub const ALL: [Belt; 6] = [
        Self::Reply,
        Self::Chat,
        Self::Publisher,
        Self::Chart,
        Self::Room,
        Self::Operator,
    ];

    /// Whether this belt holds a posting tool.
    pub fn can_post(self) -> bool {
        matches!(self, Self::Publisher | Self::Chart | Self::Operator)
    }

    /// The tools this belt holds when everything is configured, in the
    /// order [`build`](Self::build) makes them.
    pub fn names(self) -> Vec<String> {
        let mut names: Vec<&str> = Vec::new();
        match self {
            Self::Reply | Self::Chat => {
                names.extend([
                    "recall_memory",
                    "x_search_posts",
                    "x_get_post",
                    "x_get_comments",
                    "x_user",
                ]);
                names.extend(WEB);
                names.extend(["find_gif", "look_at_image"]);
                names.extend(MARKET);
            }
            Self::Publisher | Self::Operator => {
                names.extend([
                    "recall_memory",
                    "remember",
                    "x_search_posts",
                    "x_get_post",
                    "x_get_comments",
                    "x_user",
                    "x_post",
                ]);
                names.extend(WEB);
                names.extend([
                    "x_critique",
                    "find_gif",
                    "look_at_image",
                    "generate_image",
                    "generate_video",
                ]);
                names.extend(MARKET);
            }
            Self::Chart => {
                names.extend(["recall_memory", "remember", "x_critique"]);
                names.extend(&MARKET[..6]);
                names.push("render_chart");
                names.extend(&MARKET[6..]);
                names.push("post_chart");
                names.extend([
                    "community_pulse",
                    "x_search_posts",
                    "x_get_post",
                    "x_get_comments",
                ]);
                names.extend(WEB);
                names.push("crypto_news");
            }
            Self::Room => names.extend(["recall_memory", "web_search", "x_search_posts"]),
        }
        names.into_iter().map(str::to_owned).collect()
    }

    /// Constructs this belt's tools, and only those.
    pub fn build(self, deps: &Deps) -> Vec<Arc<dyn Tool>> {
        let mut tools: Vec<Arc<dyn Tool>> = Vec::new();
        match self {
            Self::Reply | Self::Chat => {
                tools.push(recall_memory(Arc::clone(&deps.memory)));
                tools.extend(x_reads(deps, true));
                tools.extend(web(deps));
                tools.extend(gifs(deps));
                // Eyes, never the generators: a public turn cannot spend.
                if let Some(media) = &deps.media {
                    tools.extend(media_tools(
                        &media.gate(false, false),
                        Arc::clone(&deps.media_budget),
                        Arc::clone(&deps.safety),
                    ));
                }
                tools.extend(market(deps));
            }
            Self::Publisher | Self::Operator => {
                tools.push(recall_memory(Arc::clone(&deps.memory)));
                tools.push(remember(Arc::clone(&deps.memory)));
                tools.extend(x_reads(deps, true));
                tools.push(posting_tool(deps));
                tools.extend(web(deps));
                tools.extend(critique(deps));
                tools.extend(gifs(deps));
                if let Some(media) = &deps.media {
                    tools.extend(media_tools(
                        media,
                        Arc::clone(&deps.media_budget),
                        Arc::clone(&deps.safety),
                    ));
                }
                tools.extend(market(deps));
            }
            Self::Chart => {
                tools.push(recall_memory(Arc::clone(&deps.memory)));
                tools.push(remember(Arc::clone(&deps.memory)));
                tools.extend(critique(deps));
                tools.extend(chart_tools(deps));
                tools.push(community_pulse(
                    Arc::new(XPostSearch(Arc::clone(&deps.x_reads))),
                    &deps.settings.community_query,
                    &deps.settings.bot_username,
                    deps.settings.search_limit,
                ));
                tools.extend(x_reads(deps, false));
                tools.extend(web(deps));
                if let Some(exa) = &deps.exa {
                    tools.push(crypto_news(
                        Arc::clone(exa) as _,
                        Some("crypto market news today bitcoin solana sentiment".into()),
                    ));
                }
            }
            Self::Room => {
                tools.push(recall_memory(Arc::clone(&deps.memory)));
                if let Some(exa) = &deps.exa {
                    tools.push(web_search(Arc::clone(exa)));
                }
                tools.push(x::search_posts(
                    Arc::clone(&deps.x_reads),
                    deps.settings.search_limit,
                ));
            }
        }
        tools
    }
}

/// The X reads; `x_user` is new and the chart agent never had it.
fn x_reads(deps: &Deps, with_user: bool) -> Vec<Arc<dyn Tool>> {
    let reads = &deps.x_reads;
    let limit = deps.settings.search_limit;
    let mut tools = vec![
        x::search_posts(Arc::clone(reads), limit),
        x::get_post(Arc::clone(reads)),
        x::get_comments(Arc::clone(reads), limit),
    ];
    if with_user {
        tools.push(x::user(Arc::clone(reads)));
    }
    tools
}

fn web(deps: &Deps) -> Vec<Arc<dyn Tool>> {
    deps.exa
        .as_ref()
        .map(|exa| web_tools(Arc::clone(exa)))
        .unwrap_or_default()
}

fn gifs(deps: &Deps) -> Option<Arc<dyn Tool>> {
    deps.gifs
        .as_ref()
        .map(|gifs| find_gif(Arc::clone(gifs), &deps.settings.gif_channel))
}

fn critique(deps: &Deps) -> Option<Arc<dyn Tool>> {
    deps.critic
        .as_ref()
        .map(|critic| x_critique(Arc::clone(critic)))
}

fn market(deps: &Deps) -> Vec<Arc<dyn Tool>> {
    deps.market
        .as_ref()
        .map(|feed| market_tools(feed.reports(&deps.settings.token_symbol)))
        .unwrap_or_default()
}

/// The cadence for own posts of `kind`, and the reply budget for replies.
fn pacing(deps: &Deps, kind: &str) -> Arc<Pacing> {
    let pacing = Pacing::new(Arc::clone(&deps.cadence), kind);
    Arc::new(match &deps.reply_budget {
        Some(budget) => pacing.with_replies(Arc::new(BudgetAllowance::new(
            Arc::clone(&deps.x_reads),
            Arc::clone(budget),
            Arc::clone(&deps.x),
        ))),
        None => pacing,
    })
}

/// A publisher whose guard names `surface` and `tool` on every hold.
fn publisher(
    deps: &Deps,
    surface: HoldSurface,
    tool: &'static str,
    pacing: &Arc<Pacing>,
) -> Arc<XPublisher> {
    let guard = PostingGuard::new(
        Arc::clone(&deps.safety),
        surface,
        tool,
        deps.settings.max_emoji,
    )
    .with_pacing(Arc::clone(pacing));
    Arc::new(XPublisher::new(
        Arc::clone(&deps.x),
        Arc::new(guard),
        deps.settings.dry_run,
    ))
}

fn posting_tool(deps: &Deps) -> Arc<dyn Tool> {
    let pacing = pacing(deps, "agent");
    x_post(XPostTool {
        publisher: publisher(deps, HoldSurface::X, "x_post", &pacing),
        x: Arc::clone(&deps.x),
        media: deps.media.as_ref().map(|media| Arc::clone(&media.store)),
        gifs: deps.gifs.clone(),
        pacing: Some(pacing),
        max_emoji: deps.settings.max_emoji,
    })
}

/// The ten market tools with `render_chart` after the first six and
/// `post_chart` last, as the legacy chart agent listed them. Nothing without
/// a token to chart.
fn chart_tools(deps: &Deps) -> Vec<Arc<dyn Tool>> {
    let Some(feed) = &deps.market else {
        return Vec::new();
    };
    let book = Arc::new(ChartBook::default());
    let mut tools = market_tools(feed.reports(&deps.settings.token_symbol));
    let render = render_chart(
        ChartSource {
            market: Arc::clone(&feed.data),
            address: feed.address.clone(),
            symbol: Some(deps.settings.token_symbol.clone()),
            watermark: deps.settings.chart_watermark.clone(),
            output_dir: deps.settings.chart_output_dir.clone(),
        },
        Arc::clone(&book),
    );
    tools.insert(6, render);
    let pacing = pacing(deps, "chart");
    tools.push(post_chart(PostChartTool {
        publisher: publisher(deps, HoldSurface::Chart, "post_chart", &pacing),
        x: Arc::clone(&deps.x),
        book,
        memory: Some(Arc::clone(&deps.memory)),
        symbol: deps.settings.token_symbol.clone(),
        pacing: Some(pacing),
    }));
    tools
}

#[cfg(test)]
#[path = "posture_tests.rs"]
mod tests;
