//! Writes: posting, replying, deleting, liking. Always signed as the account,
//! never cached, and crate-private: the one public way to publish is
//! [`crate::XPublisher`], which runs the guard and honours `DRY_RUN` first.

use reqwest::Method;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use teeny_core::types::PostId;

use crate::client::{Auth, XApi};
use crate::error::{XError, XResult};
use crate::model::{CreateOptions, Created};
use crate::oauth::percent_encode;
use crate::wire::Envelope;

#[derive(Debug, Deserialize)]
struct CreatedPost {
    id: Option<PostId>,
}

#[derive(Debug, Deserialize)]
struct Deleted {
    deleted: Option<bool>,
}

impl XApi {
    /// Publishes a post, attached to whatever `options` names.
    pub(crate) async fn create_post(
        &self,
        text: &str,
        options: &CreateOptions,
    ) -> XResult<Created> {
        let mut body = Map::new();
        body.insert("text".into(), json!(text));
        if let Some(parent) = &options.reply_to {
            body.insert("reply".into(), json!({ "in_reply_to_tweet_id": parent }));
        }
        if let Some(quoted) = &options.quote {
            body.insert("quote_tweet_id".into(), json!(quoted));
        }
        if !options.media_ids.is_empty() {
            body.insert("media".into(), json!({ "media_ids": options.media_ids }));
        }
        let response: Envelope<CreatedPost> = self
            .call(
                Method::POST,
                "/2/tweets",
                &[],
                Some(&Value::Object(body)),
                Auth::User,
            )
            .await?;
        match response.data.and_then(|post| post.id) {
            Some(id) => Ok(Created { id }),
            None => Err(XError::Missing(format!(
                "X did not return a post id: {}",
                response.errors.unwrap_or_default()
            ))),
        }
    }

    /// Answers a post.
    pub(crate) async fn reply(&self, post_id: &PostId, text: &str) -> XResult<Created> {
        let options = CreateOptions {
            reply_to: Some(post_id.clone()),
            ..CreateOptions::default()
        };
        self.create_post(text, &options).await
    }

    /// Deletes one of the account's posts. `false` when X did not say it did.
    pub(crate) async fn delete_post(&self, post_id: &PostId) -> XResult<bool> {
        let response: Envelope<Deleted> = self
            .call(
                Method::DELETE,
                &format!("/2/tweets/{}", percent_encode(post_id.as_str())),
                &[],
                None,
                Auth::User,
            )
            .await?;
        Ok(response.data.and_then(|data| data.deleted).unwrap_or(false))
    }

    /// Likes a post as the account. Idempotent on X's side.
    pub(crate) async fn like_post(&self, post_id: &PostId) -> XResult<()> {
        let me = self.user_id().await?;
        let response: Envelope<Value> = self
            .call(
                Method::POST,
                &format!("/2/users/{}/likes", percent_encode(me.as_str())),
                &[],
                Some(&json!({ "tweet_id": post_id })),
                Auth::User,
            )
            .await?;
        match response.errors {
            Some(Value::Array(errors)) if !errors.is_empty() => Err(XError::Missing(format!(
                "X refused the like: {}",
                Value::Array(errors)
            ))),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
#[path = "writes_tests.rs"]
mod tests;
