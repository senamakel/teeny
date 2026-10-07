use super::*;

#[test]
fn keeps_four_significant_figures_on_a_sub_cent_price() {
    assert_eq!(price(0.0007472834089), "0.0007473");
    assert_eq!(price(0.0), "0");
}

#[test]
fn prices_above_a_dollar_and_above_a_thousand_read_like_the_legacy_chart() {
    assert_eq!(price(1.23456), "1.2346");
    assert_eq!(price(2.0), "2");
    assert_eq!(price(1234.5678), "1,234.57");
    assert_eq!(price(1_000_000.0), "1,000,000");
    assert_eq!(price(-1234.5), "-1,234.5");
    assert_eq!(price(f64::NAN), "—");
}

#[test]
fn compacts_money_the_way_a_chart_header_does() {
    assert_eq!(compact_usd(730_709.0), "$730.7K");
    assert_eq!(compact_usd(1_240_000.0), "$1.24M");
    assert_eq!(compact_usd(999.0), "$999");
    assert_eq!(compact_usd(93_428.0), "$93.4K");
    assert_eq!(compact_usd(-12_300_000_000.0), "-$12.3B");
}

#[test]
fn escapes_the_characters_a_token_name_can_carry_into_markup() {
    assert_eq!(escape_xml("<a & \"b\">"), "&lt;a &amp; &quot;b&quot;&gt;");
    assert_eq!(escape_xml("it's"), "it&apos;s");
}

#[test]
fn signs_a_percentage() {
    assert_eq!(signed_percent(17.4), "+17.40%");
    assert_eq!(signed_percent(-3.456), "-3.46%");
    assert_eq!(signed_percent(0.0), "+0.00%");
}

#[test]
fn labels_the_time_axis_by_timeframe() {
    // 2023-11-14 22:13:20 UTC.
    assert_eq!(axis_time(1_700_000_000, "1h"), "14 Nov 22:13");
    assert_eq!(axis_time(1_700_000_000, "1d"), "14 Nov");
    assert_eq!(axis_time(1_704_067_200, "4h"), "01 Jan 00:00");
}

#[test]
fn prints_numbers_the_way_javascript_does() {
    assert_eq!(js_number(1200.0), "1200");
    assert_eq!(js_number(0.1 + 0.2), "0.30000000000000004");
    assert_eq!(js_number(-0.0), "0");
    assert_eq!(js_number(123.456), "123.456");
    assert_eq!(js_number(-7.5), "-7.5");
    assert_eq!(js_number(1e21), "1e+21");
    assert_eq!(js_number(1.5e-7), "1.5e-7");
    assert_eq!(js_number(0.000001), "0.000001");
    assert_eq!(js_number(f64::NAN), "NaN");
}

#[test]
fn rounds_ties_away_from_zero_like_to_fixed() {
    assert_eq!(to_fixed(0.5, 0), "1");
    assert_eq!(to_fixed(2.5, 0), "3");
    assert_eq!(to_fixed(-2.5, 0), "-3");
    assert_eq!(to_fixed(0.125, 2), "0.13");
    // 1.005 is stored as 1.00499999...; toFixed sees that, not the literal.
    assert_eq!(to_fixed(1.005, 2), "1.00");
    assert_eq!(to_fixed(9.995, 2), "9.99");
    assert_eq!(to_fixed(99.96, 1), "100.0");
    assert_eq!(to_fixed(-0.001, 2), "-0.00");
    assert_eq!(to_fixed(-0.0, 2), "0.00");
    assert_eq!(to_fixed(12.0, 3), "12.000");
}

#[test]
fn rounds_to_significant_figures_like_to_precision() {
    assert_eq!(to_precision_number(0.30000000000000004, 12), 0.3);
    assert_eq!(to_precision_number(0.000700000000000001, 12), 0.0007);
    assert_eq!(
        to_precision_number(123_456_789_012_345.0, 12),
        123_456_789_012_000.0
    );
    assert_eq!(to_precision_number(0.0, 12), 0.0);
}
