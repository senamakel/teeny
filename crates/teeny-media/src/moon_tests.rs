use super::*;

const PNG: [u8; 12] = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0];

fn coin(symbol: &str, market_cap_usd: f64) -> ListedCoin {
    ListedCoin {
        id: symbol.to_lowercase(),
        symbol: symbol.to_owned(),
        name: symbol.to_owned(),
        price_usd: market_cap_usd / 1_000_000_000.0,
        market_cap_usd,
        circulating_supply: 1_000_000_000.0,
        rank: None,
        board_rank: None,
        change_24h_pct: None,
    }
}

fn tiny() -> ListedCoin {
    ListedCoin {
        id: "tiny-humans-ai".into(),
        price_usd: 0.0005,
        change_24h_pct: Some(8.3),
        board_rank: Some(9),
        ..coin("TINY", 500_000.0)
    }
}

fn board() -> Vec<ListedCoin> {
    vec![
        ListedCoin { rank: Some(150), board_rank: Some(1), ..coin("TIBBIR", 248_000_000.0) },
        coin("FARTCOIN", 143_000_000.0),
        coin("TURBO", 63_000_000.0),
        coin("AIC", 32_000_000.0),
        ListedCoin { board_rank: Some(5), ..coin("DRB", 18_500_000.0) },
        coin("CRABAI", 14_700_000.0),
        coin("LUNA", 4_900_000.0),
        coin("AIDOGE", 1_950_000.0),
        coin("SMALL", 600_000.0),
        tiny(),
        coin("BELOW", 100_000.0),
    ]
}

fn symbols(rungs: &[Rung]) -> Vec<&str> {
    rungs.iter().map(|entry| entry.coin.symbol.as_str()).collect()
}

fn input() -> MoonChartInput {
    let board = board();
    let token = tiny();
    MoonChartInput {
        target: rung(&token, &board[4]),
        ladder: ladder(&token, &board, &board[4], 7, 2.0),
        token,
        board_name: "AI meme coins".into(),
        board_size: Some(12),
        date: "2026-09-14".into(),
        token_logo: None,
        target_logo: None,
        watermark: Some("@TinyMascot".into()),
        width: None,
        height: None,
    }
}

#[test]
fn prices_the_token_at_the_target_cap_with_its_own_supply() {
    let result = rung(&tiny(), &board()[1]);
    assert_eq!(result.multiple.round(), 286.0);
    assert!((result.implied_price_usd - 0.143).abs() < 0.00005);
}

#[test]
fn formats_a_multiple_the_way_a_reader_says_it() {
    assert_eq!(multiple(490.07), "490x");
    assert_eq!(multiple(37.04), "37x");
    assert_eq!(multiple(64.8), "64.8x");
    assert_eq!(multiple(2.5), "2.5x");
    assert_eq!(multiple(f64::NAN), "—");
}

#[test]
fn spreads_the_board_across_the_climb_keeps_the_target_and_sorts_ascending() {
    let board = board();
    let rungs = ladder(&tiny(), &board, &board[4], 5, 2.0);
    assert_eq!(rungs.len(), 5);
    let names = symbols(&rungs);
    assert!(names.contains(&"DRB") && names.contains(&"TIBBIR"));
    assert!(!names.contains(&"BELOW"));
    assert!(rungs.windows(2).all(|pair| pair[1].coin.market_cap_usd > pair[0].coin.market_cap_usd));
}

#[test]
fn leaves_neighbours_under_the_floor_off_the_strip() {
    let board = board();
    assert!(!symbols(&ladder(&tiny(), &board, &board[4], 12, 2.0)).contains(&"SMALL"));
    assert!(symbols(&ladder(&tiny(), &board, &board[4], 12, 1.0)).contains(&"SMALL"));
}

#[test]
fn draws_the_target_alone_when_nothing_else_is_above() {
    let target = coin("ONLY", 5_000_000.0);
    let rungs = ladder(&tiny(), &[target.clone(), coin("BELOW", 100_000.0)], &target, 7, 2.0);
    assert_eq!(symbols(&rungs), vec!["ONLY"]);
}

#[test]
fn draws_byte_for_byte_what_the_legacy_renderer_drew() {
    assert_eq!(render_moon_svg(&input()), include_str!("testdata/moon-plain.svg"));

    let token = tiny();
    let board = board();
    let drop = ListedCoin {
        name: "<script>&\"".into(),
        change_24h_pct: Some(-12.25),
        rank: Some(77),
        ..coin("X", 5_000_000.0)
    };
    let logos = MoonChartInput {
        target: rung(&token, &drop),
        ladder: vec![rung(&token, &drop), rung(&token, &board[3]), rung(&token, &board[2])],
        token_logo: Some(Logo { data: PNG.to_vec(), mime_type: "image/png".into() }),
        target_logo: Some(Logo { data: PNG.to_vec(), mime_type: "image/jpeg".into() }),
        board_size: None,
        watermark: None,
        ..input()
    };
    assert_eq!(render_moon_svg(&logos), include_str!("testdata/moon-logos.svg"));

    let near: Vec<ListedCoin> = [("A", 10_000_000.0), ("B", 10_100_000.0), ("C", 10_200_000.0), ("D", 10_300_000.0)]
        .into_iter()
        .map(|(symbol, cap)| coin(symbol, cap))
        .collect();
    let near_input = MoonChartInput {
        ladder: near.iter().map(|entry| rung(&token, entry)).collect(),
        target: rung(&token, &near[0]),
        ..input()
    };
    assert_eq!(render_moon_svg(&near_input), include_str!("testdata/moon-near.svg"));
}

#[test]
fn says_what_it_is_in_the_headline_and_the_fine_print() {
    let svg = render_moon_svg(&input());
    for expected in [
        "$TINY WITH THE MARKET CAP OF $DRB",
        ">VS<",
        ">$DRB<",
        "$0.0185",
        ">37x<",
        "#9 of 12 AI meme coins",
        "#5 of 12 AI meme coins",
        "Arithmetic, not a forecast.",
        "AI meme coins (12 coins) · CoinGecko · 2026-09-14",
        "@TinyMascot",
    ] {
        assert!(svg.contains(expected), "missing {expected}");
    }
}

#[test]
fn lights_the_target_rung_and_draws_the_others() {
    let input = input();
    let svg = render_moon_svg(&input);
    assert_eq!(svg.matches("fill=\"#7c6cff33\"").count(), 1);
    for entry in &input.ladder {
        assert!(svg.contains(&format!(">${}<", entry.coin.symbol)));
    }
}

#[test]
fn embeds_both_logos_as_data_uris_and_falls_back_to_a_lettered_disc_without_one() {
    let with_logos = render_moon_svg(&MoonChartInput {
        token_logo: Some(Logo { data: PNG.to_vec(), mime_type: "image/png".into() }),
        target_logo: Some(Logo { data: PNG.to_vec(), mime_type: "image/jpeg".into() }),
        ..input()
    });
    // The fighter and the strip.
    assert_eq!(with_logos.matches("href=\"data:image/png;base64,").count(), 2);
    assert!(with_logos.contains("href=\"data:image/jpeg;base64,"));
    let without = render_moon_svg(&input());
    assert!(!without.contains("data:image"));
    assert!(without.contains(">D<"));
    assert!(without.contains(">T<"));
}

#[test]
fn keeps_rung_labels_apart_along_the_strip_when_coins_are_almost_the_same_size() {
    let token = tiny();
    let near: Vec<ListedCoin> = [("A", 10_000_000.0), ("B", 10_100_000.0), ("C", 10_200_000.0), ("D", 10_300_000.0)]
        .into_iter()
        .map(|(symbol, cap)| coin(symbol, cap))
        .collect();
    let svg = render_moon_svg(&MoonChartInput {
        ladder: near.iter().map(|entry| rung(&token, entry)).collect(),
        target: rung(&token, &near[0]),
        ..input()
    });
    let mut xs: Vec<f64> = svg
        .lines()
        .filter(|line| line.starts_with("<circle cx=\"") && (line.contains("cy=\"562\" r=\"5\"") || line.contains("cy=\"562\" r=\"8\"")))
        .map(|line| line["<circle cx=\"".len()..].split('"').next().unwrap().parse().unwrap())
        .collect();
    xs.sort_by(f64::total_cmp);
    assert_eq!(xs.len(), 4);
    // Alternate lanes, so within a lane the gap is the minimum.
    assert!(xs[2] - xs[0] >= 95.9);
    assert!(xs[3] - xs[1] >= 95.9);
}

#[test]
fn escapes_a_coin_name_that_carries_markup() {
    let token = tiny();
    let nasty = ListedCoin { name: "<script>&\"".into(), ..coin("X", 5_000_000.0) };
    let svg = render_moon_svg(&MoonChartInput {
        target: rung(&token, &nasty),
        ladder: vec![rung(&token, &nasty)],
        ..input()
    });
    assert!(!svg.contains("<script>"));
    assert!(svg.contains("&lt;script&gt;&amp;&quot;"));
}

#[test]
fn rasterises_the_picture() {
    let png = crate::png::svg_to_png(&render_moon_svg(&MoonChartInput {
        token_logo: Some(Logo { data: PNG.to_vec(), mime_type: "image/png".into() }),
        ..input()
    }))
    .unwrap();
    let image = image::load_from_memory(&png).unwrap().to_rgba8();
    assert_eq!(image.dimensions(), (1_200, 675));
    // The moon itself, at the right end of the strip.
    assert_eq!(image.get_pixel(1_200 - 168 + 24 + 10, 562).0, [0xf5, 0xe6, 0xa8, 0xff]);
}
