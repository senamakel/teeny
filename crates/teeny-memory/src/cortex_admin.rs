//! The operator's CortexDB calls: list the scope's events, page through
//! them (`teeny memory import`), preview a forget, and perform it
//! (`teeny forget`).
//!
//! Operational notes written during setup are recalled like anything else,
//! and a persona repeats them in public voice, so they have to be removed
//! rather than only discouraged in the prompt. Every forget redacts the
//! events behind what it removes, and none is ever scope-wide: with a
//! selector, `confirm_all` would authorize forgetting the whole scope and
//! the selector would be ignored.

use serde_json::{Value, json};

use std::time::Duration;

use super::{CortexMemory, api_error, timed};
use crate::Result;

/// One page of a scope's events, newest first.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EventsPage {
    /// The events as CortexDB sent them; a listing can repeat one.
    pub items: Vec<Value>,
    /// The cursor of the next page; `None` at the end.
    pub next: Option<String>,
}

/// What a forget removes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForgetSelector {
    /// These events, by id (`teeny forget --find` lists them).
    MemoryIds(Vec<String>),
    /// Everything about a subject.
    AboutSubject(String),
}

impl ForgetSelector {
    fn to_json(&self) -> Value {
        match self {
            Self::MemoryIds(ids) => json!({ "memory_ids": ids }),
            Self::AboutSubject(subject) => json!({ "about_subject": subject }),
        }
    }
}

impl CortexMemory {
    /// The scope's newest `limit` events, as CortexDB returns them.
    pub async fn events(&self, limit: u32) -> Result<Vec<Value>> {
        let limit = limit.to_string();
        let response = self
            .http
            .get(format!("{}/v1/events", self.api_url))
            .query(&[
                ("scope", self.scope.as_str()),
                ("view", "local"),
                ("limit", limit.as_str()),
            ])
            .header("X-Cortex-Actor", &self.actor)
            .bearer_auth(self.bearer.expose())
            .timeout(super::TIMEOUT)
            .send()
            .await?;
        let status = response.status();
        let text = response.text().await?;
        if !status.is_success() {
            return Err(api_error(status.as_u16(), &text));
        }
        let page: Value = serde_json::from_str(&text)?;
        Ok(page["items"].as_array().cloned().unwrap_or_default())
    }

    /// One page of the scope's own events (`view=local`, never a heavy
    /// view) carrying any one of `labels`, after `cursor`, within
    /// `deadline`. The label filter is the one listing shape the shared
    /// store always answered quickly.
    pub async fn events_page(
        &self,
        labels: &[String],
        cursor: Option<&str>,
        limit: usize,
        deadline: Duration,
    ) -> Result<EventsPage> {
        let limit = limit.to_string();
        let joined = labels.join(",");
        let mut query = vec![
            ("scope", self.scope.as_str()),
            ("view", "local"),
            ("limit", limit.as_str()),
        ];
        if !labels.is_empty() {
            query.push(("labels", joined.as_str()));
        }
        if let Some(cursor) = cursor {
            query.push(("cursor", cursor));
        }
        let response = self
            .http
            .get(format!("{}/v1/events", self.api_url))
            .query(&query)
            .header("X-Cortex-Actor", &self.actor)
            .bearer_auth(self.bearer.expose())
            .timeout(deadline)
            .send()
            .await
            .map_err(|error| timed(error, deadline))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|error| timed(error, deadline))?;
        if !status.is_success() {
            return Err(api_error(status.as_u16(), &text));
        }
        let page: Value = serde_json::from_str(&text)?;
        let next = page["next_cursor"]
            .as_str()
            .filter(|next| !next.is_empty())
            // An explicit `has_more: false` ends the listing whatever the
            // cursor says; a store that does not send it pages by cursor.
            .filter(|_| page["has_more"].as_bool() != Some(false))
            .map(str::to_owned);
        Ok(EventsPage {
            items: page["items"].as_array().cloned().unwrap_or_default(),
            next,
        })
    }

    /// What a forget would remove, and the `preview_id` to perform it with.
    /// Nothing is deleted.
    pub async fn forget_preview(&self, selector: &ForgetSelector) -> Result<Value> {
        let body = self.forget_body(selector);
        Ok(serde_json::from_str(
            &self
                .post("/v1/forget/preview", &body, super::TIMEOUT)
                .await?,
        )?)
    }

    /// Forgets what `selector` names, for `reason`, from the preview that
    /// showed it when there was one.
    pub async fn forget(
        &self,
        selector: &ForgetSelector,
        reason: &str,
        preview_id: Option<&str>,
    ) -> Result<Value> {
        let mut body = self.forget_body(selector);
        body["reason"] = json!(reason);
        if let Some(preview_id) = preview_id {
            body["from_preview_id"] = json!(preview_id);
        }
        Ok(serde_json::from_str(
            &self.post("/v1/forget", &body, super::TIMEOUT).await?,
        )?)
    }

    fn forget_body(&self, selector: &ForgetSelector) -> Value {
        json!({
            "scope": self.scope,
            "selector": selector.to_json(),
            "cascade": "redact_events",
            "confirm_all": false,
        })
    }
}

#[cfg(test)]
#[path = "cortex_admin_tests.rs"]
mod tests;
