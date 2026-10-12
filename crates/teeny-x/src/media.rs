//! Media upload: INIT, one APPEND per 4 MB chunk, FINALIZE, then STATUS
//! polls until X has processed it.
//!
//! Every type takes the chunked route, as the legacy client did: a still fits
//! in one APPEND, so for an image this is the simple upload with two extra
//! calls. An animated GIF or a video is transcoded after FINALIZE and only
//! attachable once processing reports `succeeded`; attaching it earlier fails
//! the post, not the upload.

use std::time::Duration;

use base64::Engine as _;
use reqwest::Method;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::client::{Auth, XApi};
use crate::error::{XError, XResult};
use crate::model::{Media, Uploaded};
use crate::oauth::percent_encode;
use crate::wire::Envelope;

/// X takes up to 5 MB per APPEND; this stays well inside it.
pub(crate) const UPLOAD_CHUNK_BYTES: usize = 4 * 1024 * 1024;
const UPLOAD_POLL_ATTEMPTS: usize = 20;
/// The longest single wait between STATUS polls, whatever X suggests.
const MAX_POLL_WAIT_SECS: u64 = 10;

#[derive(Debug, Deserialize)]
struct UploadData {
    id: Option<String>,
    processing_info: Option<ProcessingInfo>,
}

#[derive(Debug, Clone, Deserialize)]
struct ProcessingInfo {
    state: Option<String>,
    check_after_secs: Option<u64>,
}

impl XApi {
    /// Uploads one attachment and returns its id, ready for a post.
    pub async fn upload_media(&self, media: &Media) -> XResult<Uploaded> {
        let initialized: Envelope<UploadData> = self
            .call(
                Method::POST,
                "/2/media/upload/initialize",
                &[],
                Some(&json!({
                    "media_type": media.mime.mime(),
                    "media_category": media.mime.category(),
                    "total_bytes": media.data.len(),
                })),
                Auth::User,
            )
            .await?;
        let Some(media_id) = initialized.data.and_then(|data| data.id) else {
            return Err(XError::Missing(format!(
                "X did not return a media id: {}",
                initialized.errors.unwrap_or_default()
            )));
        };
        let encoded_id = percent_encode(&media_id);

        for (segment, chunk) in media.data.chunks(UPLOAD_CHUNK_BYTES).enumerate() {
            let _: Value = self
                .call(
                    Method::POST,
                    &format!("/2/media/upload/{encoded_id}/append"),
                    &[],
                    Some(&json!({
                        "media": base64::engine::general_purpose::STANDARD.encode(chunk),
                        "segment_index": segment,
                    })),
                    Auth::User,
                )
                .await?;
        }

        let finalized: Envelope<UploadData> = self
            .call(
                Method::POST,
                &format!("/2/media/upload/{encoded_id}/finalize"),
                &[],
                None,
                Auth::User,
            )
            .await?;
        self.await_processing(
            &media_id,
            finalized.data.and_then(|data| data.processing_info),
        )
        .await?;
        Ok(Uploaded { media_id })
    }

    async fn await_processing(&self, media_id: &str, info: Option<ProcessingInfo>) -> XResult<()> {
        let mut state = info.as_ref().and_then(|info| info.state.clone());
        let mut wait = info.and_then(|info| info.check_after_secs).unwrap_or(1);

        for _ in 0..UPLOAD_POLL_ATTEMPTS {
            match state.as_deref() {
                None | Some("succeeded") => return Ok(()),
                Some("failed") => return Err(XError::MediaFailed(media_id.to_owned())),
                Some(_) => {}
            }
            tokio::time::sleep(Duration::from_secs(wait.min(MAX_POLL_WAIT_SECS))).await;
            let status: Envelope<UploadData> = self
                .call(
                    Method::GET,
                    "/2/media/upload",
                    &[
                        ("media_id", media_id.to_owned()),
                        ("command", "STATUS".to_owned()),
                    ],
                    None,
                    Auth::User,
                )
                .await?;
            let next = status.data.and_then(|data| data.processing_info);
            state = next.as_ref().and_then(|info| info.state.clone());
            wait = next.and_then(|info| info.check_after_secs).unwrap_or(wait);
        }
        Err(XError::Missing(format!(
            "X is still processing media {media_id} after {UPLOAD_POLL_ATTEMPTS} checks"
        )))
    }
}

#[cfg(test)]
#[path = "media_tests.rs"]
mod tests;
