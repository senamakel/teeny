//! Number formatting for a chart nobody can zoom into, and the JavaScript
//! number spellings the legacy SVG was written with.
//!
//! The SVG is a string the tests (and a diff) read, so a coordinate has to be
//! spelled exactly as the TypeScript service spelled it: `Number#toString`
//! for interpolated values and `Number#toFixed` where it rounded.

/// `Number#toString`: the shortest spelling that reads back as the same
/// double, with an exponent only below 1e-6 or from 1e21 up.
pub fn js_number(value: f64) -> String {
    if value.is_nan() {
        return "NaN".to_owned();
    }
    if value.is_infinite() {
        return if value > 0.0 { "Infinity" } else { "-Infinity" }.to_owned();
    }
    if value == 0.0 {
        return "0".to_owned();
    }
    let size = value.abs();
    if (1e-6..1e21).contains(&size) {
        return format!("{value}");
    }
    let spelled = format!("{value:e}");
    match spelled.split_once('e') {
        Some((mantissa, exponent)) if !exponent.starts_with('-') => format!("{mantissa}e+{exponent}"),
        _ => spelled,
    }
}

/// `Number#toFixed`: rounds the double's exact decimal value, and a tie goes
/// away from zero (Rust's own `{:.2}` sends it to the even digit).
pub fn to_fixed(value: f64, digits: usize) -> String {
    if !value.is_finite() || value.abs() >= 1e21 {
        return js_number(value);
    }
    let negative = value < 0.0;
    // 1100 places is past the last digit of any double's expansion, so this
    // is the exact value, not a rounding of it.
    let exact = format!("{:.1100}", value.abs());
    let (whole, fraction) = exact.split_once('.').unwrap_or((exact.as_str(), ""));
    let mut kept: Vec<u8> = whole
        .bytes()
        .chain(fraction.bytes().take(digits))
        .map(|byte| byte - b'0')
        .collect();
    let mut whole_len = whole.len();
    if fraction.as_bytes().get(digits).is_some_and(|&next| next >= b'5') {
        increment(&mut kept, &mut whole_len);
    }
    let mut out = String::with_capacity(kept.len() + 2);
    if negative {
        out.push('-');
    }
    for (index, digit) in kept.iter().enumerate() {
        if index == whole_len {
            out.push('.');
        }
        out.push(char::from(b'0' + digit));
    }
    out
}

/// `Number(value.toPrecision(digits))`: the value rounded to `digits`
/// significant figures, read back as a number.
pub fn to_precision_number(value: f64, digits: usize) -> f64 {
    if value == 0.0 || !value.is_finite() || digits == 0 {
        return value;
    }
    let exponent: i64 = format!("{:e}", value.abs())
        .split_once('e')
        .and_then(|(_, exponent)| exponent.parse().ok())
        .unwrap_or(0);
    let places = digits as i64 - 1 - exponent;
    let spelled = if places >= 0 {
        to_fixed(value, places as usize)
    } else {
        format!("{:.*e}", digits - 1, value)
    };
    spelled.parse().unwrap_or(value)
}

/// $730.7K, $1.24M, $12.3B.
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
    // A market cap of 730,709 reads as $730.7K, not $731K: on a chart posted
    // at this size the tenth of a percent is the part people compare day to
    // day. Plain dollars keep no decimals at all, so $999 is not $999.0.
    let places = if scaled >= 100.0 {
        usize::from(!suffix.is_empty())
    } else if scaled >= 10.0 {
        1
    } else {
        2
    };
    format!("{sign}${}{suffix}", to_fixed(scaled, places))
}

/// A price, with enough significant figures to be useful and no more.
///
/// Sub-cent prices get four significant digits, so 0.0007472834089 prints as
/// 0.0007473 rather than as 0.00 or as the whole float.
pub fn price(value: f64) -> String {
    if !value.is_finite() {
        return "—".to_owned();
    }
    let size = value.abs();
    if size == 0.0 {
        return "0".to_owned();
    }
    if size >= 1_000.0 {
        return grouped_two_places(value);
    }
    if size >= 1.0 {
        return trim_zeros(&to_fixed(value, 4));
    }
    let decimals = ((-size.log10()).ceil() + 3.0).clamp(4.0, 12.0) as usize;
    trim_zeros(&to_fixed(value, decimals))
}

/// `+17.40%`.
pub fn signed_percent(value: f64) -> String {
    format!("{}{}%", if value >= 0.0 { "+" } else { "" }, to_fixed(value, 2))
}

/// Axis labels. Intraday charts want the clock; daily charts want the date.
pub fn axis_time(unix_seconds: i64, timeframe: &str) -> String {
    let Some(date) = chrono::DateTime::from_timestamp(unix_seconds, 0) else {
        return String::new();
    };
    if timeframe == "1d" {
        date.format("%d %b").to_string()
    } else {
        date.format("%d %b %H:%M").to_string()
    }
}

/// SVG has five characters that cannot appear in text; a token name may
/// contain all of them.
pub fn escape_xml(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
    out
}

/// Adds one in the last place of a digit string, carrying; a carry out of
/// the front grows the whole part by a digit.
fn increment(digits: &mut Vec<u8>, whole_len: &mut usize) {
    for digit in digits.iter_mut().rev() {
        if *digit == 9 {
            *digit = 0;
        } else {
            *digit += 1;
            return;
        }
    }
    digits.insert(0, 1);
    *whole_len += 1;
}

/// `toFixed(...).replace(/0+$/, '').replace(/\.$/, '')`.
fn trim_zeros(spelled: &str) -> String {
    let trimmed = spelled.trim_end_matches('0');
    trimmed.strip_suffix('.').unwrap_or(trimmed).to_owned()
}

/// `toLocaleString('en-US', { maximumFractionDigits: 2 })`: ICU rounds the
/// shortest decimal spelling half away from zero, then groups by thousands.
fn grouped_two_places(value: f64) -> String {
    let spelled = format!("{}", value.abs());
    let (whole, fraction) = spelled.split_once('.').unwrap_or((spelled.as_str(), ""));
    let mut digits: Vec<u8> = whole
        .bytes()
        .chain(fraction.bytes().chain(std::iter::repeat(b'0')).take(2))
        .map(|byte| byte - b'0')
        .collect();
    let mut whole_len = whole.len();
    if fraction.as_bytes().get(2).is_some_and(|&next| next >= b'5') {
        increment(&mut digits, &mut whole_len);
    }
    let mut out = String::new();
    if value < 0.0 {
        out.push('-');
    }
    for (index, digit) in digits[..whole_len].iter().enumerate() {
        if index > 0 && (whole_len - index) % 3 == 0 {
            out.push(',');
        }
        out.push(char::from(b'0' + digit));
    }
    let cents: String = digits[whole_len..].iter().map(|digit| char::from(b'0' + digit)).collect();
    let cents = cents.trim_end_matches('0');
    if !cents.is_empty() {
        out.push('.');
        out.push_str(cents);
    }
    out
}

#[cfg(test)]
#[path = "format_tests.rs"]
mod tests;
