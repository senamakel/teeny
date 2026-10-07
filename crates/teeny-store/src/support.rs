//! Small helpers every repository shares: timestamps in the TypeScript
//! service's spelling, generated ids, and numbers that read back whatever the
//! Node driver wrote them as.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};

use chrono::{DateTime, SecondsFormat, Utc};

/// A time as `Date.prototype.toISOString` spells it,
/// `2026-09-13T01:00:00.000Z`. Rows compare these as strings, so the
/// spelling has to match what the TypeScript service already wrote.
pub fn iso(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// Now, as [`iso`] spells it.
pub fn iso_now() -> String {
    iso(Utc::now())
}

/// `prefix-<ms base36>-<6 random base36>`, the shape the TypeScript service
/// gives a journal row (`r-`), a ledger row (`i-`) or a hold (`h-`).
pub(crate) fn new_id(prefix: &str) -> String {
    let millis = u64::try_from(Utc::now().timestamp_millis()).unwrap_or_default();
    let mut random = base36(random_u64());
    while random.len() < 6 {
        random.insert(0, '0');
    }
    format!(
        "{prefix}-{}-{}",
        base36(millis),
        &random[random.len() - 6..]
    )
}

/// A random number from the standard library's per-process hasher keys, so
/// ids need no extra dependency. Not for anything secret.
pub(crate) fn random_u64() -> u64 {
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u128(Utc::now().timestamp_nanos_opt().unwrap_or_default() as u128);
    hasher.finish()
}

pub(crate) fn base36(mut value: u64) -> String {
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if value == 0 {
        return "0".to_owned();
    }
    let mut out = Vec::new();
    while value > 0 {
        out.push(DIGITS[(value % 36) as usize]);
        value /= 36;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

/// Counters the Node driver wrote. It stores a JS number as an Int32 when it
/// fits and as a Double when it does not, so a token total past two billion
/// is a Double in the production database. These read either back.
pub(crate) mod lenient {
    use serde::{Deserialize, Deserializer};

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Number {
        Int(i64),
        Float(f64),
    }

    impl Number {
        fn get(self) -> i64 {
            match self {
                Self::Int(value) => value,
                Self::Float(value) => value as i64,
            }
        }
    }

    pub fn i64<'de, D: Deserializer<'de>>(deserializer: D) -> Result<i64, D::Error> {
        Number::deserialize(deserializer).map(Number::get)
    }

    pub fn opt_i64<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<i64>, D::Error> {
        Option::<Number>::deserialize(deserializer).map(|number| number.map(Number::get))
    }

    pub fn map<'de, D, K>(deserializer: D) -> Result<std::collections::BTreeMap<K, i64>, D::Error>
    where
        D: Deserializer<'de>,
        K: Deserialize<'de> + Ord,
    {
        let raw = std::collections::BTreeMap::<K, Number>::deserialize(deserializer)?;
        Ok(raw
            .into_iter()
            .map(|(key, value)| (key, value.get()))
            .collect())
    }
}

/// The first `max` characters, as `text.slice(0, max)` keeps them.
pub(crate) fn clip(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}
