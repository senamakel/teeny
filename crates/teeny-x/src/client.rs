//! The X v2 client: the transport, the two ways it authenticates, and the
//! reads. Writes, media and Articles are further `impl XApi` blocks in their
//! own modules.
//!
//! Reads go out on the app's bearer when there is one and on the account's
//! OAuth 1.0a pair otherwise. A few reads are the account's to make and
//! always sign as it: "who am I", a follow relationship, and the counters
//! (impressions) of its own posts.

use reqwest::{Method, StatusCode};
use serde::de::DeserializeOwned;
use teeny_core::Secret;
use teeny_core::config::XConfig;
use teeny_core::types::{PostId, UserId};
use tokio::sync::OnceCell;

use crate::error::{XError, XResult};
use crate::model::{
    Identity, MentionWindow, OwnPostMetrics, SearchOptions, SearchPage, Trend, TrendOptions,
    WindowStart, XPost, XUser,
};
use crate::oauth::{OAuth1, percent_encode};
use crate::wire::{Envelope, RawPost, RawTrend, RawUser, to_posts};

/// Where X lives.
pub const X_API_BASE: &str = "https://api.x.com";

/// The post fields every read asks for. The counters ride along on the same
/// request: a field, not a post, so nothing extra is billed.
const POST_FIELDS: &str = "created_at,conversation_id,text,attachments,public_metrics";
/// Photos carry `url`; videos and GIFs only a preview frame.
const MEDIA_FIELDS: &str = "media_key,type,url,preview_image_url,alt_text";
const USER_FIELDS: &str = "username";
/// What a lookup by handle asks for about the account.
const USER_LOOKUP_FIELDS: &str = "created_at,description,public_metrics,verified";
const EXPANSIONS: &str = "author_id,in_reply_to_user_id,attachments.media_keys";

/// How much of an error body is kept.
const ERROR_BODY_CHARS: usize = 300;

/// Which credentials a request carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Auth {
    /// The app bearer if configured, else the account.
    App,
    /// The account's OAuth 1.0a pair.
    User,
}

/// A typed client for the slice of X v2 Teeny uses.
#[derive(Debug)]
pub struct XApi {
    http: reqwest::Client,
    base: String,
    oauth: OAuth1,
    bearer: Option<Secret>,
    username: String,
    identity: OnceCell<Identity>,
}

impl XApi {
    /// A client acting as `username` through `oauth`, reading on `bearer`
    /// when given.
    pub fn new(username: &str, oauth: OAuth1, bearer: Option<Secret>) -> Self {
        Self {
            http: reqwest::Client::new(),
            base: X_API_BASE.to_owned(),
            oauth,
            bearer,
            username: username.trim_start_matches('@').to_owned(),
            identity: OnceCell::new(),
        }
    }

    /// The client the configuration describes.
    pub fn from_config(config: &XConfig) -> Self {
        Self::new(
            &config.bot_username,
            OAuth1::new(
                config.api_key.expose(),
                config.api_secret.clone(),
                config.access_token.expose(),
                config.access_token_secret.clone(),
            ),
            config.bearer_token.clone(),
        )
    }

    /// Points the client somewhere other than `https://api.x.com`.
    pub fn with_base_url(mut self, base: impl Into<String>) -> Self {
        self.base = base.into().trim_end_matches('/').to_owned();
        self
    }

    // --- transport ------------------------------------------------------

    /// Sends one request and reads its JSON. `path` starts with `/2/`.
    pub(crate) async fn call<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<&serde_json::Value>,
        auth: Auth,
    ) -> XResult<T> {
        let response = self.send(method, path, query, body, auth).await?;
        let status = response.status();
        let text = response.text().await?;
        if !status.is_success() {
            return Err(http_error(status, &text));
        }
        serde_json::from_str(&text).map_err(|error| XError::Decode(error.to_string()))
    }

    pub(crate) async fn send(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<&serde_json::Value>,
        auth: Auth,
    ) -> XResult<reqwest::Response> {
        let mut url = format!("{}{}", self.base, path);
        if !query.is_empty() {
            let pairs: Vec<String> = query
                .iter()
                .map(|(name, value)| format!("{}={}", percent_encode(name), percent_encode(value)))
                .collect();
            url.push('?');
            url.push_str(&pairs.join("&"));
        }
        let authorization = match (auth, &self.bearer) {
            (Auth::App, Some(bearer)) => format!("Bearer {}", bearer.expose()),
            _ => self.oauth.header(method.as_str(), &url, &[]),
        };
        let mut request = self
            .http
            .request(method, &url)
            .header(reqwest::header::AUTHORIZATION, authorization);
        if let Some(body) = body {
            request = request.json(body);
        }
        Ok(request.send().await?)
    }

    // --- reads ----------------------------------------------------------

    /// One post with its author, attachments and counters; `None` when X
    /// does not have it (deleted, protected, not yet visible).
    pub async fn get_post(&self, id: &PostId) -> XResult<Option<XPost>> {
        let response: Envelope<RawPost> = self
            .call(
                Method::GET,
                &format!("/2/tweets/{}", percent_encode(id.as_str())),
                &post_query(),
                None,
                Auth::App,
            )
            .await?;
        let includes = response.includes;
        Ok(response
            .data
            .and_then(|post| to_posts(vec![post], includes.as_ref()).into_iter().next()))
    }

    /// One page of a recent search (the last 7 days).
    pub async fn search_page(&self, query: &str, options: &SearchOptions) -> XResult<SearchPage> {
        let mut params = vec![
            ("query", query.to_owned()),
            // X's floor is 10, so asking for fewer still bills for 10.
            (
                "max_results",
                options.max_results.unwrap_or(10).clamp(10, 100).to_string(),
            ),
        ];
        if let Some(since_id) = &options.since_id {
            params.push(("since_id", since_id.to_string()));
        }
        if let Some(start_time) = &options.start_time {
            params.push(("start_time", start_time.clone()));
        }
        if let Some(end_time) = &options.end_time {
            params.push(("end_time", end_time.clone()));
        }
        if let Some(sort_order) = options.sort_order {
            params.push(("sort_order", sort_order.as_str().to_owned()));
        }
        if let Some(token) = &options.pagination_token {
            params.push(("pagination_token", token.clone()));
        }
        params.extend(post_query());

        let response: Envelope<Vec<RawPost>> = self
            .call(
                Method::GET,
                "/2/tweets/search/recent",
                &params,
                None,
                Auth::App,
            )
            .await?;
        let meta = response.meta.unwrap_or_default();
        Ok(SearchPage {
            posts: to_posts(
                response.data.unwrap_or_default(),
                response.includes.as_ref(),
            ),
            next_token: meta.next_token,
            newest_id: meta.newest_id,
        })
    }

    /// The first page of a recent search.
    pub async fn search_posts(&self, query: &str, options: &SearchOptions) -> XResult<Vec<XPost>> {
        Ok(self.search_page(query, options).await?.posts)
    }

    /// Replies under a post, found through its conversation id; 25 unless told.
    pub async fn get_conversation(
        &self,
        conversation_id: &PostId,
        max_results: Option<u32>,
    ) -> XResult<Vec<XPost>> {
        let options = SearchOptions {
            max_results: Some(max_results.unwrap_or(25)),
            ..SearchOptions::default()
        };
        self.search_posts(&format!("conversation_id:{conversation_id}"), &options)
            .await
    }

    /// Everything addressed to the account: posts that mention it and replies
    /// to its posts (`to:` catches those without the handle in the text),
    /// minus its own posts and reposts.
    ///
    /// Keyed on the live handle from the token, not the configured one: a
    /// renamed account would otherwise exclude nobody (or whoever took the old
    /// handle). This is the ingest poll, and it never goes through the cache.
    pub async fn find_mentions(&self, window: &MentionWindow) -> XResult<Vec<XPost>> {
        let username = self.identity().await?.username;
        let mut options = SearchOptions {
            max_results: window.max_results,
            ..SearchOptions::default()
        };
        match &window.start {
            WindowStart::Anything => {}
            WindowStart::SinceId(id) => options.since_id = Some(id.clone()),
            WindowStart::StartTime(time) => options.start_time = Some(time.clone()),
        }
        self.search_posts(
            &format!("(@{username} OR to:{username}) -from:{username} -is:retweet"),
            &options,
        )
        .await
    }

    /// The account the credentials act as, asked of `/2/users/me` once and
    /// kept. Signed as the account: the app bearer identifies the app.
    pub async fn identity(&self) -> XResult<Identity> {
        self.identity
            .get_or_try_init(|| async {
                let response: Envelope<RawUser> = self
                    .call(
                        Method::GET,
                        "/2/users/me",
                        &[("user.fields", "username,public_metrics".to_owned())],
                        None,
                        Auth::User,
                    )
                    .await?;
                let user = response.data.unwrap_or_default();
                let Some(id) = user.id else {
                    return Err(XError::Missing(format!(
                        "X did not identify the authenticated account: {}",
                        response.errors.unwrap_or_default()
                    )));
                };
                Ok(Identity {
                    id,
                    username: user.username.unwrap_or_else(|| self.username.clone()),
                    followers: user
                        .public_metrics
                        .and_then(|metrics| metrics.followers_count),
                })
            })
            .await
            .cloned()
    }

    /// The account behind `username` (a leading `@` is ignored); `None`
    /// when X has no such account (suspended, deactivated, never existed).
    pub async fn user_by_username(&self, username: &str) -> XResult<Option<XUser>> {
        let handle = username.trim().trim_start_matches('@');
        let response: Envelope<RawUser> = self
            .call(
                Method::GET,
                &format!("/2/users/by/username/{}", percent_encode(handle)),
                &[("user.fields", USER_LOOKUP_FIELDS.to_owned())],
                None,
                Auth::App,
            )
            .await?;
        let Some(user) = response.data else {
            return Ok(None);
        };
        let Some(id) = user.id else {
            return Ok(None);
        };
        let metrics = user.public_metrics;
        Ok(Some(XUser {
            id,
            username: user.username.unwrap_or_else(|| handle.to_owned()),
            name: user.name,
            description: user.description,
            created_at: user.created_at,
            verified: user.verified,
            followers: metrics.as_ref().and_then(|m| m.followers_count),
            following: metrics.as_ref().and_then(|m| m.following_count),
            posts: metrics.as_ref().and_then(|m| m.tweet_count),
        }))
    }

    /// The account's own numeric id.
    pub async fn user_id(&self) -> XResult<UserId> {
        Ok(self.identity().await?.id)
    }

    /// What X says is trending, worldwide unless told a place; 1..=50 trends,
    /// 20 unless told.
    pub async fn trends(&self, options: TrendOptions) -> XResult<Vec<Trend>> {
        let response: Envelope<Vec<RawTrend>> = self
            .call(
                Method::GET,
                &format!("/2/trends/by/woeid/{}", options.woeid.unwrap_or(1)),
                &[
                    (
                        "max_trends",
                        options.max.unwrap_or(20).clamp(1, 50).to_string(),
                    ),
                    ("trend.fields", "trend_name,tweet_count".to_owned()),
                ],
                None,
                Auth::App,
            )
            .await?;
        Ok(response
            .data
            .unwrap_or_default()
            .into_iter()
            .filter_map(|trend| {
                let name = trend.trend_name?.trim().to_owned();
                (!name.is_empty()).then_some(Trend {
                    name,
                    posts: trend.tweet_count,
                })
            })
            .collect())
    }

    /// Whether this person follows the account (`followed_by`; the account
    /// following them is not evidence of anything). Signed as the account:
    /// a relationship needs somebody to be the other half of it.
    ///
    /// `None` when it cannot be established (a deleted user, a rate limit, an
    /// outage), which a caller must not read as "no".
    pub async fn follows_account(&self, user_id: &UserId) -> Option<bool> {
        let response: XResult<Envelope<RawUser>> = self
            .call(
                Method::GET,
                &format!("/2/users/{}", percent_encode(user_id.as_str())),
                &[("user.fields", "connection_status".to_owned())],
                None,
                Auth::User,
            )
            .await;
        match response {
            Ok(envelope) => {
                let status = envelope.data?.connection_status?;
                Some(status.iter().any(|entry| entry == "followed_by"))
            }
            Err(error) => {
                tracing::debug!(%user_id, %error, "x: could not read the follow relationship");
                None
            }
        }
    }

    /// Counters for the account's own posts, a hundred ids per request,
    /// through the account's credentials (impressions are the author's).
    pub async fn post_metrics(&self, ids: &[PostId]) -> XResult<Vec<OwnPostMetrics>> {
        let mut metrics = Vec::new();
        for batch in ids.chunks(100) {
            let joined: Vec<&str> = batch.iter().map(PostId::as_str).collect();
            let response: Envelope<Vec<RawPost>> = self
                .call(
                    Method::GET,
                    "/2/tweets",
                    &[
                        ("ids", joined.join(",")),
                        ("tweet.fields", "public_metrics,created_at".to_owned()),
                    ],
                    None,
                    Auth::User,
                )
                .await?;
            for post in response.data.unwrap_or_default() {
                let (Some(id), Some(counts)) = (post.id, post.public_metrics) else {
                    continue;
                };
                metrics.push(OwnPostMetrics {
                    id,
                    impressions: counts.impression_count.unwrap_or(0),
                    likes: counts.like_count.unwrap_or(0),
                    replies: counts.reply_count.unwrap_or(0),
                    reposts: counts.repost_count.unwrap_or(0),
                    quotes: counts.quote_count.unwrap_or(0),
                });
            }
        }
        Ok(metrics)
    }
}

fn post_query() -> Vec<(&'static str, String)> {
    vec![
        ("tweet.fields", POST_FIELDS.to_owned()),
        ("user.fields", USER_FIELDS.to_owned()),
        ("media.fields", MEDIA_FIELDS.to_owned()),
        ("expansions", EXPANSIONS.to_owned()),
    ]
}

pub(crate) fn http_error(status: StatusCode, body: &str) -> XError {
    XError::Http {
        status: status.as_u16(),
        body: body.chars().take(ERROR_BODY_CHARS).collect(),
    }
}

#[cfg(test)]
#[path = "client_tests.rs"]
mod tests;
