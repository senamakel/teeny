//! One JSON GET, with a deadline and a retry for the failures worth waiting out.
//!
//! The chart agent runs unattended. A provider that accepts the connection and
//! never answers would hang the run forever, so every call carries a timeout.

use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use serde_json::Value;

use crate::error::{MarketError, Result};

/// Several providers sit behind a CDN that refuses a request with no user agent.
pub const USER_AGENT: &str = "teeny-chart/1.0 (+https://tinyhumans.ai)";

/// How a backoff waits. Injected so a test does not have to.
pub type Sleeper = Arc<dyn Fn(Duration) -> BoxFuture<'static, ()> + Send + Sync>;

/// A JSON GET client: 15s deadline, two retries of a 429, a 5xx or a timeout.
#[derive(Clone)]
pub struct JsonClient {
    client: reqwest::Client,
    timeout: Duration,
    retries: u32,
    sleep: Sleeper,
}

impl std::fmt::Debug for JsonClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JsonClient")
            .field("timeout", &self.timeout)
            .field("retries", &self.retries)
            .finish_non_exhaustive()
    }
}

impl Default for JsonClient {
    fn default() -> Self {
        Self::new()
    }
}

impl JsonClient {
    /// The defaults: 15 second deadline, two retries, real sleeps.
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
            timeout: Duration::from_secs(15),
            retries: 2,
            sleep: Arc::new(|duration| Box::pin(tokio::time::sleep(duration))),
        }
    }

    /// A different deadline per attempt.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// A different number of retries.
    pub fn with_retries(mut self, retries: u32) -> Self {
        self.retries = retries;
        self
    }

    /// A different way to wait between attempts.
    pub fn with_sleep(mut self, sleep: Sleeper) -> Self {
        self.sleep = sleep;
        self
    }

    /// The deadline each attempt gets.
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// The underlying HTTP client, for the few calls that are not a JSON GET.
    pub fn http(&self) -> &reqwest::Client {
        &self.client
    }

    /// GETs `url` and parses the body as JSON, retrying what is worth retrying.
    pub async fn get(&self, url: &str, headers: &[(String, String)]) -> Result<Value> {
        let mut attempt = 0;
        loop {
            let error = match self.once(url, headers).await {
                Ok(body) => return Ok(body),
                Err(error) => error,
            };
            // A rate limit and a bad gateway are worth waiting out; a 404 is
            // not, and neither is a malformed request. Retrying those just
            // spends the budget the retryable ones need.
            if !worth_retrying(&error) || attempt >= self.retries {
                return Err(error);
            }
            // When the server said how long to wait, wait that long: backing
            // off on our own schedule against an explicit Retry-After burns
            // another attempt on a request it already said it would refuse.
            let wait =
                retry_after(&error).unwrap_or(Duration::from_millis(500 * 2u64.pow(attempt)));
            (self.sleep)(wait).await;
            attempt += 1;
        }
    }

    async fn once(&self, url: &str, headers: &[(String, String)]) -> Result<Value> {
        let mut request = self
            .client
            .get(url)
            .timeout(self.timeout)
            .header(reqwest::header::ACCEPT, "application/json")
            .header(reqwest::header::USER_AGENT, USER_AGENT);
        for (name, value) in headers {
            request = request.header(name.as_str(), value.as_str());
        }
        let response = request
            .send()
            .await
            .map_err(|error| transport(url, &error))?;
        let status = response.status();
        if !status.is_success() {
            let retry_after = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);
            let body = response.text().await.unwrap_or_default();
            return Err(MarketError::Status {
                url: url.to_owned(),
                status: status.as_u16(),
                body: body.chars().take(300).collect(),
                retry_after,
            });
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|error| transport(url, &error))?;
        serde_json::from_slice(&bytes).map_err(|error| MarketError::Transport {
            url: url.to_owned(),
            message: format!("not JSON: {error}"),
        })
    }
}

fn transport(url: &str, error: &reqwest::Error) -> MarketError {
    if error.is_timeout() {
        MarketError::Timeout {
            url: url.to_owned(),
        }
    } else {
        MarketError::Transport {
            url: url.to_owned(),
            message: error.to_string(),
        }
    }
}

fn worth_retrying(error: &MarketError) -> bool {
    match error {
        MarketError::Status { status, .. } => matches!(status, 429 | 500 | 502 | 503 | 504),
        MarketError::Timeout { .. } => true,
        _ => false,
    }
}

/// `Retry-After` in whole seconds, capped at 30. A date, or anything else that
/// does not start with digits, is ignored in favour of the backoff.
fn retry_after(error: &MarketError) -> Option<Duration> {
    let MarketError::Status {
        retry_after: Some(value),
        ..
    } = error
    else {
        return None;
    };
    let digits: String = value
        .trim_start()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    let seconds: u64 = digits.parse().ok()?;
    Some(Duration::from_secs(seconds.min(30)))
}

/// A number from an API that may spell it as a string, or not send it at all:
/// the TypeScript's `num`.
pub fn num(value: Option<&Value>) -> Option<f64> {
    let parsed = match value? {
        Value::Number(number) => number.as_f64(),
        Value::String(text) if !text.trim().is_empty() => text.trim().parse::<f64>().ok(),
        _ => None,
    }?;
    parsed.is_finite().then_some(parsed)
}

/// `a ?? b`: the first of two fields that is present and not null.
pub fn either<'a>(value: &'a Value, first: &str, second: &str) -> Option<&'a Value> {
    match value.get(first) {
        Some(found) if !found.is_null() => Some(found),
        _ => value.get(second),
    }
}

/// A string field, when it is one.
pub fn text(value: Option<&Value>) -> Option<String> {
    value?.as_str().map(str::to_owned)
}

#[cfg(test)]
#[path = "http_tests.rs"]
mod tests;
