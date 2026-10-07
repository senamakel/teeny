//! "$TINY with the market cap of X": the arithmetic and the picture.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;

use crate::chart::js_len;
use crate::format::{compact_usd, escape_xml, js_number, price, to_fixed};

/// One row of a market-cap leaderboard: what the moon chart needs.
#[derive(Debug, Clone, PartialEq)]
pub struct ListedCoin {
    pub id: String,
    pub symbol: String,
    pub name: String,
    pub price_usd: f64,
    pub market_cap_usd: f64,
    pub circulating_supply: f64,
    /// CoinGecko's overall market cap rank.
    pub rank: Option<u32>,
    /// Position on the category board, 1-based, set when the coin came from one.
    pub board_rank: Option<u32>,
    pub change_24h_pct: Option<f64>,
}

/// What the token would be at another coin's market cap.
#[derive(Debug, Clone, PartialEq)]
pub struct Rung {
    pub coin: ListedCoin,
    /// The target's market cap over the token's own.
    pub multiple: f64,
    /// The token's price at that market cap, with its circulating supply as it is today.
    pub implied_price_usd: f64,
}

/// A logo to embed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Logo {
    pub data: Vec<u8>,
    pub mime_type: String,
}

/// Everything the picture shows.
#[derive(Debug, Clone, PartialEq)]
pub struct MoonChartInput {
    pub token: ListedCoin,
    pub target: Rung,
    pub ladder: Vec<Rung>,
    pub board_name: String,
    pub board_size: Option<u32>,
    pub date: String,
    pub token_logo: Option<Logo>,
    pub target_logo: Option<Logo>,
    pub watermark: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

/// The moon's palette, on top of the chart's, so the two posts look like one
/// account.
pub mod palette {
    pub use crate::chart::THEME as BASE;

    pub const ACCENT: &str = "#7c6cff";
    pub const ACCENT_SOFT: &str = "#7c6cff33";
    pub const MOON: &str = "#f5e6a8";
    pub const MOON_SHADE: &str = "#d9c98a";
    pub const CARD: &str = "#1b2030";
    pub const CARD_BORDER: &str = "#2a2e39";
}

use palette::{ACCENT, ACCENT_SOFT, BASE, CARD, CARD_BORDER, MOON, MOON_SHADE};

/// Rung labels closer than this along the strip are pushed apart so they do
/// not overprint.
const MIN_RUNG_GAP: f64 = 96.0;

/// The arithmetic behind "$TINY with the market cap of X".
///
/// Price at the target cap is the target's cap divided by the token's own
/// circulating supply, which is the same as today's price times the multiple.
/// It is a hypothetical about today's supply, not a forecast; the picture says
/// so and so does the caption.
pub fn rung(token: &ListedCoin, target: &ListedCoin) -> Rung {
    Rung {
        coin: target.clone(),
        multiple: target.market_cap_usd / token.market_cap_usd,
        implied_price_usd: target.market_cap_usd / token.circulating_supply,
    }
}

/// The rungs drawn beside the pick: the board spread across the climb from
/// the token's own cap to the biggest coin on the list, picked evenly in log
/// space, with the target always among them and duplicates folded. Ascending,
/// smallest first. Only coins at least `min_multiple` above the token qualify
/// (the legacy defaults were 7 rungs and a floor of 2).
pub fn ladder(
    token: &ListedCoin,
    board: &[ListedCoin],
    target: &ListedCoin,
    count: usize,
    min_multiple: f64,
) -> Vec<Rung> {
    // The same floor as the pick: a coin at 1.04x is a neighbour, and a rung
    // that reads "1.04x" next to the token makes the strip look like a typo.
    let mut above: Vec<&ListedCoin> = board
        .iter()
        .filter(|coin| coin.id != token.id && coin.market_cap_usd / token.market_cap_usd >= min_multiple)
        .collect();
    above.sort_by(|a, b| a.market_cap_usd.total_cmp(&b.market_cap_usd));
    let (Some(lowest), Some(highest)) = (above.first(), above.last()) else {
        return vec![rung(token, target)];
    };
    let mut chosen: Vec<&ListedCoin> = vec![target];
    let slots = count.saturating_sub(1).max(1);
    let low = lowest.market_cap_usd.log10();
    let high = highest.market_cap_usd.log10();
    for slot in 0..slots {
        if chosen.len() >= count {
            break;
        }
        let wanted = if slots == 1 { high } else { low + ((high - low) * slot as f64) / (slots - 1) as f64 };
        // The coin nearest the wanted cap that is not already on the ladder.
        let distance = |coin: &ListedCoin| (coin.market_cap_usd.log10() - wanted).abs();
        let nearest = above
            .iter()
            .filter(|coin| !chosen.iter().any(|picked| picked.id == coin.id))
            .min_by(|a, b| distance(a).total_cmp(&distance(b)));
        if let Some(nearest) = nearest {
            chosen.push(nearest);
        }
    }
    chosen.sort_by(|a, b| a.market_cap_usd.total_cmp(&b.market_cap_usd));
    chosen.into_iter().map(|coin| rung(token, coin)).collect()
}

/// 490.07x reads as 490x; 2.5x keeps its half.
pub fn multiple(value: f64) -> String {
    if !value.is_finite() {
        return "—".to_owned();
    }
    if value >= 100.0 {
        return format!("{}x", js_number((value + 0.5).floor()));
    }
    if value >= 10.0 {
        let spelled = to_fixed(value, 1);
        return format!("{}x", spelled.strip_suffix(".0").unwrap_or(&spelled));
    }
    let spelled = to_fixed(value, 2);
    let trimmed = spelled.trim_end_matches('0');
    format!("{}x", trimmed.strip_suffix('.').unwrap_or(trimmed))
}

/// "$TINY with the market cap of X", as a picture.
///
/// A versus card: the token on the left with its logo, the yardstick on the
/// right with its own, the multiple between them; under that the result, the
/// price the token would be at that cap; and along the bottom the climb, a
/// log-scale strip from the token's cap to the moon with a rung for each of
/// the coins between, today's yardstick lit. The fine print says exactly what
/// the arithmetic is and the date it was true. Logos are embedded as data
/// URIs; without one the coin gets a disc with its first letter.
pub fn render_moon_svg(input: &MoonChartInput) -> String {
    let n = js_number;
    let width = f64::from(input.width.unwrap_or(1_200));
    let height = f64::from(input.height.unwrap_or(675));
    let token = &input.token;
    let target = &input.target;
    let token_tag = cashtag(&token.symbol);
    let target_tag = cashtag(&target.coin.symbol);
    let mut parts: Vec<String> = vec![
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}" font-family="{font}">"#,
            w = n(width),
            h = n(height),
            font = BASE.font,
        ),
        "<defs>".to_owned(),
        format!(
            r##"<linearGradient id="sky" x1="0" y1="1" x2="0" y2="0"><stop offset="0" stop-color="{}"/><stop offset="1" stop-color="#191d33"/></linearGradient>"##,
            BASE.background,
        ),
        format!(
            r#"<linearGradient id="track" x1="0" y1="0" x2="1" y2="0"><stop offset="0" stop-color="{}"/><stop offset="1" stop-color="{ACCENT}"/></linearGradient>"#,
            BASE.fast,
        ),
        r#"<clipPath id="disc-fighter"><circle cx="0" cy="0" r="64"/></clipPath>"#.to_owned(),
        r#"<clipPath id="disc-token"><circle cx="0" cy="0" r="14"/></clipPath>"#.to_owned(),
        "</defs>".to_owned(),
        format!(r#"<rect width="{}" height="{}" fill="url(#sky)"/>"#, n(width), n(height)),
    ];

    // A few stars, fixed by arithmetic rather than random so the same input
    // draws the same picture.
    let star_band = (height * 0.6 + 0.5).floor();
    for index in 0..48_u32 {
        let index_f = f64::from(index);
        parts.push(format!(
            r##"<circle cx="{}" cy="{}" r="{}" fill="#ffffff" opacity="{}"/>"##,
            n((index_f * 293.0) % width),
            n((index_f * 173.0) % star_band),
            if index % 5 == 0 { "1.6" } else { "1" },
            if index % 3 == 0 { "0.35" } else { "0.18" },
        ));
    }

    // --- the header ---------------------------------------------------------
    parts.push(format!(
        r#"<text x="{}" y="60" fill="{}" font-size="20" letter-spacing="4" text-anchor="middle">{}</text>"#,
        n(width / 2.0),
        BASE.muted,
        escape_xml(&format!("{token_tag} WITH THE MARKET CAP OF {target_tag}")),
    ));

    // --- the two fighters ---------------------------------------------------
    let ring_y = 168.0;
    let left_x = 300.0;
    let right_x = width - 300.0;
    parts.push(fighter(left_x, ring_y, token, input.token_logo.as_ref(), BASE.fast, &position(token, input)));
    parts.push(fighter(
        right_x,
        ring_y,
        &target.coin,
        input.target_logo.as_ref(),
        ACCENT,
        &position(&target.coin, input),
    ));

    // VS between them, and the multiple under it: the one number the picture
    // is about, in the middle where the eye lands first.
    let mid_x = width / 2.0;
    parts.push(format!(
        r#"<circle cx="{}" cy="{}" r="36" fill="{CARD}" stroke="{CARD_BORDER}" stroke-width="2"/>"#,
        n(mid_x),
        n(ring_y),
    ));
    parts.push(format!(
        r#"<text x="{}" y="{}" fill="{}" font-size="26" font-weight="700" text-anchor="middle" letter-spacing="2">VS</text>"#,
        n(mid_x),
        n(ring_y + 9.0),
        BASE.text,
    ));
    parts.push(format!(
        r#"<text x="{}" y="{}" fill="{}" font-size="48" font-weight="700" text-anchor="middle">{}</text>"#,
        n(mid_x),
        n(ring_y + 82.0),
        BASE.up,
        escape_xml(&multiple(target.multiple)),
    ));
    parts.push(format!(
        r#"<text x="{}" y="{}" fill="{}" font-size="16" text-anchor="middle">{}</text>"#,
        n(mid_x),
        n(ring_y + 108.0),
        BASE.muted,
        escape_xml("the market cap gap"),
    ));

    // --- the result ---------------------------------------------------------
    let panel_x = 120.0;
    let panel_w = width - panel_x * 2.0;
    let panel_y = 366.0;
    let implied = format!("${}", price(target.implied_price_usd));
    parts.push(format!(
        r#"<rect x="{}" y="{}" width="{}" height="104" rx="16" fill="{CARD}" stroke="{CARD_BORDER}" stroke-width="1"/>"#,
        n(panel_x),
        n(panel_y),
        n(panel_w),
    ));
    parts.push(format!(
        r#"<text x="{}" y="{}" fill="{}" font-size="15" letter-spacing="2">{}</text>"#,
        n(panel_x + 28.0),
        n(panel_y + 32.0),
        BASE.muted,
        escape_xml(&format!("{token_tag} AT {target_tag}'S MARKET CAP")),
    ));
    parts.push(format!(
        r#"<text x="{}" y="{}" fill="{}" font-size="{}" font-weight="700">{}</text>"#,
        n(panel_x + 28.0),
        n(panel_y + 86.0),
        BASE.text,
        if js_len(&implied) > 11.0 { 48 } else { 56 },
        escape_xml(&implied),
    ));
    parts.push(format!(
        r#"<text x="{}" y="{}" fill="{}" font-size="15" letter-spacing="2" text-anchor="end">TODAY</text>"#,
        n(panel_x + panel_w - 28.0),
        n(panel_y + 32.0),
        BASE.muted,
    ));
    parts.push(format!(
        r#"<text x="{}" y="{}" fill="{}" font-size="40" font-weight="700" text-anchor="end">{}</text>"#,
        n(panel_x + panel_w - 28.0),
        n(panel_y + 86.0),
        BASE.muted,
        escape_xml(&format!("${}", price(token.price_usd))),
    ));

    // --- the climb, along the bottom ----------------------------------------
    let track_y = 562.0;
    let track_left = 168.0;
    let track_right = width - 168.0;
    let rungs: Vec<&Rung> =
        input.ladder.iter().filter(|entry| entry.coin.market_cap_usd > token.market_cap_usd).collect();
    let highest = rungs
        .iter()
        .map(|entry| entry.coin.market_cap_usd)
        .fold(target.coin.market_cap_usd, |max, cap| if cap.is_nan() || max.is_nan() { f64::NAN } else { max.max(cap) });
    let low_log = token.market_cap_usd.log10();
    let high_log = highest.log10();
    let x_of = |cap: f64| {
        if high_log == low_log {
            track_left
        } else {
            track_left + ((cap.log10() - low_log) / (high_log - low_log)) * (track_right - track_left - 48.0)
        }
    };

    parts.push(format!(
        r#"<rect x="{}" y="{}" width="{}" height="3" rx="1.5" fill="url(#track)"/>"#,
        n(track_left),
        n(track_y - 1.5),
        n(track_right - track_left),
    ));
    // The token at the left end, where it is today; the moon at the right.
    parts.push(disc(track_left, track_y, 14.0, token, input.token_logo.as_ref(), "disc-token"));
    parts.push(format!(
        r#"<text x="{}" y="{}" fill="{}" font-size="16" font-weight="700" text-anchor="end">{}</text>"#,
        n(track_left - 24.0),
        n(track_y + 6.0),
        BASE.text,
        escape_xml(&token_tag),
    ));
    parts.push(format!(
        r#"<text x="{}" y="{}" fill="{}" font-size="13" text-anchor="end">{}</text>"#,
        n(track_left - 24.0),
        n(track_y + 26.0),
        BASE.muted,
        escape_xml(&compact_usd(token.market_cap_usd)),
    ));
    parts.push(format!(r#"<circle cx="{}" cy="{}" r="18" fill="{MOON}"/>"#, n(track_right + 24.0), n(track_y)));
    parts.push(format!(
        r#"<circle cx="{}" cy="{}" r="4" fill="{MOON_SHADE}"/>"#,
        n(track_right + 18.0),
        n(track_y - 5.0),
    ));
    parts.push(format!(
        r#"<circle cx="{}" cy="{}" r="5" fill="{MOON_SHADE}"/>"#,
        n(track_right + 30.0),
        n(track_y + 5.0),
    ));

    // Rungs, in two lanes so labels can be twice as dense as the strip is
    // wide, and pushed apart within a lane: the strip is log-scaled and two
    // coins a few percent apart would otherwise land on the same pixel.
    let mut placed: Vec<(&Rung, f64)> = rungs.iter().map(|entry| (*entry, x_of(entry.coin.market_cap_usd))).collect();
    placed.sort_by(|a, b| a.1.total_cmp(&b.1));
    for lane in 0..2 {
        let mut floor = track_left + 24.0;
        for (_, x) in placed.iter_mut().skip(lane).step_by(2) {
            if *x < floor {
                *x = floor;
            }
            floor = *x + MIN_RUNG_GAP;
        }
    }
    for (index, (entry, x)) in placed.iter().enumerate() {
        let x = *x;
        let lit = entry.coin.id == target.coin.id;
        let tag = cashtag(&entry.coin.symbol);
        let above = index % 2 == 0;
        let tag_y = if above { track_y - 34.0 } else { track_y + 40.0 };
        let detail_y = if above { track_y - 16.0 } else { track_y + 58.0 };
        if lit {
            parts.push(format!(
                r#"<rect x="{}" y="{}" width="116" height="48" rx="10" fill="{ACCENT_SOFT}"/>"#,
                n(x - 58.0),
                n(if above { track_y - 52.0 } else { track_y + 22.0 }),
            ));
        }
        parts.push(format!(
            r#"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="{}" stroke-width="2"/>"#,
            n(x),
            n(track_y),
            n(x),
            n(if above { track_y - 10.0 } else { track_y + 10.0 }),
            if lit { ACCENT } else { BASE.muted },
        ));
        parts.push(format!(
            r#"<circle cx="{}" cy="{}" r="{}" fill="{}" stroke="{}" stroke-width="2"/>"#,
            n(x),
            n(track_y),
            if lit { 8 } else { 5 },
            if lit { ACCENT } else { BASE.background },
            if lit { ACCENT } else { BASE.muted },
        ));
        parts.push(format!(
            r#"<text x="{}" y="{}" fill="{}" font-size="{}" font-weight="700" text-anchor="middle">{}</text>"#,
            n(x),
            n(tag_y),
            if lit { BASE.text } else { BASE.muted },
            if lit { 16 } else { 14 },
            escape_xml(&tag),
        ));
        parts.push(format!(
            r#"<text x="{}" y="{}" fill="{}" font-size="12" text-anchor="middle">{}</text>"#,
            n(x),
            n(detail_y),
            if lit { BASE.text } else { BASE.muted },
            escape_xml(&format!("{} · {}", compact_usd(entry.coin.market_cap_usd), multiple(entry.multiple))),
        ));
    }

    // --- the fine print -----------------------------------------------------
    let board_size = input.board_size.map_or_else(String::new, |size| format!(" ({size} coins)"));
    parts.push(format!(
        r#"<text x="{}" y="{}" fill="{}" font-size="13">{}</text>"#,
        n(panel_x),
        n(height - 18.0),
        BASE.muted,
        escape_xml(&format!(
            "{token_tag} at its supply today, priced at {target_tag}'s market cap. Arithmetic, not a forecast. · {}{board_size} · CoinGecko · {}",
            input.board_name, input.date,
        )),
    ));
    if let Some(watermark) = input.watermark.as_deref().filter(|mark| !mark.is_empty()) {
        parts.push(format!(
            r#"<text x="{}" y="{}" fill="{}" font-size="13" font-weight="700" text-anchor="end">{}</text>"#,
            n(width - panel_x),
            n(height - 18.0),
            BASE.muted,
            escape_xml(watermark),
        ));
    }

    parts.push("</svg>".to_owned());
    parts.join("\n")
}

/// One side of the versus: the logo in a coloured ring, the ticker, the name
/// and place, the cap and price.
fn fighter(cx: f64, cy: f64, coin: &ListedCoin, logo: Option<&Logo>, ring: &str, place: &str) -> String {
    let n = js_number;
    let tag = cashtag(&coin.symbol);
    let change = coin.change_24h_pct.map_or_else(String::new, |change| {
        format!(
            r#" · <tspan fill="{}">{}</tspan>"#,
            if change >= 0.0 { BASE.up } else { BASE.down },
            escape_xml(&format!("{}{}% 24h", if change >= 0.0 { "+" } else { "" }, to_fixed(change, 1))),
        )
    });
    [
        format!(r#"<circle cx="{}" cy="{}" r="70" fill="none" stroke="{ring}" stroke-width="4"/>"#, n(cx), n(cy)),
        disc(cx, cy, 64.0, coin, logo, "disc-fighter"),
        format!(
            r#"<text x="{}" y="{}" fill="{}" font-size="40" font-weight="700" text-anchor="middle">{}</text>"#,
            n(cx),
            n(cy + 116.0),
            BASE.text,
            escape_xml(&tag),
        ),
        format!(
            r#"<text x="{}" y="{}" fill="{}" font-size="16" text-anchor="middle">{}</text>"#,
            n(cx),
            n(cy + 142.0),
            BASE.muted,
            escape_xml(&format!("{}{place}", coin.name)),
        ),
        format!(
            r#"<text x="{}" y="{}" fill="{}" font-size="18" text-anchor="middle">{}{change}</text>"#,
            n(cx),
            n(cy + 166.0),
            BASE.text,
            escape_xml(&format!("{} mcap", compact_usd(coin.market_cap_usd))),
        ),
    ]
    .join("\n")
}

/// "#11 of 360 AI meme coins", or CoinGecko's overall rank when the coin is
/// not from a board.
fn position(coin: &ListedCoin, input: &MoonChartInput) -> String {
    if let Some(board_rank) = coin.board_rank {
        let of = input.board_size.map_or_else(String::new, |size| format!(" of {size}"));
        return format!(" · #{board_rank}{of} {}", input.board_name);
    }
    if let Some(rank) = coin.rank {
        return format!(" · #{rank} on CoinGecko");
    }
    String::new()
}

fn cashtag(symbol: &str) -> String {
    if symbol.starts_with('$') { symbol.to_owned() } else { format!("${symbol}") }
}

/// A coin's logo in a circle, or a disc with its initial when there is none.
fn disc(cx: f64, cy: f64, r: f64, coin: &ListedCoin, logo: Option<&Logo>, clip: &str) -> String {
    let n = js_number;
    if let Some(logo) = logo {
        let href = format!("data:{};base64,{}", logo.mime_type, BASE64.encode(&logo.data));
        return format!(
            r#"<g transform="translate({} {})" clip-path="url(#{clip})"><circle cx="0" cy="0" r="{}" fill="{CARD}"/><image href="{href}" x="{}" y="{}" width="{}" height="{}" preserveAspectRatio="xMidYMid slice"/></g>"#,
            n(cx),
            n(cy),
            n(r),
            n(-r),
            n(-r),
            n(r * 2.0),
            n(r * 2.0),
        );
    }
    let bare = coin.symbol.strip_prefix('$').unwrap_or(&coin.symbol);
    let letter = bare.chars().next().map_or_else(|| "?".to_owned(), |first| first.to_uppercase().collect());
    format!(
        r#"<circle cx="{}" cy="{}" r="{}" fill="{CARD}" stroke="{CARD_BORDER}" stroke-width="2"/><text x="{}" y="{}" fill="{}" font-size="{}" font-weight="700" text-anchor="middle">{}</text>"#,
        n(cx),
        n(cy),
        n(r),
        n(cx),
        n(cy + r * 0.36),
        BASE.text,
        n(r),
        escape_xml(&letter),
    )
}

#[cfg(test)]
#[path = "moon_tests.rs"]
mod tests;
