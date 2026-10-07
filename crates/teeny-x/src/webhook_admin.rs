//! Managing where X delivers the account's activity (`teeny webhook`): the
//! app's one webhook (`/2/webhooks`) and the Activity API subscriptions that
//! bind event types to it (`/2/activity/subscriptions`).
//!
//! The endpoints disagree about credentials one by one: most want the app
//! bearer, creating a subscription acts for the account and wants OAuth 1.0a.
//! So every call goes out on the bearer and is retried as the account only
//! when X's refusal says that is what it wanted (a 403 naming OAuth 1.0a, a
//! 400 `OauthAccessTokenRequired`). Guessing is worse than it sounds: user
//! context sent to an app-only endpoint comes back a bare 503.

use reqwest::{Method, StatusCode};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::client::{Auth, XApi, http_error};
use crate::error::{XError, XResult};
use crate::oauth::percent_encode;

/// The events the account is subscribed to: somebody writing a post about
/// it (mention, reply, quote), which the queue answers, and somebody
/// noticing it (repost, like, follow), which feeds the people record.
pub const EVENTS: [&str; 6] = [
    "post.mention.create",
    "post.reply.create",
    "post.quote.create",
    "post.repost.create",
    "like.create",
    "follow.follow",
];

/// One registered webhook.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Webhook {
    pub id: Option<String>,
    pub url: Option<String>,
    pub valid: Option<bool>,
    pub created_at: Option<String>,
}

/// Whose activity a subscription delivers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct SubscriptionFilter {
    pub user_id: Option<String>,
}

/// One event type bound to a webhook.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Subscription {
    pub subscription_id: Option<String>,
    pub event_type: Option<String>,
    pub webhook_id: Option<String>,
    pub filter: Option<SubscriptionFilter>,
    pub tag: Option<String>,
}

impl Subscription {
    /// The account it is for, when it names one.
    pub fn user_id(&self) -> Option<&str> {
        self.filter.as_ref().and_then(|filter| filter.user_id.as_deref())
    }

    /// `event_type for user`.
    pub fn describe(&self) -> String {
        format!(
            "{} for {}",
            self.event_type.as_deref().unwrap_or("?"),
            self.user_id().unwrap_or("?")
        )
    }
}

#[derive(Deserialize)]
struct Listing<T> {
    #[serde(default = "Vec::new")]
    data: Vec<T>,
}

impl XApi {
    /// The app's registered webhooks (X allows one).
    pub async fn list_webhooks(&self) -> XResult<Vec<Webhook>> {
        let value = self.admin(Method::GET, "/2/webhooks", None).await?;
        listing(value)
    }

    /// Registers `url`. X sends it the CRC challenge during this call, so a
    /// URL that is not serving is refused here. Returns the new id.
    pub async fn create_webhook(&self, url: &str) -> XResult<String> {
        let created = self
            .admin(Method::POST, "/2/webhooks", Some(json!({ "url": url })))
            .await?;
        created["data"]["id"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| {
                XError::Missing(format!("X accepted the webhook but returned no id: {created}"))
            })
    }

    pub async fn delete_webhook(&self, id: &str) -> XResult<()> {
        let path = format!("/2/webhooks/{}", percent_encode(id));
        self.admin(Method::DELETE, &path, None).await.map(drop)
    }

    /// Every Activity API subscription the app holds.
    pub async fn list_subscriptions(&self) -> XResult<Vec<Subscription>> {
        let value = self
            .admin(Method::GET, "/2/activity/subscriptions", None)
            .await?;
        listing(value)
    }

    /// Binds `event_type` for the account `user_id` to `webhook_id`.
    pub async fn create_subscription(
        &self,
        webhook_id: &str,
        event_type: &str,
        user_id: &str,
    ) -> XResult<()> {
        let body = json!({
            "event_type": event_type,
            "filter": { "user_id": user_id },
            "webhook_id": webhook_id,
            "tag": format!("teeny {event_type}"),
        });
        self.admin(Method::POST, "/2/activity/subscriptions", Some(body))
            .await
            .map(drop)
    }

    pub async fn delete_subscription(&self, subscription_id: &str) -> XResult<()> {
        let path = format!(
            "/2/activity/subscriptions/{}",
            percent_encode(subscription_id)
        );
        self.admin(Method::DELETE, &path, None).await.map(drop)
    }

    /// One management call: the bearer, then the account when X asks.
    async fn admin(&self, method: Method, path: &str, body: Option<Value>) -> XResult<Value> {
        if !self.has_bearer() {
            return Err(XError::Missing(
                "X_BEARER_TOKEN is required to manage the webhook; set it in .env".into(),
            ));
        }
        match self.attempt(method.clone(), path, body.as_ref(), Auth::App).await {
            Err(XError::Http { status, body: detail }) if wants_user_context(status, &detail) => {
                self.attempt(method, path, body.as_ref(), Auth::User).await
            }
            other => other,
        }
    }

    async fn attempt(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
        auth: Auth,
    ) -> XResult<Value> {
        let response = self.send(method, path, &[], body, auth).await?;
        let status = response.status();
        let text = response.text().await?;
        if !status.is_success() {
            return Err(http_error(status, &text));
        }
        if text.trim().is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&text).map_err(|error| XError::Decode(error.to_string()))
    }
}

/// Whether X's refusal names user context as what it wanted.
fn wants_user_context(status: u16, detail: &str) -> bool {
    let lower = detail.to_ascii_lowercase();
    (status == StatusCode::FORBIDDEN.as_u16() && lower.contains("oauth 1.0a"))
        || (status == StatusCode::BAD_REQUEST.as_u16()
            && (lower.contains("oauthaccesstokenrequired") || lower.contains("user access token")))
}

fn listing<T: serde::de::DeserializeOwned>(value: Value) -> XResult<Vec<T>> {
    if value.is_null() {
        return Ok(Vec::new());
    }
    serde_json::from_value::<Listing<T>>(value)
        .map(|listing| listing.data)
        .map_err(|error| XError::Decode(error.to_string()))
}

#[cfg(test)]
#[path = "webhook_admin_tests.rs"]
mod tests;
