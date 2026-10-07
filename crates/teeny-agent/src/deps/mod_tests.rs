//! The wiring: everything a belt needs, from configuration or in process.

use super::*;

fn config(extra: &[(&'static str, &'static str)]) -> Config {
    let mut vars = fake_vars();
    vars.extend_from_slice(extra);
    Config::from_vars(vars).unwrap()
}

#[tokio::test]
async fn wires_the_optional_parts_only_when_configured() {
    let bare = Deps::from_config(config(&[])).await.unwrap();
    assert!(bare.exa.is_none());
    assert!(bare.gifs.is_none());
    assert!(bare.market.is_none());
    assert!(bare.critic.is_some(), "ALGO_CRITIC is on by default");
    assert!(bare.settings.dry_run);

    let full = Deps::from_config(config(&[
        ("EXA_API_KEY", "exa"),
        ("GIPHY_API_KEY", "giphy"),
        (
            "CHART_TOKEN_ADDRESS",
            "2AF7CqwieUjUPALL7icuZtL3X7wENdjUjGBMmfV2pump",
        ),
        ("ALGO_CRITIC", "false"),
    ]))
    .await
    .unwrap();
    assert!(full.exa.is_some());
    assert!(full.gifs.is_some());
    assert_eq!(
        full.market.as_ref().map(|market| market.address.as_str()),
        Some("2AF7CqwieUjUPALL7icuZtL3X7wENdjUjGBMmfV2pump")
    );
    assert!(full.critic.is_none());
}

#[test]
fn reads_the_settings_the_tools_name() {
    let settings = Settings::from_config(&config(&[
        ("X_BOT_USERNAME", "@teenydroid"),
        ("MAX_EMOJI_PER_POST", "2"),
        ("X_SEARCH_MAX_RESULTS", "25"),
    ]));
    assert_eq!(settings.bot_username, "teenydroid");
    assert_eq!(settings.max_emoji, 2);
    assert_eq!(settings.search_limit, 25);
    assert_eq!(settings.gif_channel, "TinyHumansAI");
    assert_eq!(settings.chart_watermark.as_deref(), Some("@teenydroid"));
}

#[test]
fn the_fake_has_every_part_and_rehearses() {
    let deps = Deps::fake();
    assert!(deps.settings.dry_run);
    assert!(deps.exa.is_some() && deps.gifs.is_some() && deps.market.is_some());
    assert!(
        deps.media
            .as_ref()
            .is_some_and(|media| media.can_generate() == (true, true))
    );
    assert!(deps.reply_budget.is_some());
}

#[tokio::test]
async fn keeps_x_reads_in_the_store_as_the_legacy_cache_did() {
    use chrono::{TimeZone, Utc};
    use teeny_x::cache::{CacheEntry, CacheKind, ReadCache};

    let store = Arc::new(teeny_store::InMemoryXCache::default());
    let cache = StoreReadCache(store.clone());
    let at = Utc.with_ymd_and_hms(2026, 10, 7, 12, 0, 0).unwrap();
    let entry = CacheEntry {
        key: "metrics:abc".into(),
        kind: CacheKind::OwnCounters,
        value: serde_json::json!([1]),
        stored_at: at,
        expires_at: at + chrono::TimeDelta::minutes(10),
    };
    cache.put(entry.clone()).await.unwrap();
    let row = teeny_store::XCacheStore::get(store.as_ref(), "metrics:abc")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.kind, teeny_store::XCacheKind::Metrics);
    assert_eq!(row.stored_at, "2026-10-07T12:00:00.000Z");
    assert_eq!(cache.get("metrics:abc").await.unwrap(), Some(entry));
    assert_eq!(cache.get("missing").await.unwrap(), None);
}
