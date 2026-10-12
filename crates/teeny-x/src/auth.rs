//! The three-legged OAuth 1.0a flow that mints the account's access pair
//! (`teeny auth-x`): a request token for the app alone, the account
//! authorizing it on x.com, and the PIN X shows traded for the pair every
//! write is signed with. The calls take their parameters in the query, as
//! X's own SDK sends them, so the signer signs them like any query.

use reqwest::Method;
use teeny_core::Secret;

use crate::client::X_API_BASE;
use crate::error::{XError, XResult};
use crate::oauth::{OAuth1, percent_encode};

/// Where the account authorizes the request token.
const AUTHORIZE: &str = "https://x.com/oauth/authorize";

/// The temporary pair the authorization is made for.
#[derive(Debug, Clone)]
pub struct RequestToken {
    pub token: String,
    pub secret: Secret,
}

/// The account's access pair: `X_ACCESS_TOKEN` and `X_ACCESS_TOKEN_SECRET`.
#[derive(Debug, Clone)]
pub struct AccessToken {
    pub token: String,
    pub secret: Secret,
    /// The account that authorized it, when X says.
    pub screen_name: Option<String>,
}

/// The app's consumer pair, able to run the flow.
#[derive(Debug, Clone)]
pub struct ThreeLegged {
    http: reqwest::Client,
    base: String,
    consumer_key: String,
    consumer_secret: Secret,
}

impl ThreeLegged {
    /// `X_API_KEY` and `X_API_SECRET`.
    pub fn new(consumer_key: &str, consumer_secret: Secret) -> Self {
        Self {
            http: reqwest::Client::new(),
            base: X_API_BASE.to_owned(),
            consumer_key: consumer_key.to_owned(),
            consumer_secret,
        }
    }

    /// Points the flow somewhere other than `https://api.x.com`.
    pub fn with_base_url(mut self, base: impl Into<String>) -> Self {
        self.base = base.into().trim_end_matches('/').to_owned();
        self
    }

    /// A request token; `callback` is `oob` for the PIN flow.
    pub async fn request_token(&self, callback: &str) -> XResult<RequestToken> {
        let signer = OAuth1::new(
            &self.consumer_key,
            self.consumer_secret.clone(),
            "",
            Secret::new(""),
        );
        let (token, secret, _) = self
            .exchange(&signer, "/oauth/request_token", "oauth_callback", callback)
            .await?;
        Ok(RequestToken { token, secret })
    }

    /// Trades the PIN (`verifier`) for the account's access pair.
    pub async fn access_token(
        &self,
        request: &RequestToken,
        verifier: &str,
    ) -> XResult<AccessToken> {
        let signer = OAuth1::new(
            &self.consumer_key,
            self.consumer_secret.clone(),
            &request.token,
            request.secret.clone(),
        );
        let (token, secret, screen_name) = self
            .exchange(
                &signer,
                "/oauth/access_token",
                "oauth_verifier",
                verifier.trim(),
            )
            .await?;
        Ok(AccessToken {
            token,
            secret,
            screen_name,
        })
    }

    /// One signed POST whose form-encoded answer carries a token pair.
    async fn exchange(
        &self,
        signer: &OAuth1,
        path: &str,
        name: &str,
        value: &str,
    ) -> XResult<(String, Secret, Option<String>)> {
        let url = format!(
            "{}{path}?{}={}",
            self.base,
            percent_encode(name),
            percent_encode(value)
        );
        let response = self
            .http
            .post(&url)
            .header(
                reqwest::header::AUTHORIZATION,
                signer.header(Method::POST.as_str(), &url, &[]),
            )
            .send()
            .await?;
        let status = response.status();
        let text = response.text().await?;
        if !status.is_success() {
            return Err(XError::Http {
                status: status.as_u16(),
                body: text.chars().take(300).collect(),
            });
        }
        let field = |key: &str| {
            url::form_urlencoded::parse(text.as_bytes())
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.into_owned())
                .filter(|value| !value.is_empty())
        };
        match (field("oauth_token"), field("oauth_token_secret")) {
            (Some(token), Some(secret)) => Ok((token, Secret::new(secret), field("screen_name"))),
            _ => Err(XError::Missing(format!(
                "X answered {path} without a token pair"
            ))),
        }
    }
}

/// Where the account goes to authorize `request`.
pub fn authorize_url(request: &RequestToken) -> String {
    format!("{AUTHORIZE}?oauth_token={}", percent_encode(&request.token))
}

#[cfg(test)]
#[path = "auth_tests.rs"]
mod tests;
