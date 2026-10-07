use super::*;

pub(crate) fn coin(symbol: &str, market_cap_usd: f64) -> ListedCoin {
    ListedCoin {
        id: symbol.to_lowercase(),
        symbol: symbol.into(),
        name: symbol.into(),
        price_usd: market_cap_usd / 1_000_000_000.0,
        market_cap_usd,
        circulating_supply: 1_000_000_000.0,
        rank: None,
        board_rank: None,
        change24h_pct: None,
        ath_usd: None,
        image_url: None,
    }
}

pub(crate) fn tiny() -> ListedCoin {
    ListedCoin {
        id: "tiny-humans-ai".into(),
        price_usd: 0.0005,
        change24h_pct: Some(8.3),
        board_rank: Some(9),
        ..coin("TINY", 500_000.0)
    }
}

pub(crate) fn board() -> Vec<ListedCoin> {
    vec![
        ListedCoin {
            rank: Some(150.0),
            board_rank: Some(1),
            ..coin("TIBBIR", 248_000_000.0)
        },
        coin("FARTCOIN", 143_000_000.0),
        coin("TURBO", 63_000_000.0),
        coin("AIC", 32_000_000.0),
        ListedCoin {
            board_rank: Some(5),
            ..coin("DRB", 18_500_000.0)
        },
        coin("CRABAI", 14_700_000.0),
        coin("LUNA", 4_900_000.0),
        coin("AIDOGE", 1_950_000.0),
        coin("SMALL", 600_000.0),
        tiny(),
        coin("BELOW", 100_000.0),
    ]
}

fn band(random: f64) -> PickOptions {
    PickOptions {
        min_multiple: 2.0,
        max_multiple: 100.0,
        exclude: Vec::new(),
        random,
    }
}

#[test]
fn only_picks_above_the_floor_inside_the_ceiling_and_never_itself() {
    let mut picks: Vec<String> = (0..50)
        .filter_map(|step| pick_target(&tiny(), &board(), &band(f64::from(step) / 50.0)))
        .map(|coin| coin.symbol)
        .collect();
    picks.sort();
    picks.dedup();
    assert_eq!(picks, ["AIC", "AIDOGE", "CRABAI", "DRB", "LUNA"]);
}

#[test]
fn skips_what_memory_says_was_used_and_resets_when_everything_was() {
    assert_eq!(
        pick_target(&tiny(), &board(), &band(0.0)).unwrap().symbol,
        "AIC"
    );
    let skip = PickOptions {
        exclude: vec!["AIC".into()],
        ..band(0.0)
    };
    assert_eq!(pick_target(&tiny(), &board(), &skip).unwrap().symbol, "DRB");
    let all = ["AIC", "AIDOGE", "CRABAI", "DRB", "LUNA"]
        .map(String::from)
        .to_vec();
    let all = PickOptions {
        exclude: all,
        ..band(0.0)
    };
    assert_eq!(pick_target(&tiny(), &board(), &all).unwrap().symbol, "AIC");
}

#[test]
fn falls_back_to_anything_above_and_to_nothing_when_nothing_is_above() {
    let high = PickOptions {
        min_multiple: 400.0,
        max_multiple: 450.0,
        ..band(0.0)
    };
    assert_eq!(
        pick_target(&tiny(), &board(), &high).unwrap().symbol,
        "TIBBIR"
    );
    assert!(
        pick_target(
            &tiny(),
            &[coin("BELOW", 100_000.0)],
            &PickOptions::default()
        )
        .is_none()
    );
}

#[test]
fn reads_used_tickers_out_of_recalled_prose_as_whole_words() {
    let recalled = "Posted $FARTCOIN at 286x on Monday; before that TURBO (127x). Someone mentioned the AIC token too.";
    let mut used = used_symbols(recalled, &board());
    used.sort();
    assert_eq!(used, ["AIC", "FARTCOIN", "TURBO"]);
    assert!(used_symbols("Lunar eclipse tonight", &board()).is_empty());
    assert!(used_symbols("", &board()).is_empty());
}

#[test]
fn a_ticker_must_be_drawable_and_sayable() {
    assert!(!printable("e/acc"));
    assert!(!printable("q*"));
    assert!(!printable("\u{3000}"));
    assert!(printable("FARTCOIN"));
    assert!(printable("AI16Z"));
}

#[test]
fn carries_a_board_row_over_to_the_picture() {
    let drawn = drawn(&tiny());
    assert_eq!(drawn.board_rank, Some(9));
    assert_eq!(drawn.change_24h_pct, Some(8.3));
    assert_eq!(drawn.symbol, "TINY");
}
