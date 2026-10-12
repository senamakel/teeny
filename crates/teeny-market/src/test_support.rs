//! Helpers shared by the unit tests: the golden fixtures, and a JSON
//! comparison that does not care whether a whole number was written `100` or
//! `100.0`, since JavaScript cannot tell them apart and serde can.

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

/// A golden fixture from `fixtures/golden`, written by `generate.mts`.
pub fn golden<T: DeserializeOwned>(name: &str) -> T {
    let path = format!("{}/fixtures/golden/{name}.json", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{path}: {error}"));
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("{path}: {error}"))
}

/// Every number as an f64, so `100` and `100.0` compare equal.
pub fn normalise(value: Value) -> Value {
    match value {
        Value::Number(number) => number
            .as_f64()
            .and_then(serde_json::Number::from_f64)
            .map_or(Value::Number(number), Value::Number),
        Value::Array(items) => Value::Array(items.into_iter().map(normalise).collect()),
        Value::Object(map) => {
            Value::Object(map.into_iter().map(|(k, v)| (k, normalise(v))).collect())
        }
        other => other,
    }
}

/// Asserts that `actual` serialises to the same JSON the TypeScript produced.
#[track_caller]
pub fn assert_same_json(actual: &impl Serialize, expected: &Value, what: &str) {
    let actual = normalise(serde_json::to_value(actual).expect("serialisable"));
    let expected = normalise(expected.clone());
    if actual != expected {
        panic!(
            "{what} differs from the TypeScript\n  rust: {}\n    ts: {}",
            serde_json::to_string(&actual).unwrap_or_default(),
            serde_json::to_string(&expected).unwrap_or_default()
        );
    }
}

/// A recorded provider response from `fixtures/http`.
pub fn fixture(name: &str) -> Value {
    golden_at("http", name)
}

fn golden_at(dir: &str, name: &str) -> Value {
    let path = format!("{}/fixtures/{dir}/{name}.json", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{path}: {error}"));
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("{path}: {error}"))
}

/// A client that never waits, for tests that exercise a retry.
pub fn no_wait() -> crate::http::JsonClient {
    crate::http::JsonClient::new().with_sleep(std::sync::Arc::new(|_| Box::pin(async {})))
}
