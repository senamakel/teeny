use teeny_core::config::ChartTimeframe;

use super::*;

fn wavy(count: usize) -> Vec<Candle> {
    (0..count)
        .map(|i| {
            let f = i as f64;
            let base = 0.0007 + (i % 7) as f64 * 0.00002 - (i % 3) as f64 * 0.000015 + f * 0.000001;
            let open = base;
            let close = if i % 9 == 0 { base } else { base + ((i % 4) as f64 - 1.5) * 0.00001 };
            Candle {
                t: 1_700_000_000 + i as i64 * 14_400,
                o: open,
                h: open.max(close) + 0.000012,
                l: open.min(close) - 0.00001,
                c: close,
                v: 1_000.0 + (i % 5) as f64 * 250.0 + if i == 40 { 500_000.0 } else { 0.0 },
            }
        })
        .collect()
}

fn daily(count: usize) -> Vec<Candle> {
    (0..count)
        .map(|i| {
            let open = 43_000.0 + (i % 4) as f64 * 650.5 - i as f64 * 120.25;
            let close = if i == 5 { open } else { open + ((i % 3) as f64 - 1.0) * 410.75 };
            Candle {
                t: 1_704_067_200 + i as i64 * 86_400,
                o: open,
                h: open.max(close) + 333.3,
                l: open.min(close) - 222.2,
                c: close,
                v: 5_000_000.0 + i as f64 * 125_000.0,
            }
        })
        .collect()
}

fn full_request() -> ChartRequest {
    let mut request = ChartRequest::new(wavy(80), ChartTimeframe::H4);
    request.overview = Some(ChartOverview {
        symbol: Some("TINY".into()),
        name: Some("Tiny <Humans> & \"AI\"".into()),
        price_usd: 0.000812,
        market_cap_usd: Some(812_345.0),
        fdv_usd: None,
        liquidity_usd: Some(93_428.0),
        volume_24h_usd: Some(1_240_000.0),
    });
    request.read = Some(ChartRead {
        fast_period: 9,
        slow_period: 21,
        change_pct: -4.2,
        support: Some(0.0006),
        resistance: Some(0.00095),
    });
    request.watermark = Some("@TinyMascot".into());
    request
}

#[test]
fn derives_the_overlays_and_the_header_as_the_legacy_pipeline_did() {
    let rendered = render_chart(&full_request()).unwrap();
    assert_eq!(rendered.svg, include_str!("../testdata/chart-full.svg"));
    assert_eq!((rendered.width, rendered.height), (1_200, 675));
    assert!(rendered.path.is_none());
}

#[test]
fn falls_back_to_fdv_the_computed_change_and_the_request_size() {
    let mut request = ChartRequest::new(daily(12), ChartTimeframe::D1);
    request.symbol = Some("BTC".into());
    request.overview = Some(ChartOverview {
        symbol: None,
        name: None,
        price_usd: 41_234.5,
        market_cap_usd: None,
        fdv_usd: Some(812_000_000_000.0),
        liquidity_usd: None,
        volume_24h_usd: None,
    });
    request.width = Some(800);
    request.height = Some(450);
    let rendered = render_chart(&request).unwrap();
    assert_eq!(rendered.svg, include_str!("../testdata/chart-daily.svg"));
    let image = image::load_from_memory(&rendered.png).unwrap();
    assert_eq!((image.width(), image.height()), (800, 450));
}

#[test]
fn rasterises_the_chart_with_something_on_it() {
    let rendered = render_chart(&full_request()).unwrap();
    let image = image::load_from_memory(&rendered.png).unwrap().to_rgba8();
    assert_eq!(image.dimensions(), (1_200, 675));
    assert_eq!(image.get_pixel(0, 0).0, [0x13, 0x17, 0x22, 0xff]);
    // Candles in both colours made it onto the canvas, fonts or no fonts.
    let up = image.pixels().filter(|pixel| pixel.0 == [0x26, 0xa6, 0x9a, 0xff]).count();
    let down = image.pixels().filter(|pixel| pixel.0 == [0xef, 0x53, 0x50, 0xff]).count();
    assert!(up > 500 && down > 500, "up {up}, down {down}");
}

#[test]
fn writes_the_png_where_asked_named_by_symbol_and_timeframe() {
    let dir = std::env::temp_dir().join(format!("teeny-media-chart-{}", std::process::id()));
    let mut request = full_request();
    request.symbol = Some("$TI-NY".into());
    request.output_dir = Some(dir.join("charts"));
    let rendered = render_chart(&request).unwrap();
    let path = rendered.path.clone().unwrap();
    let name = path.file_name().unwrap().to_str().unwrap().to_owned();
    assert!(name.starts_with("TINY-4h-") && name.ends_with(".png"), "{name}");
    assert_eq!(std::fs::read(&path).unwrap(), rendered.png);
    // The symbol from the request wins over the overview's, cashtag kept once.
    assert!(rendered.svg.contains(">$TI-NY<"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn refuses_a_chart_with_no_candles() {
    let error = render_chart(&ChartRequest::new(Vec::new(), ChartTimeframe::H1)).unwrap_err();
    assert_eq!(error.to_string(), "Cannot render a chart with no candles");
}

#[test]
fn seeds_the_ema_on_the_simple_average_of_the_first_period() {
    let values = [1.0, 2.0, 3.0, 4.0, 5.0];
    assert_eq!(ema(&values, 3), vec![None, None, Some(2.0), Some(3.0), Some(4.0)]);
    assert_eq!(ema(&values, 6), vec![None; 5]);
}
