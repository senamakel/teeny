//! The account's memory on TinyMemory: the one store every Teeny agent
//! shares through OpenHuman, under one layout root.
//!
//! What the account captures (mentions, replies, chat, its own posts and
//! notes) is stored at the root, so every agent bound there reads it; each
//! agent's own turns OpenHuman logs at that agent's node below. A capture
//! keeps everything the CortexDB client stored: the labels as tags, the
//! idempotency key as the item's source id, the time as `observed_at`, and
//! the person as `observed_actor`. An identical capture is a replay, since
//! TinyMemory derives an item's identity from all of that but the time and
//! the person.
//!
//! [`ItemStore`] is the seam: the runtime's memory facade in production (in
//! `teeny-agent`), [`EngineStore`] over a bare engine for tests and tools.
//! [`TinyMemory`] is the [`Memory`] the rest of the account holds, with the
//! same deadlines and [`RecallBreaker`] the CortexDB client had.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tinymemory_api::{
    DocumentBody, FetchMode, FetchRequest, Hit, ItemKind, ListRequest, MemoryEngine, MemoryMeta,
    MetaFilter, Namespace, ObservedActor, Reach, Role as TurnRole, SourceKind, SourceRef,
    StoreItem, StoreReceipt, Turn, WriteOptions,
};

use crate::breaker::RecallBreaker;
use crate::cortex::{ABOUT_TOKENS, CHARS_PER_TOKEN, GENERAL_TOKENS, PACK_SEPARATOR};
use crate::experience::{Content, Role};
use crate::{
    CortexOptions, Experience, Memory, MemoryError, Person, RECALL_LABELS, Result, cap_pack,
    format_recall,
};

/// The TinyMemory contract, for the crates that implement [`ItemStore`].
pub use tinymemory_api as api;

/// The layout root the account's memory lives under unless configured
/// otherwise (`TEENY_MEMORY_ROOT`).
pub const DEFAULT_ROOT: &str = "team:teeny";
/// Memories about the person in a recall.
const ABOUT_ITEMS: usize = 8;
/// Memories about the topic in a recall.
const GENERAL_ITEMS: usize = 12;
/// What an activity document is filed as.
const JSON_MIME: &str = "application/json";

/// The store the account's memory lives in: TinyMemory items under one
/// layout root.
#[async_trait]
pub trait ItemStore: Send + Sync {
    /// Stores `item` at the root, returning once the store has durably
    /// accepted it. An identical item stored again is a replay.
    async fn store(&self, item: StoreItem) -> Result<StoreReceipt>;

    /// At most `limit` items ranked against `query`, among those carrying
    /// any one of `tags_any` (every item when empty).
    async fn search(&self, query: &str, tags_any: &[String], limit: usize) -> Result<Vec<Hit>>;

    /// The newest `limit` items carrying any one of `tags_any`.
    async fn latest(&self, tags_any: &[String], limit: usize) -> Result<Vec<Hit>>;
}

impl From<tinymemory_api::Error> for MemoryError {
    fn from(error: tinymemory_api::Error) -> Self {
        Self::Store(error.to_string())
    }
}

/// [`ItemStore`] over a bare engine, confined to one root's subtree: what
/// the runtime's facade does, without a runtime.
#[derive(Clone)]
pub struct EngineStore {
    engine: Arc<dyn MemoryEngine>,
    root: Namespace,
}

impl std::fmt::Debug for EngineStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineStore")
            .field("engine", &self.engine.descriptor().id)
            .field("root", &self.root.to_string())
            .finish()
    }
}

impl EngineStore {
    /// `engine`, under `root` (`team:teeny`). The store root itself is no
    /// root: it would be every tenant's.
    pub fn new(engine: Arc<dyn MemoryEngine>, root: &str) -> Result<Self> {
        let root: Namespace = root.trim().parse()?;
        if root.is_root() {
            return Err(MemoryError::InvalidInput(
                "memory needs a root below the store root, such as team:teeny".into(),
            ));
        }
        Ok(Self { engine, root })
    }

    fn filter(&self, tags_any: &[String]) -> MetaFilter {
        MetaFilter {
            reach: Some(Reach::subtree(self.root.clone())),
            tags_any: tags_any.to_vec(),
            ..MetaFilter::default()
        }
    }
}

#[async_trait]
impl ItemStore for EngineStore {
    async fn store(&self, mut item: StoreItem) -> Result<StoreReceipt> {
        item.meta_mut().namespace = self.root.clone();
        Ok(self
            .engine
            .store_with(item, WriteOptions::accepted())
            .await?)
    }

    async fn search(&self, query: &str, tags_any: &[String], limit: usize) -> Result<Vec<Hit>> {
        let mode = self
            .engine
            .descriptor()
            .fetch_modes
            .first()
            .copied()
            .unwrap_or(FetchMode::Hybrid);
        let page = self
            .engine
            .fetch(FetchRequest {
                query: query.to_owned(),
                mode,
                filter: self.filter(tags_any),
                limit,
                cursor: None,
                beliefs: 0,
                refers_to: None,
                max_scopes: None,
            })
            .await?;
        Ok(page.hits)
    }

    async fn latest(&self, tags_any: &[String], limit: usize) -> Result<Vec<Hit>> {
        let page = self
            .engine
            .list(ListRequest::new(self.filter(tags_any), limit))
            .await?;
        Ok(page.items)
    }
}

/// What a capture becomes in the store; `None` for a message with no text,
/// which TinyMemory refuses and nobody would recall.
///
/// A message is a one-turn conversation, an activity a JSON document. The turn carries no time of its own: the time is
/// `observed_at`, which is not part of an item's identity, so a reply
/// captured again a second later is the same item.
pub fn item_of(experience: &Experience) -> Option<StoreItem> {
    let handle = experience
        .labels
        .iter()
        .find_map(|label| label.strip_prefix('@'))
        .map(str::to_owned);
    let source = match &experience.content {
        Content::Message { .. } => SourceKind::Conversation,
        Content::Json { .. } => SourceKind::Agent,
    };
    let meta = MemoryMeta {
        source: SourceRef {
            kind: source,
            id: Some(experience.idempotency_key.clone()),
        },
        tags: experience.labels.clone(),
        observed_at: DateTime::parse_from_rfc3339(&experience.observed_at)
            .ok()
            .map(|at| at.with_timezone(&Utc)),
        observed_actor: experience.observed_actor.as_ref().map(|id| ObservedActor {
            id: id.clone(),
            name: handle,
        }),
        ..MemoryMeta::default()
    };
    match &experience.content {
        Content::Message { role, text } => {
            if text.trim().is_empty() {
                return None;
            }
            let role = match role {
                Role::User => TurnRole::User,
                Role::Assistant => TurnRole::Assistant,
            };
            Some(StoreItem::Conversation {
                turns: vec![Turn::new(role, text.clone())],
                meta,
            })
        }
        Content::Json { data } => Some(StoreItem::Document {
            // The kind is in the JSON, and in the tags.
            title: None,
            body: DocumentBody::Text(data.to_string()),
            mime: Some(JSON_MIME.to_owned()),
            meta,
        }),
    }
}

/// One recalled memory as a line of a pack: when, then what, a turn spoken
/// by its `@handle` or by `you`.
fn line(hit: &Hit) -> String {
    let text = match hit.kind {
        ItemKind::Conversation => {
            let handle = hit
                .meta
                .tags
                .iter()
                .find(|tag| tag.starts_with('@'))
                .map_or("them", String::as_str);
            hit.text
                .lines()
                .map(|turn| {
                    if let Some(said) = turn.strip_prefix("user: ") {
                        format!("{handle}: {said}")
                    } else if let Some(said) = turn.strip_prefix("assistant: ") {
                        format!("you: {said}")
                    } else {
                        turn.to_owned()
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
        ItemKind::Document | ItemKind::Learning => hit.text.clone(),
    };
    match hit.meta.observed_at {
        Some(at) => format!("[{}] {text}", at.format("%Y-%m-%d %H:%M")),
        None => text,
    }
}

/// Hits as one pack, newest-first order kept, cut at `tokens`.
fn pack(hits: &[Hit], tokens: usize) -> String {
    let block = hits
        .iter()
        .map(line)
        .collect::<Vec<_>>()
        .join(PACK_SEPARATOR);
    cap_pack(&block, tokens * CHARS_PER_TOKEN)
}

/// The account's memory on an [`ItemStore`].
#[derive(Clone)]
pub struct TinyMemory {
    store: Arc<dyn ItemStore>,
    options: CortexOptions,
    breaker: RecallBreaker,
}

impl std::fmt::Debug for TinyMemory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TinyMemory")
            .field("options", &self.options)
            .finish_non_exhaustive()
    }
}

impl TinyMemory {
    /// Memory on `store`, on [`CortexOptions::default`].
    pub fn new(store: Arc<dyn ItemStore>) -> Self {
        let options = CortexOptions::default();
        Self {
            store,
            breaker: RecallBreaker::new(options.cooldown, options.max_in_flight),
            options,
        }
    }

    /// The same memory on `options`, with a breaker of its own.
    pub fn with_options(mut self, options: CortexOptions) -> Self {
        self.breaker = RecallBreaker::new(options.cooldown, options.max_in_flight);
        self.options = options;
        self
    }

    /// Stores `experience` as [`item_of`] shapes it, within the capture
    /// deadline. `None` when there was nothing to store; otherwise whether
    /// the store already held it.
    pub async fn store_experience(&self, experience: &Experience) -> Result<Option<bool>> {
        let Some(item) = item_of(experience) else {
            tracing::debug!(key = %experience.idempotency_key, "[memory] nothing to store");
            return Ok(None);
        };
        let receipt = within(self.options.capture_timeout, self.store.store(item)).await?;
        Ok(Some(receipt.replayed))
    }

    /// One read within the recall deadline; a timeout trips the breaker,
    /// since the store may still be working on it.
    async fn read<F>(&self, read: F) -> Result<Vec<Hit>>
    where
        F: Future<Output = Result<Vec<Hit>>>,
    {
        let hits = within(self.options.recall_timeout, read).await;
        if hits.as_ref().is_err_and(MemoryError::is_timeout) {
            self.breaker.trip();
        }
        hits
    }

    async fn general(&self, query: &str) -> Result<String> {
        let tags: Vec<String> = RECALL_LABELS
            .iter()
            .map(|&label| label.to_owned())
            .collect();
        let hits = self
            .read(self.store.search(query, &tags, GENERAL_ITEMS))
            .await?;
        Ok(pack(&hits, GENERAL_TOKENS))
    }

    async fn about(&self, handle: &str) -> Result<String> {
        let hits = self
            .read(self.store.latest(&[handle.to_owned()], ABOUT_ITEMS))
            .await?;
        Ok(pack(&hits, ABOUT_TOKENS))
    }

    async fn recall_once(&self, query: &str, person: Option<&Person>) -> Result<String> {
        // As the CortexDB client found a person: by the `@handle` label every
        // exchange with them carries; a person known by id alone is recalled
        // about generally.
        let person = person.filter(|person| person.username.is_some());
        let handle = person.and_then(Person::handle_label);
        if let Some(handle) = &handle
            && !self.options.person_pass
        {
            let general = self.general(&format!("{handle} {query}")).await?;
            return Ok(format_recall(None, "", &general));
        }
        // Settled, not all-or-nothing: losing the person pass costs the turn
        // its history with them, not its general context too.
        let about = async {
            match &handle {
                Some(handle) => self.about(handle).await,
                None => Ok(String::new()),
            }
        };
        let (about, general) = futures::join!(about, self.general(query));
        let general = match (general, &about) {
            (Err(error), Err(_)) => return Err(error),
            (Err(error), Ok(_)) if handle.is_none() => return Err(error),
            (general, _) => general.unwrap_or_default(),
        };
        Ok(format_recall(person, &about.unwrap_or_default(), &general))
    }
}

/// `call`, or a timeout at `deadline`.
async fn within<T, F>(deadline: Duration, call: F) -> Result<T>
where
    F: Future<Output = Result<T>>,
{
    tokio::time::timeout(deadline, call)
        .await
        .unwrap_or(Err(MemoryError::Timeout {
            after_ms: deadline.as_millis(),
        }))
}

#[async_trait]
impl Memory for TinyMemory {
    async fn capture(&self, experience: Experience) -> Result<()> {
        self.store_experience(&experience).await.map(|_| ())
    }

    /// Behind the breaker: refused without a call while recall is paused or
    /// too many are in flight, and paused for the cooldown after a timeout.
    async fn recall(&self, query: &str, person: Option<&Person>) -> Result<String> {
        let _permit = self.breaker.enter()?;
        self.recall_once(query, person).await
    }
}

#[cfg(test)]
#[path = "tiny_tests.rs"]
mod tests;
