//! The schema: which variable feeds which field, with what default and range,
//! one function per section. Issues come out in the order the sections are
//! read here.

use chrono::NaiveDate;

use super::Config;
use super::content::*;
use super::env::{Env, split_list};
use super::kinds::*;
use super::service::*;

/// Effectively unbounded, for ranges legacy left open above.
const NO_MAX: u32 = u32::MAX;

pub(super) fn load(env: &mut Env) -> Config {
    Config {
        dry_run: env.switch("DRY_RUN", true),
        server: server(env),
        reply: reply(env),
        x: x(env),
        cortex: cortex(env),
        memory: memory(env),
        learnings: learnings(env),
        llm: llm(env),
        agent: agent(env),
        webhook: WebhookConfig {
            forward_url: env.optional_url("WEBHOOK_FORWARD_URL"),
            forward_token: env.optional_secret("WEBHOOK_FORWARD_TOKEN", 16),
        },
        safety: safety(env),
        cadence: cadence(env),
        store: store(env),
        brain: brain(env),
        rota: rota(env),
        chart: chart(env),
        morning: morning(env),
        moon: moon(env),
        lore: lore(env),
        quote: quote(env),
        narrative: narrative(env),
        backrooms: backrooms(env),
        article: article(env),
        calendar: CalendarConfig {
            enabled: env.switch("CALENDAR_ENABLED", false),
            path: env.text("CALENDAR_PATH", "calendar.json"),
            telegram_chat_ids: env.list("CALENDAR_TELEGRAM_CHAT_IDS", ""),
            timezone: env.optional("CALENDAR_TZ"),
        },
        telegram: telegram(env),
        media: media(env),
        openhuman: openhuman(env),
    }
}

fn server(env: &mut Env) -> ServerConfig {
    ServerConfig {
        // 1..=65535, so the narrowing is exact.
        port: env.int("PORT", 3001, 1, 65_535) as u16,
        log_level: env.text("LOG_LEVEL", "info"),
        service_role: env.choice("SERVICE_ROLE", ServiceRole::All),
    }
}

fn reply(env: &mut Env) -> ReplyConfig {
    ReplyConfig {
        cron: env.text("REPLY_CRON", "*/5 * * * *"),
        ingest_cron: env.text("INGEST_CRON", "*/5 * * * *"),
        max_per_run: env.int("MAX_REPLIES_PER_RUN", 5, 1, 100),
        mode: env.choice("REPLY_MODE", ReplyMode::Worker),
        idle_ms: u64::from(env.int("REPLY_IDLE_MS", 2_000, 250, 60_000)),
        min_interval_ms: u64::from(env.int("REPLY_MIN_INTERVAL_MS", 30_000, 0, 3_600_000)),
        max_interval_ms: u64::from(env.int("REPLY_MAX_INTERVAL_MS", 60_000, 0, 3_600_000)),
        auto_per_post: env.int("REPLY_AUTO_PER_POST", 10, 0, 100),
        smart_per_post: env.int("REPLY_SMART_PER_POST", 10, 0, 100),
        like_mentions: env.flag("REPLY_LIKE_MENTIONS", true),
    }
}

fn x(env: &mut Env) -> XConfig {
    XConfig {
        bot_username: env.required("X_BOT_USERNAME"),
        api_key: env.required_secret("X_API_KEY"),
        api_secret: env.required_secret("X_API_SECRET"),
        access_token: env.required_secret("X_ACCESS_TOKEN"),
        access_token_secret: env.required_secret("X_ACCESS_TOKEN_SECRET"),
        bearer_token: env.optional_secret("X_BEARER_TOKEN", 1),
        cache_enabled: env.flag("X_CACHE_ENABLED", true),
        cache_post_ttl_days: env.number("X_CACHE_POST_TTL_DAYS", 30.0, 0.0, f64::MAX),
        cache_search_ttl_minutes: env.number("X_CACHE_SEARCH_TTL_MINUTES", 10.0, 0.0, f64::MAX),
        mention_lookback_minutes: env.int("MENTION_LOOKBACK_MINUTES", 15, 1, 10_080),
        search_max_results: env.int("X_SEARCH_MAX_RESULTS", 10, 10, 100),
        activity_stream_enabled: env.flag("ACTIVITY_STREAM_ENABLED", true),
        thread_context_max_posts: env.int("THREAD_CONTEXT_MAX_POSTS", 10, 0, 100),
    }
}

fn cortex(env: &mut Env) -> CortexConfig {
    CortexConfig {
        api_url: env.url("CORTEX_API_URL", "https://api-v1.cortexdb.ai"),
        actor: env.name("CORTEX_ACTOR", "agent:teeny"),
        scope: env.name("CORTEX_SCOPE", "agent:teeny"),
        token: env.required_secret("CORTEX_TOKEN"),
        recall_timeout_ms: u64::from(env.int("CORTEX_RECALL_TIMEOUT_MS", 15_000, 1_000, 120_000)),
        capture_timeout_ms: u64::from(env.int("CORTEX_CAPTURE_TIMEOUT_MS", 30_000, 1_000, 120_000)),
        person_recall: env.switch("CORTEX_PERSON_RECALL", true),
        recall_cooldown_ms: u64::from(env.int("CORTEX_RECALL_COOLDOWN_MS", 300_000, 0, 3_600_000)),
        health_cron: env.text("CORTEX_HEALTH_CRON", "17 * * * *"),
    }
}

fn memory(env: &mut Env) -> MemoryConfig {
    MemoryConfig {
        url: env.url("TEENY_MEMORY_URL", "http://127.0.0.1:3143"),
        token: env.optional_secret("TEENY_MEMORY_TOKEN", 1),
        root: env.name("TEENY_MEMORY_ROOT", "team:teeny"),
        build_beliefs_every: env.int("MEMORY_BUILD_BELIEFS_EVERY", 10, 0, 1_000),
        learnings_limit: env.int("MEMORY_LEARNINGS_LIMIT", 8, 0, 50),
        turn_pack: env.switch("MEMORY_TURN_PACK", true),
        exchanges: env.int("MEMORY_EXCHANGES", 6, 0, 50),
        own_posts_hours: env.int("MEMORY_OWN_POSTS_HOURS", 72, 0, 24 * 14),
        own_posts_limit: env.int("MEMORY_OWN_POSTS_LIMIT", 8, 0, 50),
    }
}

fn learnings(env: &mut Env) -> LearningsConfig {
    LearningsConfig {
        enabled: env.switch("LEARNINGS_ENABLED", true),
        path: env.text("LEARNINGS_PATH", "data/memory/LEARNINGS.md"),
        cron: env.text("LEARNINGS_CRON", "0 5 * * *"),
        max_chars: env.int("LEARNINGS_MAX_CHARS", 10_000, 1_000, 40_000),
        days: env.int("LEARNINGS_DAYS", 7, 1, 60),
    }
}

fn llm(env: &mut Env) -> LlmConfig {
    LlmConfig {
        base_url: env.url("LLM_BASE_URL", "https://api.openai.com/v1"),
        api_key: env.required_secret("LLM_API_KEY"),
        model: env.required("LLM_MODEL"),
    }
}

fn agent(env: &mut Env) -> AgentConfig {
    AgentConfig {
        mode: env.choice("AGENT_MODE", AgentMode::Langgraph),
        max_steps: env.int("AGENT_MAX_STEPS", 24, 1, 100),
        api_token: env.optional_secret("AGENT_API_TOKEN", 16),
        exa_api_key: env.optional_secret("EXA_API_KEY", 1),
        soul_path: env.text("SOUL_PATH", "SOUL.md"),
        context_path: env.text("CONTEXT_PATH", "CONTEXT.md"),
        skills_dir: env.text("SKILLS_DIR", "skills"),
        max_emoji_per_post: env.int("MAX_EMOJI_PER_POST", 1, 0, 10),
        own_token_addresses: env.list("OWN_TOKEN_ADDRESSES", ""),
        trusted_handles: env.list(
            "TRUSTED_HANDLES",
            "TinyHumansAISol,senamakel,enamakel,tinyhumansai",
        ),
        self_names: env.list("SELF_NAMES", "Teeny"),
    }
}

fn safety(env: &mut Env) -> SafetyConfig {
    SafetyConfig {
        judge: env.switch("SAFETY_JUDGE", true),
        judge_model: env.optional("SAFETY_JUDGE_MODEL"),
        algo_critic: env.switch("ALGO_CRITIC", true),
        algo_critic_model: env.text_or("ALGO_CRITIC_MODEL", "flash"),
        link_allowlist: env.list("LINK_ALLOWLIST", ""),
    }
}

fn cadence(env: &mut Env) -> CadenceConfig {
    CadenceConfig {
        post_min_gap_hours: env.number("POST_MIN_GAP_HOURS", 7.0, 0.0, 48.0),
        post_gap_tolerance_minutes: env.number("POST_GAP_TOLERANCE_MINUTES", 20.0, 0.0, 120.0),
        heat_enabled: env.flag("HEAT_ENABLED", true),
        heat_cron: env.text("HEAT_CRON", "*/20 * * * *"),
        hot_impressions_per_hour: env.number("HOT_IMPRESSIONS_PER_HOUR", 500.0, 1.0, f64::MAX),
        hot_baseline_multiple: env.number("HOT_BASELINE_MULTIPLE", 3.0, 1.0, f64::MAX),
        hot_max_hold_slots: env.int("HOT_MAX_HOLD_SLOTS", 2, 1, 6),
    }
}

fn store(env: &mut Env) -> StoreConfig {
    StoreConfig {
        mongo_uri: env.name("MONGO_URI", "mongodb://mongo:27017"),
        mongo_database: env.name("MONGO_DATABASE", "teeny"),
        mentions_collection: env.name("MENTIONS_COLLECTION", "mentions"),
        holds_collection: env.name("HOLDS_COLLECTION", "holds"),
        transcript_collection: env.name("TRANSCRIPT_COLLECTION", "telegram_transcript"),
    }
}

fn brain(env: &mut Env) -> BrainConfig {
    BrainConfig {
        // Absent takes the default; blank serves neither.
        data_dir: env.blank_disables("BRAIN_DATA_DIR", "brain"),
        company: env.name("BRAIN_COMPANY", "teeny"),
        usage_cron: env.text("BRAIN_USAGE_CRON", "* * * * *"),
    }
}

fn chart(env: &mut Env) -> ChartConfig {
    ChartConfig {
        enabled: env.switch("CHART_ENABLED", false),
        token_address: env.optional_min("CHART_TOKEN_ADDRESS", 26),
        token_symbol: env.name("CHART_TOKEN_SYMBOL", "TINY"),
        chain: env.name("CHART_CHAIN", "solana"),
        timeframe: env.choice("CHART_TIMEFRAME", ChartTimeframe::H1),
        bars: env.int("CHART_BARS", 120, 20, 500),
        renderer: env.choice("CHART_RENDERER", ChartRenderer::Native),
        url_template: env.optional("CHART_URL_TEMPLATE"),
        browser_channel: env.optional("CHART_BROWSER_CHANNEL"),
        browser_path: env.optional("CHART_BROWSER_PATH"),
        output_dir: env.text("CHART_OUTPUT_DIR", "data/charts"),
        community_query: env.text("CHART_COMMUNITY_QUERY", "$TINY -is:retweet"),
        watermark: env.optional("CHART_WATERMARK"),
        gmgn_base_url: env.optional_url("GMGN_BASE_URL"),
        solana_rpc_url: env.optional_url("SOLANA_RPC_URL"),
        kol_wallets: kol_wallets(&env.raw_list("KOL_WALLETS", "")),
    }
}

/// `address:label` pairs; an entry with no label is on the `watchlist`.
fn kol_wallets(value: &str) -> Vec<KolWallet> {
    split_list(value)
        .iter()
        .filter_map(|entry| {
            let (wallet, label) = entry.split_once(':').unwrap_or((entry, ""));
            let wallet = wallet.trim();
            let label = label.trim();
            (!wallet.is_empty()).then(|| KolWallet {
                wallet: wallet.to_owned(),
                label: if label.is_empty() { "watchlist" } else { label }.to_owned(),
            })
        })
        .collect()
}

fn morning(env: &mut Env) -> MorningConfig {
    MorningConfig {
        enabled: env.switch("MORNING_ENABLED", false),
        cron: env.text("MORNING_CRON", "0 7 * * *"),
        news_topics: env.list(
            "MORNING_NEWS_TOPICS",
            "artificial intelligence news,crypto and web3 news,technology trends",
        ),
        news_days: env.int("MORNING_NEWS_DAYS", 2, 1, 7),
        stories_per_topic: env.int("MORNING_STORIES_PER_TOPIC", 8, 1, 20),
        x_posts_per_query: env.int("MORNING_X_POSTS_PER_QUERY", 20, 0, 100),
        x_hours: env.int("MORNING_X_HOURS", 24, 1, 168),
        x_queries: env.text("MORNING_X_QUERIES", ""),
        x_trends_woeid: env.int("MORNING_X_TRENDS_WOEID", 23_424_977, 0, NO_MAX),
        backrooms: env.switch("MORNING_BACKROOMS", false),
        backrooms_turns: env.int("MORNING_BACKROOMS_TURNS", 10, 2, 60),
        video_seconds: env.int("MORNING_VIDEO_SECONDS", 10, 4, 20),
    }
}

fn rota(env: &mut Env) -> RotaConfig {
    let anchor = NaiveDate::from_ymd_opt(2026, 9, 14).expect("a real date");
    RotaConfig {
        days: env.int("POST_ROTA_DAYS", 3, 1, 14),
        anchor: env.date("POST_ROTA_ANCHOR", anchor),
        afternoon_cron: env.text("AFTERNOON_CRON", "0 15 * * *"),
        chart_slot: env.int("CHART_ROTA_SLOT", 0, 0, NO_MAX),
        moon_slot: env.int("MOON_ROTA_SLOT", 1, 0, NO_MAX),
        evening_cron: env.text("EVENING_CRON", "0 23 * * *"),
        article_slots: env.slots("ARTICLE_ROTA_SLOTS", "1"),
        narrative_slots: env.slots("NARRATIVE_ROTA_SLOTS", "2"),
    }
}

fn moon(env: &mut Env) -> MoonConfig {
    MoonConfig {
        enabled: env.switch("MOON_ENABLED", false),
        coin_id: env.name("MOON_COIN_ID", "tiny-humans-ai"),
        category: env.name("MOON_CATEGORY", "ai-meme-coins"),
        board_name: env.name("MOON_BOARD_NAME", "AI meme coins"),
        board_size: env.int("MOON_BOARD_SIZE", 500, 5, 1_000),
        rungs: env.int("MOON_RUNGS", 7, 1, 12),
        exclude_ids: env.list("MOON_EXCLUDE_IDS", "tutorial"),
        min_multiple: env.number("MOON_MIN_MULTIPLE", 2.0, 1.0, f64::MAX),
        max_multiple: env.number("MOON_MAX_MULTIPLE", 2_000.0, 1.0, f64::MAX),
        output_dir: env.text("MOON_OUTPUT_DIR", "data/moon"),
    }
}

fn lore(env: &mut Env) -> LoreConfig {
    LoreConfig {
        enabled: env.switch("LORE_ENABLED", false),
        seeds: env.choices("LORE_SEEDS", "news,forum"),
        topics: env.list(
            "LORE_TOPICS",
            "ai agents,robots,artificial intelligence,open source ai,memecoins",
        ),
        backrooms_turns: env.int("LORE_BACKROOMS_TURNS", 10, 2, 60),
    }
}

fn quote(env: &mut Env) -> QuoteConfig {
    QuoteConfig {
        enabled: env.switch("QUOTE_ENABLED", false),
        query: env.name(
            "QUOTE_QUERY",
            "$TINY OR \"2AF7CqwieUjUPALL7icuZtL3X7wENdjUjGBMmfV2pump\"",
        ),
        hours: env.int("QUOTE_HOURS", 48, 1, 168),
        posts_per_search: env.int("QUOTE_POSTS_PER_SEARCH", 50, 10, 100),
        author_cooldown_days: env.int("QUOTE_AUTHOR_COOLDOWN_DAYS", 7, 0, 90),
        log_path: env.text("QUOTE_LOG_PATH", "data/quotes.json"),
    }
}

fn narrative(env: &mut Env) -> NarrativeConfig {
    NarrativeConfig {
        enabled: env.switch("NARRATIVE_ENABLED", false),
        voice_path: env.text("NARRATIVE_VOICE_PATH", "narrative/VOICE.md"),
        lore_path: env.text("NARRATIVE_LORE_PATH", "narrative/LORE.md"),
        state_path: env.text("NARRATIVE_STATE_PATH", "data/narrative-state.json"),
        likeness_dir: env.text("NARRATIVE_LIKENESS_DIR", "data/likeness"),
        branches: env.int("NARRATIVE_BRANCHES", 3, 1, 6),
        turns: env.int("NARRATIVE_TURNS", 6, 2, 8),
        video_seconds: env.int("NARRATIVE_VIDEO_SECONDS", 10, 4, 20),
        wild_temperature: env.number("NARRATIVE_WILD_TEMPERATURE", 1.0, 0.0, 2.0),
        straight_temperature: env.number("NARRATIVE_STRAIGHT_TEMPERATURE", 0.6, 0.0, 2.0),
        telegram_chat_ids: env.list("NARRATIVE_TELEGRAM_CHAT_IDS", ""),
    }
}

fn backrooms(env: &mut Env) -> BackroomsConfig {
    BackroomsConfig {
        dir: env.text("BACKROOMS_DIR", "data/backrooms"),
        editor_path: env.text("BACKROOMS_EDITOR_PATH", "narrative/EDITOR.md"),
        budget_tokens: env.int("BACKROOMS_BUDGET_TOKENS", 9_000, 1_000, 30_000),
        turns: env.int("BACKROOMS_TURNS", 12, 2, 60),
        agents: env.list("BACKROOMS_AGENTS", "teeny,swarm"),
        tool_calls: env.int("BACKROOMS_TOOL_CALLS", 3, 0, 8),
        reddit_subs: env.list(
            "REDDIT_SUBS",
            "singularity,artificial,ControlProblem,LocalLLaMA,conspiracy",
        ),
        fourchan_boards: env.list("FOURCHAN_BOARDS", "g,x,sci,biz"),
    }
}

fn article(env: &mut Env) -> ArticleConfig {
    ArticleConfig {
        enabled: env.switch("ARTICLE_ENABLED", false),
        publish: env.switch("ARTICLE_PUBLISH", false),
        dir: env.text("ARTICLES_DIR", "data/articles"),
        rooms: env.int("ARTICLE_ROOMS", 2, 1, 4),
        sections_max: env.int("ARTICLE_SECTIONS_MAX", 4, 3, 5),
        words: env.int("ARTICLE_WORDS", 1_500, 600, 3_000),
        news_topics: env.list(
            "ARTICLE_NEWS_TOPICS",
            "artificial intelligence news,robots and automation,crypto and web3 news",
        ),
        humanizer_path: env.text("ARTICLE_HUMANIZER_PATH", "skills/humanizer/SKILL.md"),
        writer_temperature: env.number("ARTICLE_WRITER_TEMPERATURE", 0.8, 0.0, 2.0),
        min_interval_hours: env.number("ARTICLE_MIN_INTERVAL_HOURS", 48.0, 1.0, 168.0),
        model_timeout_seconds: env.int("ARTICLE_MODEL_TIMEOUT_SECONDS", 360, 30, 1_800),
    }
}

fn telegram(env: &mut Env) -> TelegramConfig {
    TelegramConfig {
        bot_token: env.optional_secret_where(
            "TELEGRAM_BOT_TOKEN",
            is_telegram_token,
            "must be a Telegram bot token",
        ),
        dry_run: env.switch("TELEGRAM_DRY_RUN", false),
        max_message_age_s: env.int("TELEGRAM_MAX_MESSAGE_AGE_S", 300, 10, 86_400),
        poll_timeout_s: env.int("TELEGRAM_POLL_TIMEOUT_S", 30, 1, 50),
        history_tokens: env.int("TELEGRAM_HISTORY_TOKENS", 5_000, 0, 50_000),
        debounce_ms: u64::from(env.int("TELEGRAM_DEBOUNCE_MS", 2_500, 0, 30_000)),
        gif_cooldown_s: env.int("TELEGRAM_GIF_COOLDOWN_S", 600, 0, 86_400),
        transport: env.choice("TELEGRAM_TRANSPORT", TelegramTransport::Teeny),
    }
}

fn media(env: &mut Env) -> MediaConfig {
    MediaConfig {
        vision_model: env.text("VISION_MODEL", "vision"),
        vision_base_url: env.optional_url("VISION_BASE_URL"),
        vision_api_key: env.optional_secret("VISION_API_KEY", 1),
        image_gen_enabled: env.switch("IMAGE_GEN_ENABLED", false),
        image_gen_model: env.name("IMAGE_GEN_MODEL", "image"),
        image_gen_base_url: env.optional_url("IMAGE_GEN_BASE_URL"),
        image_gen_api_key: env.optional_secret("IMAGE_GEN_API_KEY", 1),
        image_gen_quality: env.optional("IMAGE_GEN_QUALITY"),
        video_gen_enabled: env.switch("VIDEO_GEN_ENABLED", false),
        video_gen_api: env.choice("VIDEO_GEN_API", VideoGenApi::Ladder),
        video_gen_model: env.name("VIDEO_GEN_MODEL", "video"),
        video_gen_base_url: env.optional_url("VIDEO_GEN_BASE_URL"),
        video_gen_api_key: env.optional_secret("VIDEO_GEN_API_KEY", 1),
        video_gen_seconds: env.int("VIDEO_GEN_SECONDS", 4, 1, 60),
        video_gen_timeout_s: env.int("VIDEO_GEN_TIMEOUT_S", 600, 30, 3_600),
        output_dir: env.text("MEDIA_OUTPUT_DIR", "data/media"),
        images_per_day: env.int("MEDIA_IMAGES_PER_DAY", 20, 0, 1_000),
        videos_per_day: env.int("MEDIA_VIDEOS_PER_DAY", 3, 0, 100),
        tiny_reference_images: reference_images(&env.raw_list("TINY_REFERENCE_IMAGES", "")),
        reference_sheet_path: env.text("REFERENCE_SHEET_PATH", "data/media/reference-sheet.json"),
        giphy_api_key: env.optional_secret("GIPHY_API_KEY", 1),
        giphy_channel: env.name("GIPHY_CHANNEL", "TinyHumansAI"),
    }
}

/// Blank is the shipped artwork; `none` turns the character sheet off.
fn reference_images(value: &str) -> Vec<String> {
    if value.trim().eq_ignore_ascii_case("none") {
        return Vec::new();
    }
    let urls = split_list(value);
    if urls.is_empty() {
        DEFAULT_REFERENCE_IMAGES
            .iter()
            .map(|s| (*s).to_owned())
            .collect()
    } else {
        urls
    }
}

/// `^\d+:[A-Za-z0-9_-]{30,}$`, the shape BotFather hands out.
fn is_telegram_token(value: &str) -> bool {
    let Some((id, secret)) = value.split_once(':') else {
        return false;
    };
    !id.is_empty()
        && id.bytes().all(|b| b.is_ascii_digit())
        && secret.len() >= 30
        && secret
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn openhuman(env: &mut Env) -> OpenHumanConfig {
    OpenHumanConfig {
        data_dir: env.text("TEENY_DATA", "data"),
        chrome_path: env.optional("BROWSER_CHROME_PATH"),
        unattended_actions: env.list("BROWSE_UNATTENDED_ACTIONS", "click,press"),
        allowed_domains: env.list("BROWSE_ALLOWED_DOMAINS", ""),
        tinycomputer_module_path: env.optional("TINYCOMPUTER_MODULE_PATH"),
    }
}
