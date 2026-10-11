//! `teeny memory import`: everything the TypeScript service remembered in
//! the legacy CortexDB scope, copied into the shared TinyMemory store.
//!
//! The legacy side is read the one way the shared store always answered
//! quickly: its own event listing (`/v1/events`, `view=local`), filtered by
//! label, a page at a time, newest first. No recall, no heavy view, no
//! person filter (see the CortexDB notes in `deploy/README.md`). Each
//! request has a deadline and is retried with backoff; the writes go at
//! most [`ImportOptions::in_flight`] at a time.
//!
//! Every event becomes the [`Experience`] it was captured from (its text or
//! JSON, labels, time, person and idempotency key) and is stored exactly as
//! a live capture is ([`TinyMemory::store_experience`]), so the store sees
//! one item whichever way it arrived and a rerun stores nothing twice.
//!
//! A cursor file makes a run resumable: after every page it records where
//! the listing goes on, and a finished run records the newest event it
//! started from, so the next run reads only what is newer.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::{StreamExt, stream};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::experience::{Content, Role, iso};
use crate::{CortexMemory, EventsPage, Experience, MemoryError, Result, TinyMemory};

/// The labels every capture the account made carries one of: its channel.
pub const CHANNEL_LABELS: [&str; 2] = ["x", "telegram"];

/// Where the legacy events are read from.
#[async_trait]
pub trait LegacyEvents: Send + Sync {
    /// One page of events carrying any one of `labels`, after `cursor`.
    async fn page(
        &self,
        labels: &[String],
        cursor: Option<&str>,
        limit: usize,
        deadline: Duration,
    ) -> Result<EventsPage>;
}

#[async_trait]
impl LegacyEvents for CortexMemory {
    async fn page(
        &self,
        labels: &[String],
        cursor: Option<&str>,
        limit: usize,
        deadline: Duration,
    ) -> Result<EventsPage> {
        self.events_page(labels, cursor, limit, deadline).await
    }
}

/// How an import runs.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportOptions {
    /// Read and count, store nothing, keep no cursor.
    pub dry_run: bool,
    /// Leave out events observed before this.
    pub since: Option<DateTime<Utc>>,
    /// The listing's label filter.
    pub labels: Vec<String>,
    /// Events asked for per page (a listing may repeat one).
    pub page_size: usize,
    /// Writes at once.
    pub in_flight: usize,
    /// Each legacy request's deadline.
    pub request_timeout: Duration,
    /// Tries per request or write before the run stops.
    pub attempts: u32,
    /// The wait before the first retry, doubled for each one after.
    pub backoff: Duration,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            dry_run: false,
            since: None,
            labels: CHANNEL_LABELS
                .iter()
                .map(|&label| label.to_owned())
                .collect(),
            page_size: 200,
            in_flight: 2,
            request_timeout: Duration::from_secs(30),
            attempts: 4,
            backoff: Duration::from_secs(1),
        }
    }
}

/// Where an import stands, kept between runs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportCursor {
    /// The layout root the events were written to.
    pub root: String,
    /// The legacy scope they were read from.
    pub scope: String,
    /// An unfinished run's next page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume: Option<String>,
    /// The newest event the unfinished run started from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_head: Option<String>,
    /// The newest event of the last finished run: the next run stops there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub done_head: Option<String>,
}

impl ImportCursor {
    /// The cursor in `path`; a fresh one when there is no file yet.
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(serde_json::from_str(&text)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(io_error(path, &error)),
        }
    }

    /// Writes the cursor to `path` whole: to a sibling first, then renamed.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent).map_err(|error| io_error(parent, &error))?;
        }
        let partial = path.with_extension("partial");
        let text = serde_json::to_string_pretty(self)?;
        std::fs::write(&partial, text).map_err(|error| io_error(&partial, &error))?;
        std::fs::rename(&partial, path).map_err(|error| io_error(path, &error))
    }
}

fn io_error(path: &Path, error: &std::io::Error) -> MemoryError {
    MemoryError::InvalidInput(format!("{}: {error}", path.display()))
}

/// The default cursor file for `root`, under the data directory.
pub fn cursor_path(data_dir: &Path, root: &str) -> PathBuf {
    let name: String = root
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    data_dir
        .join("state")
        .join(format!("memory-import-{name}.json"))
}

/// One kind of memory's tally.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct KindCount {
    /// Events read.
    pub read: usize,
    /// Stored now (on a dry run: would be).
    pub stored: usize,
    /// Already in the store.
    pub already: usize,
    /// Left out: no text, or older than `--since`.
    pub skipped: usize,
}

/// What a run did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ImportReport {
    pub dry_run: bool,
    /// Listing pages read.
    pub pages: usize,
    /// Whether the run reached the end of what was new.
    pub finished: bool,
    /// By kind: `mention`, `reply`, `message`, and each activity's own kind.
    pub kinds: BTreeMap<String, KindCount>,
}

impl ImportReport {
    /// Every kind's tally added up.
    pub fn total(&self) -> KindCount {
        self.kinds
            .values()
            .fold(KindCount::default(), |sum, count| KindCount {
                read: sum.read + count.read,
                stored: sum.stored + count.stored,
                already: sum.already + count.already,
                skipped: sum.skipped + count.skipped,
            })
    }

    /// The report as the command prints it.
    pub fn render(&self) -> String {
        let stored = if self.dry_run { "to store" } else { "stored" };
        let mut out = format!(
            "{:<20} {:>7} {:>9} {:>8} {:>8}\n",
            "kind", "read", stored, "already", "skipped"
        );
        let total = self.total();
        for (kind, count) in self.kinds.iter().chain([(&"total".to_owned(), &total)]) {
            out.push_str(&format!(
                "{kind:<20} {:>7} {:>9} {:>8} {:>8}\n",
                count.read, count.stored, count.already, count.skipped
            ));
        }
        out.push_str(&format!(
            "{} page(s){}{}\n",
            self.pages,
            if self.finished {
                ", finished"
            } else {
                ", unfinished"
            },
            if self.dry_run {
                ", dry run: nothing written"
            } else {
                ""
            }
        ));
        out
    }
}

/// One legacy event, as the import reads it.
#[derive(Debug, Clone, PartialEq)]
struct LegacyEvent {
    id: String,
    labels: Vec<String>,
    experience: Option<Experience>,
    observed: Option<DateTime<Utc>>,
}

impl LegacyEvent {
    /// The event, or `None` when it has no id to tell it by.
    fn parse(raw: &Value) -> Option<Self> {
        let id = raw["id"].as_str().filter(|id| !id.is_empty())?.to_owned();
        let context = &raw["context"];
        let labels: Vec<String> = context["labels"]
            .as_array()
            .map(|labels| {
                labels
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let observed = [
            &context["observed_at"],
            &context["recorded_at"],
            &raw["observed_at"],
            &raw["recorded_at"],
            &raw["created_at"],
        ]
        .into_iter()
        .filter_map(Value::as_str)
        .find_map(|at| DateTime::parse_from_rfc3339(at).ok())
        .map(|at| at.with_timezone(&Utc));
        let content = &raw["content"];
        let text = content["text"].as_str();
        let content = match content["kind"].as_str() {
            Some("json") => Some(Content::Json {
                data: content["data"].clone(),
            }),
            Some("message") | None if text.is_some() => Some(Content::Message {
                role: match content["role"].as_str() {
                    Some("assistant") => Role::Assistant,
                    _ => Role::User,
                },
                text: text.unwrap_or_default().to_owned(),
            }),
            _ => None,
        };
        let key = [&raw["idempotency_key"], &context["idempotency_key"]]
            .into_iter()
            .filter_map(Value::as_str)
            .find(|key| !key.is_empty())
            .map_or_else(|| format!("cortex:{id}"), str::to_owned);
        let actor = match &raw["observed_actor"] {
            Value::String(actor) => Some(actor.clone()),
            Value::Object(actor) => actor.get("id").and_then(Value::as_str).map(str::to_owned),
            _ => None,
        }
        .filter(|actor| !actor.is_empty());
        let experience = content.map(|content| Experience {
            content,
            labels: labels.clone(),
            observed_at: iso(observed.unwrap_or_else(Utc::now)),
            idempotency_key: key,
            observed_actor: actor,
        });
        Some(Self {
            id,
            labels,
            experience,
            observed,
        })
    }

    /// What kind of memory it is, from its labels: `mention`, `reply`,
    /// `message`, or an activity's own kind.
    fn kind(&self) -> String {
        let kind = match self.labels.get(1).map(String::as_str) {
            Some("activity") => self.labels.get(2),
            _ => self.labels.get(1),
        };
        kind.cloned().unwrap_or_else(|| "unlabelled".to_owned())
    }
}

/// Copies the legacy scope into the shared store.
pub struct Importer<'a> {
    legacy: &'a dyn LegacyEvents,
    memory: &'a TinyMemory,
    /// The layout root `memory` writes to, recorded in the cursor.
    root: String,
    /// The legacy scope, recorded in the cursor.
    scope: String,
    options: ImportOptions,
}

impl<'a> Importer<'a> {
    pub fn new(
        legacy: &'a dyn LegacyEvents,
        memory: &'a TinyMemory,
        root: &str,
        scope: &str,
        options: ImportOptions,
    ) -> Self {
        Self {
            legacy,
            memory,
            root: root.to_owned(),
            scope: scope.to_owned(),
            options,
        }
    }

    /// Runs the import, keeping its place in `cursor_file`. A run that fails
    /// part way has saved where it got to, so running it again goes on from
    /// there; what it already stored is stored once either way.
    pub async fn run(&self, cursor_file: Option<&Path>) -> Result<ImportReport> {
        let options = &self.options;
        let mut state = match cursor_file {
            Some(path) => ImportCursor::load(path)?,
            None => ImportCursor::default(),
        };
        for (what, kept, now) in [
            ("root", &state.root, &self.root),
            ("scope", &state.scope, &self.scope),
        ] {
            if !kept.is_empty() && kept != now {
                return Err(MemoryError::InvalidInput(format!(
                    "the cursor file is for {what} {kept:?}, not {now:?}; name another with --cursor"
                )));
            }
        }
        state.root.clone_from(&self.root);
        state.scope.clone_from(&self.scope);
        let save = |state: &ImportCursor| match cursor_file {
            Some(path) if !options.dry_run => state.save(path),
            _ => Ok(()),
        };

        let mut report = ImportReport {
            dry_run: options.dry_run,
            ..ImportReport::default()
        };
        let mut cursor = state.resume.clone();
        let mut run_head = if cursor.is_some() {
            state.run_head.clone()
        } else {
            None
        };
        let mut seen = HashSet::new();
        loop {
            let page = self.page(cursor.as_deref()).await?;
            if page.next.is_some() && page.next == cursor {
                return Err(MemoryError::InvalidInput(
                    "the legacy listing returned the cursor it was given; stopping rather than \
                     reading one page for ever"
                        .into(),
                ));
            }
            report.pages += 1;
            let mut reached_done = false;
            let mut batch = Vec::new();
            for raw in &page.items {
                let Some(event) = LegacyEvent::parse(raw) else {
                    report.kinds.entry("unreadable".into()).or_default().skipped += 1;
                    continue;
                };
                if !seen.insert(event.id.clone()) {
                    continue;
                }
                if run_head.is_none() {
                    run_head = Some(event.id.clone());
                }
                if state.done_head.as_deref() == Some(event.id.as_str()) {
                    reached_done = true;
                    break;
                }
                let kind = event.kind();
                let count = report.kinds.entry(kind.clone()).or_default();
                count.read += 1;
                let before = matches!(
                    (options.since, event.observed),
                    (Some(since), Some(at)) if at < since
                );
                match event.experience {
                    Some(experience) if !before => batch.push((kind, experience)),
                    _ => count.skipped += 1,
                }
            }

            let stored = self.store(batch).await;
            for (kind, outcome) in &stored.outcomes {
                let count = report.kinds.entry(kind.clone()).or_default();
                match outcome {
                    Some(true) => count.already += 1,
                    Some(false) => count.stored += 1,
                    None => count.skipped += 1,
                }
            }
            if let Some(error) = stored.error {
                // Nothing past this page's start is marked done: a rerun
                // reads the page again and stores the rest of it.
                state.resume.clone_from(&cursor);
                state.run_head.clone_from(&run_head);
                save(&state)?;
                return Err(error);
            }

            cursor = page.next;
            if reached_done || cursor.is_none() {
                break;
            }
            state.resume.clone_from(&cursor);
            state.run_head.clone_from(&run_head);
            save(&state)?;
        }
        if run_head.is_some() {
            state.done_head = run_head;
        }
        state.resume = None;
        state.run_head = None;
        save(&state)?;
        report.finished = true;
        Ok(report)
    }

    /// One listing page, retried with backoff while the store is slow, down
    /// or overloaded; a refusal is not retried.
    async fn page(&self, cursor: Option<&str>) -> Result<EventsPage> {
        let options = &self.options;
        retry(options, retryable, || {
            self.legacy.page(
                &options.labels,
                cursor,
                options.page_size,
                options.request_timeout,
            )
        })
        .await
    }

    /// Stores a page's events, a few at a time.
    async fn store(&self, batch: Vec<(String, Experience)>) -> Stored {
        if self.options.dry_run {
            return Stored {
                outcomes: batch
                    .into_iter()
                    .map(|(kind, experience)| (kind, crate::item_of(&experience).map(|_| false)))
                    .collect(),
                error: None,
            };
        }
        let results: Vec<(String, Result<Option<bool>>)> = stream::iter(batch)
            .map(|(kind, experience)| async move {
                let outcome = retry(
                    &self.options,
                    |_| true,
                    || self.memory.store_experience(&experience),
                )
                .await;
                (kind, outcome)
            })
            .buffer_unordered(self.options.in_flight.max(1))
            .collect()
            .await;
        let mut stored = Stored::default();
        for (kind, result) in results {
            match result {
                Ok(outcome) => stored.outcomes.push((kind, outcome)),
                Err(error) => {
                    stored.error.get_or_insert(error);
                }
            }
        }
        stored
    }
}

/// What a page's writes came to.
#[derive(Default)]
struct Stored {
    /// Per event: `None` nothing to store, else whether it was there.
    outcomes: Vec<(String, Option<bool>)>,
    /// The first write that failed every try.
    error: Option<MemoryError>,
}

/// Whether a failed legacy request is worth another try.
fn retryable(error: &MemoryError) -> bool {
    match error {
        MemoryError::Api { status, .. } => *status == 429 || *status >= 500,
        MemoryError::Transport(_) | MemoryError::Timeout { .. } | MemoryError::Store(_) => true,
        _ => false,
    }
}

/// `call` until it succeeds, `options.attempts` times at most, waiting
/// `options.backoff` doubled between tries, while `worth` says so.
async fn retry<T, F, Fut>(
    options: &ImportOptions,
    worth: impl Fn(&MemoryError) -> bool,
    call: F,
) -> Result<T>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let mut wait = options.backoff;
    let mut tries = 1;
    loop {
        match call().await {
            Ok(value) => return Ok(value),
            Err(error) if tries < options.attempts.max(1) && worth(&error) => {
                tracing::warn!(%error, tries, "[memory:import] retrying");
                tokio::time::sleep(wait).await;
                wait = wait.saturating_mul(2);
                tries += 1;
            }
            Err(error) => return Err(error),
        }
    }
}

#[cfg(test)]
#[path = "import_tests.rs"]
mod tests;
