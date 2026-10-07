//! Port of `legacy/test/config.test.ts`, plus the defaults, the validation and
//! the deliberate departures from the legacy schema.
//!
//! A filled-in .env spells "not set" as `KEY=`, so every optional value with a
//! format has to accept the empty spelling. Getting this wrong is not a config
//! warning: the process refuses to boot and the deploy's health check is what
//! tells you.

use super::*;
use crate::{ConfigError, Secret};
use chrono::NaiveDate;

const REQUIRED: &[(&str, &str)] = &[
    ("X_BOT_USERNAME", "teenydroid"),
    ("X_API_KEY", "k"),
    ("X_API_SECRET", "s"),
    ("X_ACCESS_TOKEN", "t"),
    ("X_ACCESS_TOKEN_SECRET", "ts"),
    ("CORTEX_TOKEN", "c"),
    ("LLM_BASE_URL", "http://ladder:6969/v1"),
    ("LLM_API_KEY", "l"),
    ("LLM_MODEL", "reasoning"),
];

fn vars(extra: &[(&str, &str)]) -> Vec<(String, String)> {
    let mut all: Vec<(String, String)> = REQUIRED
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    for (key, value) in extra {
        all.retain(|(k, _)| k != key);
        all.push(((*key).to_owned(), (*value).to_owned()));
    }
    all
}

fn load(extra: &[(&str, &str)]) -> Config {
    match Config::from_vars(vars(extra)) {
        Ok(config) => config,
        Err(error) => panic!("expected the config to load, got: {error}"),
    }
}

fn load_err(extra: &[(&str, &str)]) -> ConfigError {
    match Config::from_vars(vars(extra)) {
        Ok(_) => panic!("expected the config to be refused"),
        Err(error) => error,
    }
}

fn keys(error: &ConfigError) -> Vec<&str> {
    error
        .issues()
        .iter()
        .map(|issue| issue.key.as_str())
        .collect()
}

mod required {
    use super::*;

    #[test]
    fn loads_with_only_the_required_keys() {
        let config = load(&[]);
        assert_eq!(config.x.bot_username, "teenydroid");
        assert_eq!(config.x.api_key.expose(), "k");
        assert_eq!(config.cortex.token.expose(), "c");
        assert_eq!(config.llm.base_url, "http://ladder:6969/v1");
        assert_eq!(config.llm.model, "reasoning");
    }

    #[test]
    fn names_every_missing_key_at_once() {
        let error = Config::from_vars(Vec::<(String, String)>::new()).unwrap_err();
        assert_eq!(
            keys(&error),
            [
                "X_BOT_USERNAME",
                "X_API_KEY",
                "X_API_SECRET",
                "X_ACCESS_TOKEN",
                "X_ACCESS_TOKEN_SECRET",
                "CORTEX_TOKEN",
                "LLM_API_KEY",
                "LLM_MODEL",
            ]
        );
    }

    #[test]
    fn treats_a_blank_required_key_as_missing() {
        let error = load_err(&[("X_API_KEY", "")]);
        assert_eq!(keys(&error), ["X_API_KEY"]);
    }

    #[test]
    fn never_puts_a_value_in_the_error() {
        let error = load_err(&[("TELEGRAM_BOT_TOKEN", "hunter2-not-a-token")]);
        assert!(!error.to_string().contains("hunter2"));
    }

    #[test]
    fn redacts_secrets_from_debug_output() {
        let config = load(&[("X_API_SECRET", "super-secret-value")]);
        assert!(!format!("{config:?}").contains("super-secret-value"));
    }
}

mod dry_run {
    use super::*;

    #[test]
    fn is_a_rehearsal_unless_switched_off() {
        let config = load(&[]);
        assert!(config.dry_run);
        assert!(!config.telegram.dry_run);
        assert!(!config.article.publish);
    }

    #[test]
    fn goes_live_when_switched_off() {
        assert!(!load(&[("DRY_RUN", "false")]).dry_run);
    }

    #[test]
    fn rejects_anything_but_true_or_false() {
        assert_eq!(keys(&load_err(&[("DRY_RUN", "no")])), ["DRY_RUN"]);
    }

    #[test]
    fn keeps_every_publishing_job_off_by_default() {
        let config = load(&[]);
        assert!(!config.chart.enabled);
        assert!(!config.morning.enabled);
        assert!(!config.moon.enabled);
        assert!(!config.lore.enabled);
        assert!(!config.quote.enabled);
        assert!(!config.narrative.enabled);
        assert!(!config.article.enabled);
        assert!(!config.calendar.enabled);
        assert!(!config.media.image_gen_enabled);
        assert!(!config.media.video_gen_enabled);
    }
}

mod wallet {
    use super::*;

    // The wallet is not ported (see the plan): its settings are ignored rather
    // than validated, so an old .env with them filled in, or malformed, boots.
    #[test]
    fn ignores_the_legacy_airdrop_settings() {
        load(&[
            ("AIRDROP_ENABLED", "maybe"),
            ("AIRDROP_PRIVATE_KEY", "0xnope"),
            ("AIRDROP_TOKEN_ADDRESS", "0xnope"),
        ]);
    }
}

mod chart_agent {
    use super::*;

    #[test]
    fn is_off_unless_it_is_switched_on_because_it_publishes() {
        let config = load(&[]);
        assert!(!config.chart.enabled);
        assert_eq!(config.chart.renderer, ChartRenderer::Native);
    }

    #[test]
    fn accepts_a_blank_token_address_rather_than_refusing_to_boot() {
        let config = load(&[
            ("CHART_TOKEN_ADDRESS", ""),
            ("CHART_URL_TEMPLATE", ""),
            ("GMGN_BASE_URL", ""),
            ("SOLANA_RPC_URL", ""),
        ]);
        assert_eq!(config.chart.token_address, None);
        assert_eq!(config.chart.gmgn_base_url, None);
    }

    #[test]
    fn rejects_a_token_address_too_short_to_be_one() {
        assert_eq!(
            keys(&load_err(&[("CHART_TOKEN_ADDRESS", "short")])),
            ["CHART_TOKEN_ADDRESS"]
        );
    }

    #[test]
    fn parses_the_watchlist_into_wallets_and_labels() {
        let config = load(&[("KOL_WALLETS", " Wallet1:a big fund , Wallet2 ,")]);
        assert_eq!(
            config.chart.kol_wallets,
            [
                KolWallet {
                    wallet: "Wallet1".into(),
                    label: "a big fund".into()
                },
                // An entry with no label still counts: knowing it is on the
                // list is the point, and "watchlist" is a truthful name for it.
                KolWallet {
                    wallet: "Wallet2".into(),
                    label: "watchlist".into()
                },
            ]
        );
    }

    #[test]
    fn has_an_empty_watchlist_by_default_so_no_wallet_is_ever_mislabelled() {
        assert!(load(&[]).chart.kol_wallets.is_empty());
    }

    #[test]
    fn reads_the_timeframe_and_refuses_one_it_does_not_know() {
        assert_eq!(load(&[]).chart.timeframe, ChartTimeframe::H1);
        assert_eq!(
            load(&[("CHART_TIMEFRAME", "4h")]).chart.timeframe,
            ChartTimeframe::H4
        );
        assert_eq!(
            keys(&load_err(&[("CHART_TIMEFRAME", "2h")])),
            ["CHART_TIMEFRAME"]
        );
    }
}

mod telegram {
    use super::*;

    #[test]
    fn is_off_when_the_token_is_blank_which_is_how_a_filled_in_env_spells_absent() {
        let config = load(&[("TELEGRAM_BOT_TOKEN", "")]);
        assert_eq!(config.telegram.bot_token, None);
        assert!(!config.telegram.dry_run);
        assert_eq!(config.telegram.max_message_age_s, 300);
        assert_eq!(config.telegram.history_tokens, 5_000);
    }

    #[test]
    fn accepts_a_token_in_the_shape_botfather_hands_out() {
        let token = "8782739916:AAGsGFV_6Rxbng3KmKWp8FgPe0gbiDTis-o";
        let config = load(&[("TELEGRAM_BOT_TOKEN", token)]);
        assert_eq!(
            config.telegram.bot_token.as_ref().map(Secret::expose),
            Some(token)
        );
    }

    #[test]
    fn rejects_something_that_is_not_a_bot_token_rather_than_polling_with_it() {
        assert_eq!(
            keys(&load_err(&[("TELEGRAM_BOT_TOKEN", "not-a-token")])),
            ["TELEGRAM_BOT_TOKEN"]
        );
    }
}

mod media {
    use super::*;

    #[test]
    fn sees_by_default_draws_and_films_only_when_switched_on_and_points_everything_at_the_ladder() {
        let config = load(&[]);
        assert_eq!(config.media.vision_model, "vision");
        assert!(!config.media.image_gen_enabled);
        assert!(!config.media.video_gen_enabled);
        assert_eq!(config.media.image_gen_model, "image");
        assert_eq!(config.media.video_gen_model, "video");
        assert_eq!(config.media.video_gen_api, VideoGenApi::Ladder);
        assert_eq!(config.media.image_gen_base_url, None);
        assert_eq!(config.media.tiny_reference_images.len(), 10);
        let first = &config.media.tiny_reference_images[0];
        assert!(first.starts_with("assets/tiny/") && first.ends_with(".jpg"));
    }

    #[test]
    fn accepts_the_blanks_a_half_filled_env_contains_and_reads_the_reference_list() {
        let config = load(&[
            ("VISION_MODEL", ""),
            ("VISION_BASE_URL", ""),
            ("IMAGE_GEN_API_KEY", ""),
            ("IMAGE_GEN_QUALITY", ""),
            ("VIDEO_GEN_BASE_URL", ""),
            (
                "TINY_REFERENCE_IMAGES",
                " https://example.com/a.jpg, assets/tiny/b.jpg ,",
            ),
        ]);
        assert_eq!(config.media.vision_model, "");
        assert_eq!(config.media.image_gen_quality, None);
        assert_eq!(
            config.media.tiny_reference_images,
            ["https://example.com/a.jpg", "assets/tiny/b.jpg"]
        );

        let none = load(&[("TINY_REFERENCE_IMAGES", "none")]);
        assert!(none.media.tiny_reference_images.is_empty());
        let blank = load(&[("TINY_REFERENCE_IMAGES", "")]);
        assert_eq!(blank.media.tiny_reference_images.len(), 10);
    }

    #[test]
    fn rejects_a_base_url_that_is_not_a_url() {
        assert_eq!(
            keys(&load_err(&[("VISION_BASE_URL", "ladder")])),
            ["VISION_BASE_URL"]
        );
    }
}

mod content_filter {
    use super::*;

    #[test]
    fn runs_the_judge_unless_it_is_switched_off_and_rides_on_the_chat_model_by_default() {
        let config = load(&[]);
        assert!(config.safety.judge);
        assert_eq!(config.safety.judge_model, None);
        assert!(!load(&[("SAFETY_JUDGE", "false")]).safety.judge);
        assert_eq!(load(&[("SAFETY_JUDGE_MODEL", "")]).safety.judge_model, None);
        assert_eq!(
            load(&[("SAFETY_JUDGE_MODEL", "fast")]).safety.judge_model,
            Some("fast".to_owned())
        );
    }

    #[test]
    fn parses_the_extra_link_allowlist_and_is_empty_by_default() {
        assert!(load(&[]).safety.link_allowlist.is_empty());
        assert_eq!(
            load(&[("LINK_ALLOWLIST", "dexscreener.com/solana, solscan.io ,,")])
                .safety
                .link_allowlist,
            ["dexscreener.com/solana", "solscan.io"]
        );
    }
}

mod critic {
    use super::*;

    #[test]
    fn is_on_by_default_on_the_fast_rung_and_a_blank_model_spells_the_default() {
        let config = load(&[("ALGO_CRITIC_MODEL", "")]);
        assert!(config.safety.algo_critic);
        assert_eq!(config.safety.algo_critic_model, "flash");
        assert_eq!(
            load(&[("ALGO_CRITIC_MODEL", "reasoning")])
                .safety
                .algo_critic_model,
            "reasoning"
        );
        assert!(!load(&[("ALGO_CRITIC", "false")]).safety.algo_critic);
    }
}

mod morning_timeline {
    use super::*;

    #[test]
    fn reads_twenty_posts_a_corner_over_the_last_day_by_default_and_zero_turns_it_off() {
        let config = load(&[]);
        assert_eq!(config.morning.x_posts_per_query, 20);
        assert_eq!(config.morning.x_hours, 24);
        assert_eq!(config.morning.x_queries, "");
        assert_eq!(config.morning.x_trends_woeid, 23_424_977);
        let off = load(&[
            ("MORNING_X_POSTS_PER_QUERY", "0"),
            ("MORNING_X_HOURS", "48"),
        ]);
        assert_eq!(off.morning.x_posts_per_query, 0);
        assert_eq!(off.morning.x_hours, 48);
    }
}

mod numbers {
    use super::*;

    #[test]
    fn reads_the_defaults() {
        let config = load(&[]);
        assert_eq!(config.server.port, 3001);
        assert_eq!(config.reply.max_per_run, 5);
        assert_eq!(config.reply.idle_ms, 2_000);
        assert_eq!(config.reply.min_interval_ms, 30_000);
        assert_eq!(config.reply.max_interval_ms, 60_000);
        assert_eq!(config.x.cache_post_ttl_days, 30.0);
        assert_eq!(config.cadence.post_min_gap_hours, 6.0);
        assert_eq!(config.narrative.straight_temperature, 0.6);
        assert_eq!(config.backrooms.budget_tokens, 9_000);
    }

    #[test]
    fn reads_a_set_value() {
        let config = load(&[("PORT", "8080"), ("NARRATIVE_WILD_TEMPERATURE", "1.5")]);
        assert_eq!(config.server.port, 8080);
        assert_eq!(config.narrative.wild_temperature, 1.5);
    }

    #[test]
    fn refuses_values_out_of_range_not_a_number_or_not_whole() {
        let error = load_err(&[
            ("PORT", "70000"),
            ("MAX_REPLIES_PER_RUN", "0"),
            ("AGENT_MAX_STEPS", "many"),
            ("CHART_BARS", "120.5"),
            ("NARRATIVE_WILD_TEMPERATURE", "3"),
        ]);
        assert_eq!(
            keys(&error),
            [
                "PORT",
                "MAX_REPLIES_PER_RUN",
                "AGENT_MAX_STEPS",
                "CHART_BARS",
                "NARRATIVE_WILD_TEMPERATURE",
            ]
        );
    }

    // Legacy coerces with `Number()`, which reads a blank as zero: a blank
    // count is zero, and a blank port is refused rather than defaulted.
    #[test]
    fn reads_a_blank_number_as_zero_like_the_legacy_schema() {
        assert_eq!(
            load(&[("MORNING_X_POSTS_PER_QUERY", "")])
                .morning
                .x_posts_per_query,
            0
        );
        assert_eq!(keys(&load_err(&[("PORT", "")])), ["PORT"]);
    }
}

mod flags {
    use super::*;

    #[test]
    fn are_on_by_default() {
        let config = load(&[]);
        assert!(config.x.cache_enabled);
        assert!(config.x.activity_stream_enabled);
        assert!(config.cadence.heat_enabled);
        assert!(config.reply.like_mentions);
    }

    // Legacy read these with `z.coerce.boolean()`, under which the string
    // "false" is true; .env.example documents `X_CACHE_ENABLED=false` as the
    // off switch. The port makes the documented spelling work.
    #[test]
    fn turn_off_when_set_to_false_as_the_env_example_documents() {
        for off in ["false", "0", "off", "no", ""] {
            let config = load(&[
                ("X_CACHE_ENABLED", off),
                ("ACTIVITY_STREAM_ENABLED", off),
                ("HEAT_ENABLED", off),
                ("REPLY_LIKE_MENTIONS", off),
            ]);
            assert!(!config.x.cache_enabled, "{off:?}");
            assert!(!config.x.activity_stream_enabled, "{off:?}");
            assert!(!config.cadence.heat_enabled, "{off:?}");
            assert!(!config.reply.like_mentions, "{off:?}");
        }
        assert!(load(&[("HEAT_ENABLED", "1")]).cadence.heat_enabled);
    }

    #[test]
    fn refuse_a_value_that_is_neither() {
        assert_eq!(
            keys(&load_err(&[("HEAT_ENABLED", "sometimes")])),
            ["HEAT_ENABLED"]
        );
    }
}

mod optional_values {
    use super::*;

    // Legacy refused a blank here (`z.string().min(1).optional()`), which made
    // .env.example itself fail to boot; blank now means absent, as it does
    // for every other optional value.
    #[test]
    fn treat_a_blank_as_absent() {
        let config = load(&[
            ("EXA_API_KEY", ""),
            ("AGENT_API_TOKEN", ""),
            ("WEBHOOK_FORWARD_URL", ""),
            ("WEBHOOK_FORWARD_TOKEN", ""),
            ("GIPHY_API_KEY", ""),
            ("X_BEARER_TOKEN", ""),
        ]);
        assert!(config.agent.exa_api_key.is_none());
        assert!(config.agent.api_token.is_none());
        assert!(config.webhook.forward_url.is_none());
        assert!(config.webhook.forward_token.is_none());
        assert!(config.media.giphy_api_key.is_none());
        assert!(config.x.bearer_token.is_none());
    }

    #[test]
    fn still_refuse_a_present_value_that_is_too_short_or_malformed() {
        let error = load_err(&[
            ("AGENT_API_TOKEN", "short"),
            ("WEBHOOK_FORWARD_URL", "not a url"),
        ]);
        assert_eq!(keys(&error), ["AGENT_API_TOKEN", "WEBHOOK_FORWARD_URL"]);
    }

    #[test]
    fn serve_the_brain_by_default_and_neither_when_blanked() {
        assert_eq!(load(&[]).brain.data_dir.as_deref(), Some("brain"));
        assert_eq!(load(&[("BRAIN_DATA_DIR", "")]).brain.data_dir, None);
    }

    #[test]
    fn refuse_a_blank_where_the_default_is_a_name() {
        assert_eq!(
            keys(&load_err(&[("MONGO_DATABASE", "")])),
            ["MONGO_DATABASE"]
        );
    }
}

mod lists {
    use super::*;

    #[test]
    fn read_their_defaults() {
        let config = load(&[]);
        assert_eq!(
            config.agent.trusted_handles,
            ["TinyHumansAISol", "senamakel", "enamakel", "tinyhumansai"]
        );
        assert_eq!(config.agent.self_names, ["Teeny"]);
        assert_eq!(config.backrooms.agents, ["teeny", "swarm"]);
        assert_eq!(config.moon.exclude_ids, ["tutorial"]);
        assert!(config.agent.own_token_addresses.is_empty());
    }

    #[test]
    fn a_blank_list_is_empty_not_the_default() {
        assert!(
            load(&[("TRUSTED_HANDLES", "")])
                .agent
                .trusted_handles
                .is_empty()
        );
    }

    #[test]
    fn the_lore_seeds_must_be_known_and_at_least_one() {
        assert_eq!(load(&[]).lore.seeds, [LoreSeed::News, LoreSeed::Forum]);
        assert_eq!(
            load(&[("LORE_SEEDS", "memory, news")]).lore.seeds,
            [LoreSeed::Memory, LoreSeed::News]
        );
        assert_eq!(keys(&load_err(&[("LORE_SEEDS", "")])), ["LORE_SEEDS"]);
        assert_eq!(
            keys(&load_err(&[("LORE_SEEDS", "news,rumour")])),
            ["LORE_SEEDS"]
        );
    }
}

mod rota {
    use super::*;

    #[test]
    fn reads_the_week_from_its_anchor() {
        let config = load(&[]);
        assert_eq!(config.rota.days, 7);
        assert_eq!(
            config.rota.anchor,
            NaiveDate::from_ymd_opt(2026, 9, 14).unwrap()
        );
        assert_eq!(config.rota.moon_slot, 0);
        assert_eq!(config.rota.chart_slot, 3);
        assert_eq!(config.rota.narrative_slots, [1, 4]);
        assert_eq!(config.rota.article_slots, [2, 5]);
        assert_eq!(config.rota.evening_cron, "0 19 * * *");
    }

    #[test]
    fn drops_slots_that_are_not_whole_numbers() {
        assert_eq!(
            load(&[("NARRATIVE_ROTA_SLOTS", " 3 , x, -1, 2.5, 6")])
                .rota
                .narrative_slots,
            [3, 6]
        );
    }

    // Legacy read an empty entry as `Number('') == 0`, so a trailing comma
    // or a blank list quietly handed Monday to the serial.
    #[test]
    fn reads_a_blank_slot_list_as_no_slots() {
        assert!(
            load(&[("ARTICLE_ROTA_SLOTS", "")])
                .rota
                .article_slots
                .is_empty()
        );
        assert_eq!(
            load(&[("ARTICLE_ROTA_SLOTS", "2,5,")]).rota.article_slots,
            [2, 5]
        );
    }

    #[test]
    fn refuses_an_anchor_that_is_not_a_date() {
        assert_eq!(
            keys(&load_err(&[("POST_ROTA_ANCHOR", "14/09/2026")])),
            ["POST_ROTA_ANCHOR"]
        );
        assert_eq!(
            keys(&load_err(&[("POST_ROTA_ANCHOR", "2026-9-14")])),
            ["POST_ROTA_ANCHOR"]
        );
    }
}

mod roles {
    use super::*;

    #[test]
    fn run_everything_in_one_process_by_default() {
        let config = load(&[]);
        assert_eq!(config.server.service_role, ServiceRole::All);
        assert_eq!(config.agent.mode, AgentMode::Langgraph);
        assert_eq!(config.reply.mode, ReplyMode::Worker);
    }

    #[test]
    fn read_a_split_role_and_refuse_an_unknown_one() {
        assert_eq!(
            load(&[("SERVICE_ROLE", "ingest")]).server.service_role,
            ServiceRole::Ingest
        );
        let error = load_err(&[("SERVICE_ROLE", "everything")]);
        assert_eq!(keys(&error), ["SERVICE_ROLE"]);
        assert!(error.to_string().contains("all, ingest, reply"));
    }
}

mod from_env {
    use super::*;

    #[test]
    fn reads_the_process_environment() {
        // Only asserts the shape of the call: the process environment of a
        // test run has no X credentials, so it is refused, naming them.
        if std::env::var_os("X_API_KEY").is_none() {
            let error = Config::from_env().unwrap_err();
            assert!(keys(&error).contains(&"X_API_KEY"));
        }
    }
}

mod openhuman {
    use super::*;

    #[test]
    fn defaults_to_a_local_data_dir_clicks_and_presses_and_no_browsing() {
        let config = load(&[]).openhuman;
        assert_eq!(config.data_dir, "data");
        assert_eq!(config.chrome_path, None);
        assert_eq!(config.unattended_actions, ["click", "press"]);
        assert!(config.allowed_domains.is_empty(), "browsing is opt-in");
        assert_eq!(config.tinycomputer_module_path, None);
    }

    #[test]
    fn reads_the_runtime_and_browser_variables() {
        let config = load(&[
            ("TEENY_DATA", "/data"),
            ("BROWSER_CHROME_PATH", "/usr/bin/chromium"),
            ("BROWSE_UNATTENDED_ACTIONS", "click, fill"),
            ("BROWSE_ALLOWED_DOMAINS", "dexscreener.com, coingecko.com"),
            (
                "TINYCOMPUTER_MODULE_PATH",
                "/opt/tinycomputer/libtinycomputer.so",
            ),
        ])
        .openhuman;
        assert_eq!(config.data_dir, "/data");
        assert_eq!(config.chrome_path.as_deref(), Some("/usr/bin/chromium"));
        assert_eq!(config.unattended_actions, ["click", "fill"]);
        assert_eq!(config.allowed_domains, ["dexscreener.com", "coingecko.com"]);
        assert_eq!(
            config.tinycomputer_module_path.as_deref(),
            Some("/opt/tinycomputer/libtinycomputer.so")
        );
    }

    #[test]
    fn a_blank_action_list_allows_no_unattended_action() {
        let config = load(&[("BROWSE_UNATTENDED_ACTIONS", "")]).openhuman;
        assert!(config.unattended_actions.is_empty());
    }
}

mod serve_wiring {
    use super::*;

    #[test]
    fn telegram_transport_defaults_to_teenys_own_poller() {
        assert_eq!(load(&[]).telegram.transport, TelegramTransport::Teeny);
        assert_eq!(
            load(&[("TELEGRAM_TRANSPORT", "openhuman")])
                .telegram
                .transport,
            TelegramTransport::OpenHuman
        );
        assert_eq!(
            keys(&load_err(&[("TELEGRAM_TRANSPORT", "carrier-pigeon")])),
            ["TELEGRAM_TRANSPORT"]
        );
    }

    #[test]
    fn calendar_tz_is_optional_and_blank_is_unset() {
        assert_eq!(load(&[]).calendar.timezone, None);
        assert_eq!(load(&[("CALENDAR_TZ", "")]).calendar.timezone, None);
        assert_eq!(
            load(&[("CALENDAR_TZ", "Europe/Berlin")])
                .calendar
                .timezone
                .as_deref(),
            Some("Europe/Berlin")
        );
    }
}
