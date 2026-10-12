use serde::Deserialize;

use super::*;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Golden {
    value: f64,
    compact_usd: String,
    price: String,
    signed_percent: String,
    string: String,
    fixed0: String,
    fixed1: String,
    fixed2: String,
    precision4: String,
    locale: String,
    locale2: String,
    round: f64,
}

fn golden() -> Vec<Golden> {
    serde_json::from_str(include_str!("../fixtures/golden/format.json")).expect("format golden")
}

#[test]
fn prints_numbers_exactly_as_javascript_does() {
    for case in golden() {
        let v = case.value;
        assert_eq!(js_number(v), case.string, "String({v})");
        assert_eq!(to_fixed(v, 0), case.fixed0, "{v}.toFixed(0)");
        assert_eq!(to_fixed(v, 1), case.fixed1, "{v}.toFixed(1)");
        assert_eq!(to_fixed(v, 2), case.fixed2, "{v}.toFixed(2)");
        assert_eq!(to_precision(v, 4), case.precision4, "{v}.toPrecision(4)");
        assert_eq!(to_locale(v, 3), case.locale, "{v}.toLocaleString()");
        assert_eq!(to_locale(v, 2), case.locale2, "{v}.toLocaleString(max 2)");
        assert_eq!(js_round(v), case.round, "Math.round({v})");
    }
}

#[test]
fn formats_money_and_prices_as_the_chart_did() {
    for case in golden() {
        let v = case.value;
        assert_eq!(compact_usd(v), case.compact_usd, "compactUsd({v})");
        assert_eq!(price(v), case.price, "price({v})");
        assert_eq!(signed_percent(v), case.signed_percent, "signedPercent({v})");
    }
}

#[test]
fn rounds_halves_towards_positive_infinity_like_math_round() {
    assert_eq!(js_round(-2.5), -2.0);
    assert_eq!(js_round(2.5), 3.0);
    assert_eq!(js_round(-0.4), 0.0);
}

#[test]
fn prints_what_has_no_digits_the_way_javascript_does() {
    assert_eq!(js_number(f64::NAN), "NaN");
    assert_eq!(js_number(f64::INFINITY), "Infinity");
    assert_eq!(js_number(-0.0), "0");
    assert_eq!(price(f64::NAN), "—");
}
