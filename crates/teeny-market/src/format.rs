//! Numbers as JavaScript prints them.
//!
//! Every number the market tools hand the model went through `String(n)`,
//! `toFixed`, `toPrecision` or `toLocaleString` in the TypeScript, and the
//! prompt was tuned against that text. Rust's own formatting differs in small
//! ways that would still change the words: it rounds an exact tie to even
//! where JavaScript rounds it up (`0.125.toFixed(2)` is `0.13`), it never
//! switches to exponent notation for `String(1e-7)`, and it prints `-0`.
//! These helpers reproduce the JavaScript output digit for digit.
//!
//! The chart's own formatters (`compactUsd`, `price`, `signedPercent`, from
//! `legacy/src/chart/format.ts`) live here too, because the tool text uses them.

/// Enough fractional digits to print any finite double exactly (the smallest
/// subnormal has 1074), so rounding below works on the true value.
const EXACT: usize = 1100;

/// The exact decimal expansion of `|value|`: integer digits and fraction digits.
fn exact_digits(value: f64) -> (String, String) {
    let text = format!("{:.*}", EXACT, value.abs());
    let (int, frac) = text.split_once('.').unwrap_or((&text, ""));
    (int.to_owned(), frac.to_owned())
}

/// Adds one to a string of decimal digits, growing it on a carry out.
fn increment(digits: &str) -> String {
    let mut bytes = digits.as_bytes().to_vec();
    for byte in bytes.iter_mut().rev() {
        if *byte == b'9' {
            *byte = b'0';
        } else {
            *byte += 1;
            return String::from_utf8(bytes).unwrap_or_default();
        }
    }
    let mut grown = String::with_capacity(bytes.len() + 1);
    grown.push('1');
    grown.push_str(&String::from_utf8(bytes).unwrap_or_default());
    grown
}

/// The shortest round-tripping decimal of `|value|` (the digits `String(n)`
/// prints), laid out as integer and fraction digits.
fn shortest_digits(value: f64) -> (String, String) {
    let scientific = format!("{:e}", value.abs());
    let (mantissa, exponent) = scientific.split_once('e').unwrap_or((&scientific, "0"));
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let point = exponent.parse::<i64>().unwrap_or(0) + 1;
    if point <= 0 {
        (
            "0".into(),
            format!("{}{digits}", "0".repeat((-point) as usize)),
        )
    } else if point as usize >= digits.len() {
        (
            format!("{digits}{}", "0".repeat(point as usize - digits.len())),
            String::new(),
        )
    } else {
        (
            digits[..point as usize].to_owned(),
            digits[point as usize..].to_owned(),
        )
    }
}

/// Rounds a decimal half-up to `digits` fraction digits: integer and fraction.
fn round_parts(int: &str, frac: &str, digits: usize) -> (String, String) {
    let frac = format!("{frac:0<width$}", width = digits + 1);
    let kept = format!("{int}{}", &frac[..digits]);
    let rounded = if frac.as_bytes()[digits] >= b'5' {
        increment(&kept)
    } else {
        kept
    };
    let split = rounded.len() - digits;
    (rounded[..split].to_owned(), rounded[split..].to_owned())
}

/// `|value|` rounded half-up on its exact binary value, which is what
/// `toFixed` specifies ("if there are two such n, pick the larger"): so
/// `1.005.toFixed(2)` is `1.00`, because 1.005 is really 1.00499999...
fn fixed_parts(value: f64, digits: usize) -> (String, String) {
    let (int, frac) = exact_digits(value);
    round_parts(&int, &frac, digits)
}

/// `|value|` rounded half-up on its shortest decimal, which is what ICU does
/// behind `toLocaleString`: `1.005` formats as `1.01` there.
fn locale_parts(value: f64, digits: usize) -> (String, String) {
    let (int, frac) = shortest_digits(value);
    round_parts(&int, &frac, digits)
}

/// `String(value)`.
pub fn js_number(value: f64) -> String {
    if value.is_nan() {
        return "NaN".into();
    }
    if value.is_infinite() {
        return if value > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    if value == 0.0 {
        return "0".into();
    }
    // Rust's `{:e}` is the shortest round-tripping digit string, the same
    // digits JavaScript chooses; only the layout differs.
    let scientific = format!("{:e}", value.abs());
    let (mantissa, exponent) = scientific.split_once('e').unwrap_or((&scientific, "0"));
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let exponent: i64 = exponent.parse().unwrap_or(0);
    let k = digits.len() as i64;
    let n = exponent + 1;
    let sign = if value < 0.0 { "-" } else { "" };

    let body = if k <= n && n <= 21 {
        format!("{digits}{}", "0".repeat((n - k) as usize))
    } else if 0 < n && n <= 21 {
        format!("{}.{}", &digits[..n as usize], &digits[n as usize..])
    } else if -6 < n && n <= 0 {
        format!("0.{}{digits}", "0".repeat((-n) as usize))
    } else {
        let rest = if k > 1 {
            format!(".{}", &digits[1..])
        } else {
            String::new()
        };
        let e = n - 1;
        format!(
            "{}{rest}e{}{}",
            &digits[..1],
            if e >= 0 { "+" } else { "-" },
            e.abs()
        )
    };
    format!("{sign}{body}")
}

/// `value.toFixed(digits)`.
pub fn to_fixed(value: f64, digits: usize) -> String {
    if !value.is_finite() || value.abs() >= 1e21 {
        return js_number(value);
    }
    let (int, frac) = fixed_parts(value, digits);
    let sign = if value < 0.0 { "-" } else { "" };
    if digits == 0 {
        format!("{sign}{int}")
    } else {
        format!("{sign}{int}.{frac}")
    }
}

/// `value.toPrecision(precision)`.
pub fn to_precision(value: f64, precision: usize) -> String {
    if !value.is_finite() {
        return js_number(value);
    }
    let precision = precision.max(1);
    if value == 0.0 {
        return if precision == 1 {
            "0".into()
        } else {
            format!("0.{}", "0".repeat(precision - 1))
        };
    }

    let (int, frac) = exact_digits(value);
    let all = format!("{int}{frac}");
    let first = all.bytes().position(|b| b != b'0').unwrap_or(0);
    // The power of ten of the first significant digit.
    let mut exponent = int.len() as i64 - 1 - first as i64;
    let mut significant = all[first..first + precision].to_owned();
    if all.as_bytes()[first + precision] >= b'5' {
        significant = increment(&significant);
        if significant.len() > precision {
            significant.truncate(precision);
            exponent += 1;
        }
    }

    let sign = if value < 0.0 { "-" } else { "" };
    let p = precision as i64;
    let body = if exponent < -6 || exponent >= p {
        let rest = if precision > 1 {
            format!(".{}", &significant[1..])
        } else {
            String::new()
        };
        format!(
            "{}{rest}e{}{}",
            &significant[..1],
            if exponent >= 0 { "+" } else { "-" },
            exponent.abs()
        )
    } else if exponent >= 0 {
        let whole = (exponent + 1) as usize;
        if whole == precision {
            significant
        } else {
            format!("{}.{}", &significant[..whole], &significant[whole..])
        }
    } else {
        format!("0.{}{significant}", "0".repeat((-exponent - 1) as usize))
    };
    format!("{sign}{body}")
}

/// `value.toLocaleString('en-US', { maximumFractionDigits })`: grouped
/// thousands, at most that many decimals, trailing zeros dropped.
pub fn to_locale(value: f64, maximum_fraction_digits: usize) -> String {
    if value.is_nan() {
        return "NaN".into();
    }
    if value.is_infinite() {
        return if value > 0.0 { "∞" } else { "-∞" }.into();
    }
    let (int, frac) = locale_parts(value, maximum_fraction_digits);
    let frac = frac.trim_end_matches('0');
    let mut grouped = String::with_capacity(int.len() + int.len() / 3);
    for (index, digit) in int.chars().enumerate() {
        if index > 0 && (int.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    // Intl keeps the sign of a negative number that rounds to zero.
    let sign = if value.is_sign_negative() { "-" } else { "" };
    if frac.is_empty() {
        format!("{sign}{grouped}")
    } else {
        format!("{sign}{grouped}.{frac}")
    }
}

/// `Math.round(value)`: the nearest integer, a half going towards +∞.
pub fn js_round(value: f64) -> f64 {
    if !value.is_finite() {
        return value;
    }
    let floor = value.floor();
    if value - floor >= 0.5 {
        floor + 1.0
    } else {
        floor
    }
}

/// `Math.round(value * 10^places) / 10^places`, the TypeScript's `round`.
pub fn round_to(value: f64, places: i32) -> f64 {
    let factor = 10f64.powi(places);
    js_round(value * factor) / factor
}

/// `$730.7K`, `$1.24M`, `$12.3B`.
pub fn compact_usd(value: f64) -> String {
    let sign = if value < 0.0 { "-" } else { "" };
    let size = value.abs();
    let (divisor, suffix) = if size >= 1e9 {
        (1e9, "B")
    } else if size >= 1e6 {
        (1e6, "M")
    } else if size >= 1e3 {
        (1e3, "K")
    } else {
        (1.0, "")
    };
    let scaled = size / divisor;
    // A market cap of 730,709 reads as $730.7K, not $731K; plain dollars keep
    // no decimals at all, so $999 does not print as $999.0.
    let places = if scaled >= 100.0 {
        usize::from(!suffix.is_empty())
    } else if scaled >= 10.0 {
        1
    } else {
        2
    };
    format!("{sign}${}{suffix}", to_fixed(scaled, places))
}

/// A price with enough significant figures to be useful and no more:
/// sub-cent prices get four significant digits, so 0.0007472834089 prints as
/// 0.0007473.
pub fn price(value: f64) -> String {
    if !value.is_finite() {
        return "—".into();
    }
    let size = value.abs();
    if size == 0.0 {
        return "0".into();
    }
    if size >= 1_000.0 {
        return to_locale(value, 2);
    }
    let fixed = if size >= 1.0 {
        to_fixed(value, 4)
    } else {
        let decimals = ((-size.log10()).ceil() + 3.0).clamp(4.0, 12.0) as usize;
        to_fixed(value, decimals)
    };
    fixed.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// JavaScript's `\s`: what `trim()` strips and `/\s+/` matches. Not quite
/// Rust's `char::is_whitespace`, which adds U+0085 and drops U+FEFF.
pub fn is_js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202F}'
                | '\u{205F}'
                | '\u{3000}'
                | '\u{FEFF}'
    )
}

/// `text.trim()`.
pub fn js_trim(text: &str) -> &str {
    text.trim_matches(is_js_space)
}

/// `text.replace(/\s+/g, ' ')`: every run of whitespace becomes one space,
/// including at the ends, which are not trimmed.
pub fn collapse_whitespace(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_run = false;
    for c in text.chars() {
        if is_js_space(c) {
            if !in_run {
                out.push(' ');
            }
            in_run = true;
        } else {
            out.push(c);
            in_run = false;
        }
    }
    out
}

/// `new Date(millis).toISOString()`: always UTC, always milliseconds.
pub fn iso_from_millis(millis: f64) -> Option<String> {
    if !millis.is_finite() {
        return None;
    }
    let time = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(millis.trunc() as i64)?;
    Some(time.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string())
}

/// `new Date().toISOString()`.
pub fn iso_now() -> String {
    chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

/// `+17.38%`, `-1.25%`.
pub fn signed_percent(value: f64) -> String {
    format!(
        "{}{}%",
        if value >= 0.0 { "+" } else { "" },
        to_fixed(value, 2)
    )
}

#[cfg(test)]
#[path = "format_tests.rs"]
mod tests;
