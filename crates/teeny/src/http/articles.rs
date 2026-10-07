//! `/articles`, `/articles/{slug}` and `/articles/{slug}/media/{file}`: what
//! `web/src/lib/articles.ts` reads, and the pictures its route proxies.

use std::collections::HashMap;

use axum::Json;
use axum::extract::{Path, Query};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};
use teeny_store::iso_now;

use super::State;
use super::reply::{bounded, error, internal};

type Params = Query<HashMap<String, String>>;

/// What the operator sees and the site does not.
const OPERATOR_DETAIL: [&str; 6] = ["problems", "error", "critique", "proof", "judge", "contentState"];

fn everything(params: &Params) -> bool {
    matches!(params.get("all").map(String::as_str), Some("1" | "true"))
}

fn unconfigured() -> Response {
    error(StatusCode::SERVICE_UNAVAILABLE, "no article store is configured")
}

pub async fn list(state: State, params: Params) -> Response {
    let Some(articles) = &state.articles else {
        return unconfigured();
    };
    let limit = bounded(params.get("limit").map(String::as_str), 50, 1, 200);
    match articles.list(everything(&params), limit).await {
        Ok(list) => Json(json!({ "source": "live", "generatedAt": iso_now(), "articles": list }))
            .into_response(),
        Err(failure) => internal(failure),
    }
}

pub async fn one(state: State, Path(slug): Path<String>, params: Params) -> Response {
    let Some(articles) = &state.articles else {
        return unconfigured();
    };
    let record = match articles.get(&slug).await {
        Ok(Some(record)) => record,
        Ok(None) => return error(StatusCode::NOT_FOUND, "no such article"),
        Err(failure) => return internal(failure),
    };
    let mut body = match serde_json::to_value(&record) {
        Ok(body) => body,
        Err(failure) => return internal(failure),
    };
    if !everything(&params)
        && let Value::Object(fields) = &mut body
    {
        for private in OPERATOR_DETAIL {
            fields.remove(private);
        }
    }
    Json(body).into_response()
}

pub async fn media(state: State, Path((slug, file)): Path<(String, String)>) -> Response {
    let Some(articles) = &state.articles else {
        return unconfigured();
    };
    match articles.media(&slug, &file).await {
        Ok(Some(media)) => (
            [
                (header::CONTENT_TYPE, HeaderValue::from_static(media.mime_type)),
                (header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=86400")),
            ],
            media.data,
        )
            .into_response(),
        Ok(None) => error(StatusCode::NOT_FOUND, "no such file"),
        Err(failure) => internal(failure),
    }
}

#[cfg(test)]
#[path = "articles_tests.rs"]
mod tests;
