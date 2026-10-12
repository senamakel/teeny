use teeny_core::config::ChartTimeframe;

use super::*;

pub(crate) fn candles(count: usize) -> Vec<Candle> {
    (0..count)
        .map(|index| {
            let i = index as f64;
            Candle {
                t: 1_700_000_000 + index as i64 * 3_600,
                o: 0.0007 + i * 0.000001,
                h: 0.00071 + i * 0.000001,
                l: 0.00069 + i * 0.000001,
                c: 0.000705 + i * 0.000001,
                v: 100.0,
            }
        })
        .collect()
}

fn tiny(candles: Vec<Candle>) -> ChartInput {
    ChartInput::new(candles, "$TINY", ChartTimeframe::H1)
}

#[test]
fn draws_byte_for_byte_what_the_legacy_renderer_drew() {
    let svg = render_chart_svg(&tiny(candles(30))).unwrap();
    assert_eq!(svg, include_str!("../testdata/chart-basic.svg"));
}

#[test]
fn draws_a_body_and_a_wick_for_every_candle() {
    let svg = render_chart_svg(&tiny(candles(30))).unwrap();
    assert!(svg.matches("<rect ").count() >= 60);
    assert!(svg.starts_with("<svg"));
    assert!(svg.trim_end().ends_with("</svg>"));
}

#[test]
fn refuses_to_draw_nothing() {
    let error = render_chart_svg(&tiny(Vec::new())).unwrap_err();
    assert!(error.to_string().contains("no candles"), "{error}");
}

#[test]
fn breaks_an_overlay_line_where_the_indicator_has_no_value() {
    let mut input = tiny(candles(10));
    input.overlays = vec![Overlay {
        label: "EMA5".into(),
        color: "#fff".into(),
        values: vec![
            None,
            None,
            Some(0.0007),
            Some(0.00071),
            None,
            Some(0.00072),
            Some(0.00073),
            Some(0.00074),
            Some(0.00075),
            Some(0.00076),
        ],
    }];
    let svg = render_chart_svg(&input).unwrap();
    let path = svg
        .split("<path d=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    // Two move commands: the gap starts a new segment rather than drawing
    // straight across it.
    assert_eq!(path.matches('M').count(), 2);
}

#[test]
fn puts_the_header_the_change_and_the_stats_where_a_reader_looks() {
    let mut input = tiny(candles(20));
    input.name = Some("Tiny Humans AI".into());
    input.change_pct = Some(17.4);
    input.stats = vec![Stat {
        label: "MC".into(),
        value: "$730.7K".into(),
    }];
    let svg = render_chart_svg(&input).unwrap();
    assert!(svg.contains("$TINY"));
    assert!(svg.contains("+17.40%"));
    assert!(svg.contains("MC $730.7K"));
}

#[test]
fn keeps_an_overlay_that_runs_past_the_candles_inside_the_panel() {
    let mut input = tiny(candles(20));
    // Far above every high: the scale has to grow to include it.
    input.overlays = vec![Overlay {
        label: "x".into(),
        color: "#fff".into(),
        values: vec![Some(0.002); 20],
    }];
    let svg = render_chart_svg(&input).unwrap();
    let ys: Vec<f64> = svg
        .split("<path d=\"M")
        .skip(1)
        .map(|rest| rest.split([' ', '"']).nth(1).unwrap().parse().unwrap())
        .collect();
    assert!(!ys.is_empty());
    assert!(ys.iter().all(|y| *y > 0.0));
}

#[test]
fn picks_round_steps_inside_the_range() {
    let ticks = price_ticks(0.0, 10.0, 5);
    assert_eq!(ticks[0], 0.0);
    assert!(*ticks.last().unwrap() <= 10.0);
    assert!(ticks.len() > 2);
}

#[test]
fn survives_a_flat_series_without_dividing_by_zero() {
    assert_eq!(price_ticks(5.0, 5.0, 6), vec![5.0]);
}

#[test]
fn takes_the_95th_percentile_so_one_launch_bar_cannot_flatten_the_panel() {
    let mut volumes = vec![10.0; 99];
    volumes.push(100_000.0);
    assert_eq!(percentile(&volumes, 0.95), 10.0);
    assert_eq!(percentile(&[], 0.95), 0.0);
}
