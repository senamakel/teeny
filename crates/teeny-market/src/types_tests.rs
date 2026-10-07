use super::*;

const DAY: i64 = 86_400;

fn daily(count: i64) -> Vec<Candle> {
    (0..count)
        .map(|index| {
            let i = index as f64;
            Candle {
                time: index * DAY,
                open: 10.0 + i,
                high: 12.0 + i,
                low: 8.0 + i,
                close: 11.0 + i,
                volume: 100.0,
            }
        })
        .collect()
}

#[test]
fn knows_weekly_has_to_be_built_rather_than_fetched() {
    assert_eq!(
        Timeframe::W1.spec().fold_from,
        Some(FoldFrom {
            timeframe: Timeframe::D1,
            factor: 7
        })
    );
    assert_eq!(Timeframe::D1.spec().fold_from, None);
    assert_eq!(Timeframe::H12.spec().seconds, 43_200);
}

#[test]
fn takes_the_first_open_the_last_close_and_the_extremes_of_the_bucket() {
    let week = fold(&daily(7), DAY * 7);
    assert_eq!(week.len(), 1);
    let bar = week[0];
    assert_eq!(
        (bar.open, bar.close, bar.high, bar.low, bar.volume),
        (10.0, 17.0, 18.0, 8.0, 700.0)
    );
}

#[test]
fn sums_volume_rather_than_averaging_it() {
    let volumes: Vec<f64> = fold(&daily(14), DAY * 7).iter().map(|c| c.volume).collect();
    assert_eq!(volumes, vec![700.0, 700.0]);
}

/// Counting back from the newest bar would silently redraw every weekly
/// candle depending on what hour the daily job happened to run.
#[test]
fn puts_a_bar_in_the_same_bucket_regardless_of_what_else_was_fetched() {
    let all = fold(&daily(21), DAY * 7);
    let fewer = fold(&daily(21)[3..], DAY * 7);
    let shared: Vec<&Candle> = all
        .iter()
        .filter(|c| fewer.iter().any(|o| o.time == c.time))
        .collect();
    assert!(!shared.is_empty());
    for candle in &shared[1..] {
        assert_eq!(fewer.iter().find(|o| o.time == candle.time), Some(*candle));
    }
}

#[test]
fn keeps_a_partial_bucket_rather_than_dropping_the_current_week() {
    let weeks = fold(&daily(10), DAY * 7);
    assert_eq!(weeks.len(), 2);
    assert_eq!(weeks[1].volume, 300.0);
}

#[test]
fn returns_nothing_for_nothing() {
    assert!(fold(&[], DAY * 7).is_empty());
}

#[test]
fn spells_timeframes_as_the_tools_and_the_config_do() {
    assert_eq!("1w".parse::<Timeframe>(), Ok(Timeframe::W1));
    assert_eq!(serde_json::to_string(&Timeframe::M15).unwrap(), "\"15m\"");
    assert_eq!(
        Timeframe::from(teeny_core::config::ChartTimeframe::H4),
        Timeframe::H4
    );
}

#[test]
fn serialises_an_overview_in_camel_case_without_the_blanks() {
    let overview = TokenOverview {
        address: "A".into(),
        chain: "solana".into(),
        name: None,
        symbol: Some("TINY".into()),
        price_usd: 0.5,
        market_cap_usd: None,
        fdv_usd: None,
        liquidity_usd: None,
        volume24h_usd: Some(10.0),
        price_change: PriceChange {
            h24: Some(1.5),
            ..PriceChange::default()
        },
        txns24h: None,
        pool_address: None,
        pool_name: None,
        pair_created_at: None,
        source: "dexscreener".into(),
    };
    assert_eq!(
        serde_json::to_string(&overview).unwrap(),
        r#"{"address":"A","chain":"solana","symbol":"TINY","priceUsd":0.5,"volume24hUsd":10.0,"priceChange":{"h24":1.5},"source":"dexscreener"}"#
    );
}
