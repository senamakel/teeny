//! The account's memory on the runtime's shared store: [`RuntimeStore`] is
//! `teeny_memory`'s [`ItemStore`] over OpenHuman's memory facade
//! (`Runtime::memory(root)`), so what the host captures and recalls lands in
//! the same TinyMemory root every agent is bound to.
//!
//! The runtime is reached lazily: building the clients ([`crate::deps`])
//! does not boot OpenHuman, the first capture or recall does (once per
//! process, [`runtime::global`]). Every call runs on the OpenHuman executor,
//! where the engine's HTTP client lives, so a caller on another tokio
//! runtime (a test's, the server's) never strands a pooled connection.

use std::sync::Arc;

use async_trait::async_trait;
use openhuman_embed::memory::api::{Hit, ItemKind, StoreItem, StoreReceipt, WriteOptions};
use openhuman_embed::memory::{ItemsQuery, Memory, MemoryError as FacadeError, SearchQuery};
use teeny_memory::{CortexOptions, ItemStore, MemoryError, Result, TinyMemory};

use crate::runtime::{self, RuntimeSettings};

/// The runtime's memory facade for one root, as an [`ItemStore`].
#[derive(Debug, Clone)]
pub struct RuntimeStore {
    settings: RuntimeSettings,
}

/// What a facade error reads as here.
fn store_error(error: FacadeError) -> MemoryError {
    MemoryError::Store(error.to_string())
}

impl RuntimeStore {
    /// The store the runtime booted with `settings` holds, under
    /// `settings.memory.root`.
    pub fn new(settings: RuntimeSettings) -> Self {
        Self { settings }
    }

    /// The facade for the root, booting the runtime on first use.
    async fn facade(&self) -> Result<Memory> {
        let runtime = runtime::global(self.settings.clone())
            .await
            .map_err(|error| MemoryError::Store(error.to_string()))?;
        runtime
            .memory(&self.settings.memory.root)
            .map_err(store_error)
    }

    /// `call` on the facade, on the OpenHuman executor.
    async fn on_executor<T, F, Fut>(&self, call: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(Memory) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<T>> + Send + 'static,
    {
        let store = self.clone();
        runtime::executor()
            .spawn(async move { call(store.facade().await?).await })
            .await
            .map_err(|error| MemoryError::Store(format!("the memory task failed: {error}")))?
    }
}

#[async_trait]
impl ItemStore for RuntimeStore {
    async fn store(&self, item: StoreItem) -> Result<StoreReceipt> {
        self.on_executor(|memory| async move {
            memory
                .store(item, None, WriteOptions::accepted())
                .await
                .map_err(store_error)
        })
        .await
    }

    async fn search(&self, query: &str, tags_any: &[String], limit: usize) -> Result<Vec<Hit>> {
        let query = SearchQuery {
            query: query.to_owned(),
            tags_any: tags_any.to_vec(),
            limit: Some(limit),
            ..SearchQuery::default()
        };
        self.on_executor(|memory| async move {
            Ok(memory.search(query).await.map_err(store_error)?.hits)
        })
        .await
    }

    async fn latest(&self, tags_any: &[String], limit: usize) -> Result<Vec<Hit>> {
        let query = ItemsQuery {
            tags_any: tags_any.to_vec(),
            // What the account captured: conversation and activity, not the
            // beliefs built from them.
            kinds: vec![ItemKind::Conversation, ItemKind::Document],
            limit: Some(limit),
            ..ItemsQuery::default()
        };
        self.on_executor(|memory| async move {
            Ok(memory.list(query).await.map_err(store_error)?.items)
        })
        .await
    }
}

/// The account's memory: the shared store under the runtime `settings`
/// boots, with the deadlines and breaker `options` set.
pub fn shared_memory(settings: RuntimeSettings, options: CortexOptions) -> TinyMemory {
    TinyMemory::new(Arc::new(RuntimeStore::new(settings))).with_options(options)
}
