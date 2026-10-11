//! Everything the service runs on, built once from [`Config`]: the clients
//! ([`Deps`]), the OpenHuman runtime with every Teeny agent registered on
//! it, the runner over those agents, and the [`JobCtx`] every job and chat
//! turn shares. `teeny serve` builds one; an operator subcommand can too.
//!
//! The wiring:
//!
//! - **Mongo** (connects lazily): the mention queue, the holds, the journal,
//!   the inference and post ledgers, the people record, the Telegram
//!   transcript, and X's read cache (inside [`Deps`]: `CachedX` over `XApi`
//!   through `StoreReadCache`).
//! - **[`Deps`]**: CortexDB memory, X, the content filter with the model
//!   judge (`SAFETY_JUDGE`), the critic, the cadence and the reply budget,
//!   the market, media, Exa and the ladder.
//! - **OpenHuman**: the process's one runtime (`teeny_agent::runtime`), a
//!   locked-down agent per belt (`TeenyAgents`), and [`OhRunner`] writing a
//!   ledger row per model call.
//! - **[`JobCtx`]** with its conversation side: the queue, the poll's cursor
//!   in `<TEENY_DATA>/state/state.json`, X as the account and the mention
//!   source, the reply outlet (`DRY_RUN` honoured), the article lookup over
//!   `ARTICLE_DIR`, and the brain fold over `BRAIN_DATA_DIR`.

use std::path::PathBuf;
use std::sync::Arc;

use openhuman_embed::Runtime;
use teeny_agent::deps::Deps;
use teeny_agent::runtime::{self as oh_runtime, RuntimeSettings};
use teeny_agent::specs::{SpecContext, TeenyAgents};
use teeny_core::Config;
use teeny_jobs::brain_fold::BrainFold;
use teeny_jobs::ctx::{Conversation, JobCtx};
use teeny_jobs::oh_runner::OhRunner;
use teeny_jobs::reply::{Pacing, XOutlet};
use teeny_jobs::runner::AgentRunner;
use teeny_store::{
    FileStateStore, HoldLog, InferenceLedger, Journal, MongoChatTranscript, MongoHoldLog,
    MongoJournal, MongoLedger, MongoMentionQueue, MongoPeople, MongoPostLedger, People,
};
use teeny_x::intake::ActivityIntake;

use crate::mentions::{Mentions, QueueAdapter, QueuedXPost};
use crate::sinks::{MemorySink, PeopleRecord};

/// The built service.
pub struct App {
    pub config: Arc<Config>,
    pub deps: Arc<Deps>,
    pub settings: RuntimeSettings,
    pub runtime: Arc<Runtime>,
    /// Holding these keeps every agent registered: the runtime frees an id
    /// when its last handle drops. The runner holds them too.
    #[allow(dead_code)]
    pub agents: Arc<TeenyAgents>,
    pub runner: Arc<dyn AgentRunner>,
    pub ctx: Arc<JobCtx>,
    pub queue: Arc<Mentions>,
    pub holds: Arc<dyn HoldLog>,
    pub journal: Arc<dyn Journal>,
    pub ledger: Arc<dyn InferenceLedger>,
    pub people: Arc<dyn People>,
}

/// Where the mention poll keeps its cursor: `<TEENY_DATA>/state/state.json`,
/// where the entrypoint migrates the legacy `data/state.json` to.
pub fn cursor_path(config: &Config) -> PathBuf {
    PathBuf::from(&config.openhuman.data_dir)
        .join("state")
        .join("state.json")
}

impl App {
    /// Builds everything. Mongo connects lazily, so this waits on no server;
    /// it does boot the OpenHuman runtime (once per process).
    pub async fn build(config: Config) -> anyhow::Result<Self> {
        let db = teeny_store::connect(&config.store).await?;
        let deps = Arc::new(Deps::from_config(config.clone()).await?);
        let settings = RuntimeSettings::from_config(&config.openhuman);
        let runtime = oh_runtime::global(settings.clone()).await?;
        let agents = Arc::new(TeenyAgents::register(
            Arc::clone(&runtime),
            &deps,
            SpecContext::from_config(&config),
        )?);
        let ledger: Arc<dyn InferenceLedger> = Arc::new(MongoLedger::new(&db));
        let runner: Arc<dyn AgentRunner> =
            Arc::new(OhRunner::new(Arc::clone(&agents)).with_ledger(Arc::clone(&ledger)));
        let queue = Arc::new(MongoMentionQueue::<QueuedXPost>::with_collection(
            &db,
            &config.store.mentions_collection,
        ));
        let holds: Arc<dyn HoldLog> = Arc::new(MongoHoldLog::with_collection(
            &db,
            &config.store.holds_collection,
        ));
        let people: Arc<dyn People> = Arc::new(MongoPeople::new(&db));
        let journal: Arc<dyn Journal> = Arc::new(MongoJournal::new(&db));

        let conversation = Conversation {
            queue: queue.clone(),
            exchanges: Some(queue.clone()),
            people: Some(Arc::clone(&people)),
            cursor: Arc::new(FileStateStore::new(cursor_path(&config))),
            transcript: Arc::new(MongoChatTranscript::with_collection(
                &db,
                &config.store.transcript_collection,
            )),
            posts: Arc::new(MongoPostLedger::new(&db)),
            inferences: Some(Arc::clone(&ledger)),
            account: deps.x.clone(),
            mentions: deps.x.clone(),
            outlet: Arc::new(XOutlet::new(Arc::clone(&deps.x), config.dry_run)),
            articles: Some(Arc::new(teeny_jobs::article::FileArticleStore::new(
                &config.article.dir,
            ))),
            brain: config
                .brain
                .data_dir
                .as_ref()
                .map(|dir| Arc::new(BrainFold::new(dir, &config.brain.company))),
            pacing: Pacing::default(),
        };
        let config = Arc::new(config);
        let ctx = JobCtx::new(
            Arc::clone(&deps),
            Arc::clone(&runner),
            Arc::clone(&config),
            Arc::clone(&journal),
        )
        .with_conversation(conversation);

        Ok(Self {
            config,
            deps,
            settings,
            runtime,
            agents,
            runner,
            ctx: Arc::new(ctx),
            queue,
            holds,
            journal,
            ledger,
            people,
        })
    }

    /// The one intake the webhook and the activity stream both deliver to:
    /// activity into memory, mentions into the queue (as the account, once
    /// X says who that is), interactions into the people record. Its event-id
    /// dedupe is what keeps an event that arrives on both transports from
    /// being handled twice, so build it once and share it.
    pub fn intake(&self) -> ActivityIntake {
        ActivityIntake::new(Arc::new(MemorySink(Arc::clone(&self.deps.memory))))
            .with_queue(
                Arc::new(QueueAdapter(self.queue.clone())),
                self.deps.x.clone(),
                &self.config.x.bot_username,
            )
            .with_people(Arc::new(PeopleRecord(Arc::clone(&self.people))))
    }
}

#[cfg(test)]
#[path = "app_tests.rs"]
mod tests;
