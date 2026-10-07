//! Reading typed values out of a set of environment variables, collecting
//! every problem rather than stopping at the first.
//!
//! Each reader mirrors one zod shape from `legacy/src/config.ts`; the doc on
//! each names it. A reader that refuses a value records an issue and returns
//! the default, so the rest of the schema is still read and one boot names
//! every bad value. The values themselves never go into an issue.

use std::collections::HashMap;

use chrono::NaiveDate;

use super::kinds::Choice;
use crate::{ConfigError, ConfigIssue, Secret};

pub(crate) struct Env {
    vars: HashMap<String, String>,
    issues: Vec<ConfigIssue>,
}

impl Env {
    pub(crate) fn new(vars: HashMap<String, String>) -> Self {
        Self {
            vars,
            issues: Vec::new(),
        }
    }

    /// Every issue recorded, or nothing if the environment was good.
    pub(crate) fn finish(self) -> Result<(), ConfigError> {
        if self.issues.is_empty() {
            Ok(())
        } else {
            Err(ConfigError::new(self.issues))
        }
    }

    fn get(&self, key: &str) -> Option<&str> {
        self.vars.get(key).map(String::as_str)
    }

    /// The value, with a blank read as absent.
    fn present(&self, key: &str) -> Option<&str> {
        self.get(key).filter(|value| !value.is_empty())
    }

    fn fail(&mut self, key: &str, message: impl Into<String>) {
        self.issues.push(ConfigIssue {
            key: key.to_owned(),
            message: message.into(),
        });
    }

    /// `z.string().min(1)`: must be set and not blank.
    pub(crate) fn required(&mut self, key: &str) -> String {
        match self.present(key) {
            Some(value) => value.to_owned(),
            None => {
                self.fail(key, "is required");
                String::new()
            }
        }
    }

    /// [`Env::required`], for a credential.
    pub(crate) fn required_secret(&mut self, key: &str) -> Secret {
        Secret::new(self.required(key))
    }

    /// `z.string().default(..)`: any value, blank included.
    pub(crate) fn text(&mut self, key: &str, default: &str) -> String {
        self.get(key).unwrap_or(default).to_owned()
    }

    /// `z.string().min(1).default(..)`: absent takes the default, blank is refused.
    pub(crate) fn name(&mut self, key: &str, default: &str) -> String {
        match self.get(key) {
            None => default.to_owned(),
            Some("") => {
                self.fail(key, "must not be blank");
                default.to_owned()
            }
            Some(value) => value.to_owned(),
        }
    }

    /// A value with a default that a blank also selects.
    pub(crate) fn text_or(&mut self, key: &str, default: &str) -> String {
        self.present(key).unwrap_or(default).to_owned()
    }

    /// `blankable(z.string().min(1)).default(..)`: absent takes the default,
    /// blank turns the thing off.
    pub(crate) fn blank_disables(&mut self, key: &str, default: &str) -> Option<String> {
        match self.get(key) {
            None => Some(default.to_owned()),
            Some("") => None,
            Some(value) => Some(value.to_owned()),
        }
    }

    /// An optional value; blank is absent.
    pub(crate) fn optional(&mut self, key: &str) -> Option<String> {
        self.optional_min(key, 1)
    }

    /// An optional value at least `min` characters long; blank is absent.
    pub(crate) fn optional_min(&mut self, key: &str, min: usize) -> Option<String> {
        let value = self.present(key)?.to_owned();
        if value.chars().count() < min {
            self.fail(key, format!("must be at least {min} characters"));
            return None;
        }
        Some(value)
    }

    /// [`Env::optional_min`], for a credential.
    pub(crate) fn optional_secret(&mut self, key: &str, min: usize) -> Option<Secret> {
        self.optional_min(key, min).map(Secret::new)
    }

    /// An optional credential that must satisfy `valid`; blank is absent.
    pub(crate) fn optional_secret_where(
        &mut self,
        key: &str,
        valid: fn(&str) -> bool,
        message: &str,
    ) -> Option<Secret> {
        let value = self.present(key)?.to_owned();
        if !valid(&value) {
            self.fail(key, message);
            return None;
        }
        Some(Secret::new(value))
    }

    /// `z.url().default(..)`.
    pub(crate) fn url(&mut self, key: &str, default: &str) -> String {
        match self.get(key) {
            None => default.to_owned(),
            Some(value) if is_url(value) => value.to_owned(),
            Some(_) => {
                self.fail(key, "must be a URL");
                default.to_owned()
            }
        }
    }

    /// An optional URL; blank is absent.
    pub(crate) fn optional_url(&mut self, key: &str) -> Option<String> {
        let value = self.present(key)?.to_owned();
        if !is_url(&value) {
            self.fail(key, "must be a URL");
            return None;
        }
        Some(value)
    }

    /// `z.enum(['true', 'false']).default(..)`: exactly `true` or `false`.
    pub(crate) fn switch(&mut self, key: &str, default: bool) -> bool {
        match self.get(key) {
            None => default,
            Some("true") => true,
            Some("false") => false,
            Some(_) => {
                self.fail(key, "must be true or false");
                default
            }
        }
    }

    /// The legacy `z.coerce.boolean().default(..)` flags, read as their
    /// documentation says rather than as `Boolean(string)` did: `false`, `0`,
    /// `no`, `off` and blank are off; `true`, `1`, `yes` and `on` are on.
    pub(crate) fn flag(&mut self, key: &str, default: bool) -> bool {
        let Some(value) = self.get(key) else {
            return default;
        };
        match value.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => true,
            "false" | "0" | "no" | "off" | "" => false,
            _ => {
                self.fail(key, "must be true or false");
                default
            }
        }
    }

    /// `z.coerce.number().min(..).max(..).default(..)`. Like `Number()`, a
    /// blank reads as zero.
    pub(crate) fn number(&mut self, key: &str, default: f64, min: f64, max: f64) -> f64 {
        let Some(raw) = self.get(key) else {
            return default;
        };
        let Some(value) = coerce_number(raw) else {
            self.fail(key, "must be a number");
            return default;
        };
        if !self.in_range(key, value, min, max) {
            return default;
        }
        value
    }

    /// `z.coerce.number().int().min(..).max(..).default(..)`.
    pub(crate) fn int(&mut self, key: &str, default: u32, min: u32, max: u32) -> u32 {
        let Some(raw) = self.get(key) else {
            return default;
        };
        let Some(value) = coerce_number(raw) else {
            self.fail(key, "must be a number");
            return default;
        };
        if value.fract() != 0.0 {
            self.fail(key, "must be a whole number");
            return default;
        }
        if !self.in_range(key, value, f64::from(min), f64::from(max)) {
            return default;
        }
        // In range of two u32 bounds and whole, so the cast is exact.
        value as u32
    }

    fn in_range(&mut self, key: &str, value: f64, min: f64, max: f64) -> bool {
        if value < min {
            self.fail(key, format!("must be at least {min}"));
            false
        } else if value > max {
            self.fail(key, format!("must be at most {max}"));
            false
        } else {
            true
        }
    }

    /// A comma-separated list, trimmed, with empty entries dropped. A blank
    /// value is an empty list, not the default.
    pub(crate) fn list(&mut self, key: &str, default: &str) -> Vec<String> {
        split_list(self.get(key).unwrap_or(default))
    }

    /// The raw comma-separated value, for lists with their own entry syntax.
    pub(crate) fn raw_list(&mut self, key: &str, default: &str) -> String {
        self.get(key).unwrap_or(default).to_owned()
    }

    /// A list of slot numbers: entries that are not whole and non-negative
    /// are dropped, as legacy did, and so are empty ones, which legacy read
    /// as zero.
    pub(crate) fn slots(&mut self, key: &str, default: &str) -> Vec<u32> {
        split_list(self.get(key).unwrap_or(default))
            .iter()
            .filter_map(|entry| coerce_number(entry))
            .filter(|n| n.fract() == 0.0 && *n >= 0.0 && *n <= f64::from(u32::MAX))
            .map(|n| n as u32)
            .collect()
    }

    /// A list whose every entry is one of a closed set, at least one entry.
    pub(crate) fn choices<T: Choice>(&mut self, key: &str, default: &str) -> Vec<T> {
        let entries = split_list(self.get(key).unwrap_or(default));
        let parsed: Option<Vec<T>> = entries.iter().map(|entry| entry.parse().ok()).collect();
        match parsed {
            Some(values) if !values.is_empty() => values,
            Some(_) => {
                self.fail(
                    key,
                    format!("must list at least one of: {}", T::SPELLINGS.join(", ")),
                );
                Vec::new()
            }
            None => {
                self.fail(key, format!("must list only: {}", T::SPELLINGS.join(", ")));
                Vec::new()
            }
        }
    }

    /// `z.enum([..]).default(..)`.
    pub(crate) fn choice<T: Choice>(&mut self, key: &str, default: T) -> T {
        let Some(value) = self.get(key) else {
            return default;
        };
        match value.parse() {
            Ok(choice) => choice,
            Err(_) => {
                self.fail(key, format!("must be one of: {}", T::SPELLINGS.join(", ")));
                default
            }
        }
    }

    /// A `YYYY-MM-DD` calendar date.
    pub(crate) fn date(&mut self, key: &str, default: NaiveDate) -> NaiveDate {
        let Some(value) = self.get(key) else {
            return default;
        };
        let shaped = value.len() == 10
            && value.char_indices().all(|(i, c)| match i {
                4 | 7 => c == '-',
                _ => c.is_ascii_digit(),
            });
        match NaiveDate::parse_from_str(value, "%Y-%m-%d") {
            Ok(date) if shaped => date,
            _ => {
                self.fail(key, "must be a date, YYYY-MM-DD");
                default
            }
        }
    }
}

fn is_url(value: &str) -> bool {
    url::Url::parse(value).is_ok()
}

/// `Number(value)` for the shapes an operator types: blank is zero,
/// surrounding whitespace is ignored, and anything not finite is refused.
fn coerce_number(raw: &str) -> Option<f64> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Some(0.0);
    }
    trimmed.parse::<f64>().ok().filter(|n| n.is_finite())
}

pub(crate) fn split_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(str::to_owned)
        .collect()
}
