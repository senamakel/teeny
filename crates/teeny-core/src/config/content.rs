//! The content side of the configuration: the rota and every job that
//! publishes, plus Telegram and media. Each publishing job is off by default.

use chrono::NaiveDate;

use super::kinds::{ChartRenderer, ChartTimeframe, LoreSeed, VideoGenApi};
use crate::Secret;

/// The account's own artwork, copied from tinyhumansworld.com into the repo
/// (`assets/tiny/README.md`): what `TINY_REFERENCE_IMAGES` means when blank.
pub const DEFAULT_REFERENCE_IMAGES: [&str; 10] = [
    "assets/tiny/tiny-ai-narrativ.jpg",
    "assets/tiny/tiny-by-openhuman.jpg",
    "assets/tiny/tiny-and-friends.jpg",
    "assets/tiny/tiny-in-the-sun.jpg",
    "assets/tiny/asian-wagmi-tiny.jpg",
    "assets/tiny/github-tiny.jpg",
    "assets/tiny/tiny-community-work.jpg",
    "assets/tiny/tiny-the-phoenix-2.jpg",
    "assets/tiny/tiny-group-sand-3.jpg",
    "assets/tiny/tiny-love.jpg",
];

/// The calendar: four slots a day and a week-long rota of who owns each.
#[derive(Debug, Clone, PartialEq)]
pub struct RotaConfig {
    /// `POST_ROTA_DAYS`, 1..=14, default 7.
    pub days: u32,
    /// `POST_ROTA_ANCHOR`, `YYYY-MM-DD`, default 2026-09-14 (a Monday, slot 0).
    pub anchor: NaiveDate,
    /// `MOON_ROTA_SLOT`, default 0.
    pub moon_slot: u32,
    /// `CHART_ROTA_SLOT`, default 3.
    pub chart_slot: u32,
    /// `EVENING_CRON`, default `0 19 * * *`.
    pub evening_cron: String,
    /// `NARRATIVE_ROTA_SLOTS`, comma separated, default `1,4`.
    pub narrative_slots: Vec<u32>,
    /// `ARTICLE_ROTA_SLOTS`, comma separated, default `2,5`.
    pub article_slots: Vec<u32>,
}

/// A wallet worth naming when it shows up in the flow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KolWallet {
    /// The address.
    pub wallet: String,
    /// For the agent's reasoning only; `watchlist` when none was given.
    pub label: String,
}

/// The chart agent.
#[derive(Debug, Clone, PartialEq)]
pub struct ChartConfig {
    /// `CHART_ENABLED`, default off.
    pub enabled: bool,
    /// `CHART_CRON`, default `0 13 * * *`.
    pub cron: String,
    /// `CHART_TOKEN_ADDRESS`, optional, at least 26 characters.
    pub token_address: Option<String>,
    /// `CHART_TOKEN_SYMBOL`, default `TINY`.
    pub token_symbol: String,
    /// `CHART_CHAIN`, default `solana`.
    pub chain: String,
    /// `CHART_TIMEFRAME`, default `1h`.
    pub timeframe: ChartTimeframe,
    /// `CHART_BARS`, 20..=500, default 120.
    pub bars: u32,
    /// `CHART_RENDERER`, default `native`.
    pub renderer: ChartRenderer,
    /// `CHART_URL_TEMPLATE`, optional.
    pub url_template: Option<String>,
    /// `CHART_BROWSER_CHANNEL`, optional.
    pub browser_channel: Option<String>,
    /// `CHART_BROWSER_PATH`, optional.
    pub browser_path: Option<String>,
    /// `CHART_OUTPUT_DIR`, default `data/charts`.
    pub output_dir: String,
    /// `CHART_COMMUNITY_QUERY`, default `$TINY -is:retweet`; blank disables
    /// the pulse tool.
    pub community_query: String,
    /// `CHART_WATERMARK`, optional; defaults to the handle at startup.
    pub watermark: Option<String>,
    /// `GMGN_BASE_URL`, optional URL.
    pub gmgn_base_url: Option<String>,
    /// `SOLANA_RPC_URL`, optional URL.
    pub solana_rpc_url: Option<String>,
    /// `KOL_WALLETS`, `address:label` pairs, comma separated.
    pub kol_wallets: Vec<KolWallet>,
}

/// The morning post.
#[derive(Debug, Clone, PartialEq)]
pub struct MorningConfig {
    /// `MORNING_ENABLED`, default off.
    pub enabled: bool,
    /// `MORNING_CRON`, default `0 7 * * *`.
    pub cron: String,
    /// `MORNING_NEWS_TOPICS`, comma separated.
    pub news_topics: Vec<String>,
    /// `MORNING_NEWS_DAYS`, 1..=7, default 2.
    pub news_days: u32,
    /// `MORNING_STORIES_PER_TOPIC`, 1..=20, default 8.
    pub stories_per_topic: u32,
    /// `MORNING_X_POSTS_PER_QUERY`, 0..=100, default 20; 0 turns the
    /// timeline off.
    pub x_posts_per_query: u32,
    /// `MORNING_X_HOURS`, 1..=168, default 24.
    pub x_hours: u32,
    /// `MORNING_X_QUERIES`, `label=query;label=query`, default blank (the
    /// built-in corners).
    pub x_queries: String,
    /// `MORNING_X_TRENDS_WOEID`, default 23424977 (the United States).
    pub x_trends_woeid: u32,
    /// `MORNING_BACKROOMS`, default off.
    pub backrooms: bool,
    /// `MORNING_BACKROOMS_TURNS`, 2..=60, default 10.
    pub backrooms_turns: u32,
    /// `MORNING_VIDEO_SECONDS`, 4..=20, default 10.
    pub video_seconds: u32,
}

/// The moon chart.
#[derive(Debug, Clone, PartialEq)]
pub struct MoonConfig {
    /// `MOON_ENABLED`, default off.
    pub enabled: bool,
    /// `MOON_CRON`, default `0 13 * * *`.
    pub cron: String,
    /// `MOON_COIN_ID`, default `tiny-humans-ai`.
    pub coin_id: String,
    /// `MOON_CATEGORY`, default `ai-meme-coins`.
    pub category: String,
    /// `MOON_BOARD_NAME`, default `AI meme coins`.
    pub board_name: String,
    /// `MOON_BOARD_SIZE`, 5..=1000, default 500.
    pub board_size: u32,
    /// `MOON_RUNGS`, 1..=12, default 7.
    pub rungs: u32,
    /// `MOON_EXCLUDE_IDS`, comma separated, default `tutorial`.
    pub exclude_ids: Vec<String>,
    /// `MOON_MIN_MULTIPLE`, at least 1, default 2.
    pub min_multiple: f64,
    /// `MOON_MAX_MULTIPLE`, at least 1, default 2000.
    pub max_multiple: f64,
    /// `MOON_OUTPUT_DIR`, default `data/moon`.
    pub output_dir: String,
}

/// The lore post.
#[derive(Debug, Clone, PartialEq)]
pub struct LoreConfig {
    /// `LORE_ENABLED`, default off.
    pub enabled: bool,
    /// `LORE_CRON`, default `0 13 * * *`.
    pub cron: String,
    /// `LORE_SEEDS`, comma separated, at least one, default `news,forum`.
    pub seeds: Vec<LoreSeed>,
    /// `LORE_TOPICS`, comma separated.
    pub topics: Vec<String>,
    /// `LORE_BACKROOMS_TURNS`, 2..=60, default 10.
    pub backrooms_turns: u32,
}

/// The quote post.
#[derive(Debug, Clone, PartialEq)]
pub struct QuoteConfig {
    /// `QUOTE_ENABLED`, default off.
    pub enabled: bool,
    /// `QUOTE_CRON`, default `0 1 * * *`.
    pub cron: String,
    /// `QUOTE_QUERY`, X search syntax.
    pub query: String,
    /// `QUOTE_HOURS`, 1..=168, default 48.
    pub hours: u32,
    /// `QUOTE_POSTS_PER_SEARCH`, 10..=100, default 50.
    pub posts_per_search: u32,
    /// `QUOTE_AUTHOR_COOLDOWN_DAYS`, 0..=90, default 7.
    pub author_cooldown_days: u32,
    /// `QUOTE_LOG_PATH`, default `data/quotes.json`.
    pub log_path: String,
}

/// The serial.
#[derive(Debug, Clone, PartialEq)]
pub struct NarrativeConfig {
    /// `NARRATIVE_ENABLED`, default off.
    pub enabled: bool,
    /// `NARRATIVE_VOICE_PATH`, default `narrative/VOICE.md`.
    pub voice_path: String,
    /// `NARRATIVE_LORE_PATH`, default `narrative/LORE.md`.
    pub lore_path: String,
    /// `NARRATIVE_STATE_PATH`, default `data/narrative-state.json`.
    pub state_path: String,
    /// `NARRATIVE_LIKENESS_DIR`, default `data/likeness`.
    pub likeness_dir: String,
    /// `NARRATIVE_BRANCHES`, 1..=6, default 3.
    pub branches: u32,
    /// `NARRATIVE_TURNS`, 2..=8, default 6.
    pub turns: u32,
    /// `NARRATIVE_VIDEO_SECONDS`, 4..=20, default 10.
    pub video_seconds: u32,
    /// `NARRATIVE_WILD_TEMPERATURE`, 0..=2, default 1.
    pub wild_temperature: f64,
    /// `NARRATIVE_STRAIGHT_TEMPERATURE`, 0..=2, default 0.6.
    pub straight_temperature: f64,
    /// `NARRATIVE_TELEGRAM_CHAT_IDS`, comma separated.
    pub telegram_chat_ids: Vec<String>,
}

/// The backrooms: rooms of agents that talk and never post.
#[derive(Debug, Clone, PartialEq)]
pub struct BackroomsConfig {
    /// `BACKROOMS_DIR`, default `data/backrooms`.
    pub dir: String,
    /// `BACKROOMS_EDITOR_PATH`, default `narrative/EDITOR.md`.
    pub editor_path: String,
    /// `BACKROOMS_BUDGET_TOKENS`, 1000..=30000, default 9000.
    pub budget_tokens: u32,
    /// `BACKROOMS_TURNS`, 2..=60, default 12.
    pub turns: u32,
    /// `BACKROOMS_AGENTS`, comma separated, default `teeny,swarm`.
    pub agents: Vec<String>,
    /// `BACKROOMS_TOOL_CALLS`, 0..=8, default 3.
    pub tool_calls: u32,
    /// `REDDIT_SUBS`, comma separated.
    pub reddit_subs: Vec<String>,
    /// `FOURCHAN_BOARDS`, comma separated, default `g,x,sci,biz`.
    pub fourchan_boards: Vec<String>,
}

/// The article.
#[derive(Debug, Clone, PartialEq)]
pub struct ArticleConfig {
    /// `ARTICLE_ENABLED`, default off.
    pub enabled: bool,
    /// `ARTICLE_PUBLISH`, default off: a rehearsal even when enabled.
    pub publish: bool,
    /// `ARTICLES_DIR`, default `data/articles`.
    pub dir: String,
    /// `ARTICLE_ROOMS`, 1..=4, default 2.
    pub rooms: u32,
    /// `ARTICLE_SECTIONS_MAX`, 3..=5, default 4.
    pub sections_max: u32,
    /// `ARTICLE_WORDS`, 600..=3000, default 1500.
    pub words: u32,
    /// `ARTICLE_NEWS_TOPICS`, comma separated.
    pub news_topics: Vec<String>,
    /// `ARTICLE_HUMANIZER_PATH`, default `skills/humanizer/SKILL.md`.
    pub humanizer_path: String,
    /// `ARTICLE_WRITER_TEMPERATURE`, 0..=2, default 0.8.
    pub writer_temperature: f64,
    /// `ARTICLE_MIN_INTERVAL_HOURS`, 1..=168, default 48.
    pub min_interval_hours: f64,
}

/// The content calendar (`calendar.json`).
#[derive(Debug, Clone, PartialEq)]
pub struct CalendarConfig {
    /// `CALENDAR_ENABLED`, default off.
    pub enabled: bool,
    /// `CALENDAR_PATH`, default `calendar.json`.
    pub path: String,
    /// `CALENDAR_TELEGRAM_CHAT_IDS`, comma separated.
    pub telegram_chat_ids: Vec<String>,
}

/// The same account on Telegram.
#[derive(Debug, Clone, PartialEq)]
pub struct TelegramConfig {
    /// `TELEGRAM_BOT_TOKEN`, optional, in the shape BotFather hands out.
    /// Unset, nothing Telegram-shaped starts.
    pub bot_token: Option<Secret>,
    /// `TELEGRAM_DRY_RUN`, default off: separate from `DRY_RUN`.
    pub dry_run: bool,
    /// `TELEGRAM_MAX_MESSAGE_AGE_S`, 10..=86400, default 300.
    pub max_message_age_s: u32,
    /// `TELEGRAM_POLL_TIMEOUT_S`, 1..=50, default 30.
    pub poll_timeout_s: u32,
    /// `TELEGRAM_HISTORY_TOKENS`, 0..=50000, default 5000.
    pub history_tokens: u32,
    /// `TELEGRAM_DEBOUNCE_MS`, 0..=30000, default 2500.
    pub debounce_ms: u64,
    /// `TELEGRAM_GIF_COOLDOWN_S`, 0..=86400, default 600.
    pub gif_cooldown_s: u32,
}

/// Seeing, drawing, filming and GIFs.
#[derive(Debug, Clone, PartialEq)]
pub struct MediaConfig {
    /// `VISION_MODEL`, default `vision`; blank disables vision.
    pub vision_model: String,
    /// `VISION_BASE_URL`, optional URL; unset uses the ladder.
    pub vision_base_url: Option<String>,
    /// `VISION_API_KEY`, optional.
    pub vision_api_key: Option<Secret>,
    /// `IMAGE_GEN_ENABLED`, default off.
    pub image_gen_enabled: bool,
    /// `IMAGE_GEN_MODEL`, default `image`.
    pub image_gen_model: String,
    /// `IMAGE_GEN_BASE_URL`, optional URL.
    pub image_gen_base_url: Option<String>,
    /// `IMAGE_GEN_API_KEY`, optional.
    pub image_gen_api_key: Option<Secret>,
    /// `IMAGE_GEN_QUALITY`, optional.
    pub image_gen_quality: Option<String>,
    /// `VIDEO_GEN_ENABLED`, default off.
    pub video_gen_enabled: bool,
    /// `VIDEO_GEN_API`, default `ladder`.
    pub video_gen_api: VideoGenApi,
    /// `VIDEO_GEN_MODEL`, default `video`.
    pub video_gen_model: String,
    /// `VIDEO_GEN_BASE_URL`, optional URL.
    pub video_gen_base_url: Option<String>,
    /// `VIDEO_GEN_API_KEY`, optional.
    pub video_gen_api_key: Option<Secret>,
    /// `VIDEO_GEN_SECONDS`, 1..=60, default 4.
    pub video_gen_seconds: u32,
    /// `VIDEO_GEN_TIMEOUT_S`, 30..=3600, default 600.
    pub video_gen_timeout_s: u32,
    /// `MEDIA_OUTPUT_DIR`, default `data/media`.
    pub output_dir: String,
    /// `MEDIA_IMAGES_PER_DAY`, 0..=1000, default 20.
    pub images_per_day: u32,
    /// `MEDIA_VIDEOS_PER_DAY`, 0..=100, default 3.
    pub videos_per_day: u32,
    /// `TINY_REFERENCE_IMAGES`, comma separated URLs or paths; blank means
    /// [`DEFAULT_REFERENCE_IMAGES`], `none` turns the character sheet off.
    pub tiny_reference_images: Vec<String>,
    /// `REFERENCE_SHEET_PATH`, default `data/media/reference-sheet.json`.
    pub reference_sheet_path: String,
    /// `GIPHY_API_KEY`, optional.
    pub giphy_api_key: Option<Secret>,
    /// `GIPHY_CHANNEL`, default `TinyHumansAI`.
    pub giphy_channel: String,
}
