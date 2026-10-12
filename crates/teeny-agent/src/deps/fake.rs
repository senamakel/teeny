//! [`Deps::fake`]: every part in process, `DRY_RUN` on, nothing on the
//! network. The X, Exa and ladder clients point at a closed local port, so a
//! test that forgets to swap one in fails loudly instead of reaching out.

use std::sync::Arc;

use teeny_cadence::budget::{BudgetRules, QueueCounter, ReplyBudget};
use teeny_cadence::cadence::{Cadence, CadenceRules};
use teeny_cadence::clock::SystemClock;
use teeny_cadence::media_budget::{DailyMediaBudget, MediaLimits};
use teeny_core::{Config, Secret};
use teeny_media::store::MediaStore;
use teeny_media::wiring::MediaAccess;
use teeny_memory::InMemoryMemory;
use teeny_safety::critic::AlgoCritic;
use teeny_safety::gate::{SafetyGate, SafetyGateOptions};
use teeny_safety::model::ScriptedModel;
use teeny_store::mentions::InMemoryMentionQueue;
use teeny_store::posts::InMemoryPostLedger;
use teeny_store::{InMemoryLedger, InferenceLedger};
use teeny_x::XApi;

use super::{Deps, QueuedXPost, Settings, TokenFeed, fake_vars};
use crate::exa::ExaClient;
use crate::fakes::{FakeGifs, FakeImages, FakeMarket, FakeVideos, FakeVision, TOKEN};
use crate::llm::LlmClient;
use crate::prompt::Persona;
use crate::tools::media::CadenceMediaBudget;

/// Nothing listens here.
const NOWHERE: &str = "http://127.0.0.1:9";

impl Deps {
    /// Every part, in process: the configuration of [`fake_vars`], memory
    /// and ledgers in memory, the lexical content filter, fake media, GIFs
    /// and market, and clients that cannot reach anything.
    pub fn fake() -> Self {
        let config = Config::from_vars(fake_vars()).expect("the fake configuration loads");
        let ledger: Arc<dyn InferenceLedger> = Arc::new(InMemoryLedger::default());
        let x = Arc::new(XApi::from_config(&config.x).with_base_url(NOWHERE));
        let posts = Arc::new(InMemoryPostLedger::default());
        let queue = Arc::new(InMemoryMentionQueue::<QueuedXPost>::default());
        Self {
            settings: Settings {
                chart_output_dir: None,
                ..Settings::from_config(&config)
            },
            persona: Persona::default(),
            llm: LlmClient::new(NOWHERE, Secret::new("fake"), "chat").with_ledger(ledger),
            memory: Arc::new(InMemoryMemory::default()),
            x_reads: x.clone(),
            x,
            safety: Arc::new(SafetyGate::new(SafetyGateOptions::default())),
            critic: Some(Arc::new(AlgoCritic::new(Arc::new(ScriptedModel::new([]))))),
            cadence: Arc::new(Cadence::new(
                posts.clone(),
                CadenceRules::from_config(&config.cadence),
                Arc::new(SystemClock),
            )),
            reply_budget: Some(Arc::new(ReplyBudget::new(
                posts,
                Arc::new(QueueCounter::<QueuedXPost, _>::new(queue)),
                BudgetRules::from_config(&config),
                Arc::new(SystemClock),
            ))),
            media: Some(MediaAccess {
                store: Arc::new(MediaStore::new()),
                reader: Some(Arc::new(FakeVision("a smol robot".into()))),
                images: Some(Arc::new(FakeImages::default())),
                videos: Some(Arc::new(FakeVideos)),
                reference: None,
            }),
            media_budget: Arc::new(CadenceMediaBudget(Arc::new(DailyMediaBudget::new(
                MediaLimits::from_config(&config.media),
                Arc::new(SystemClock),
            )))),
            gifs: Some(Arc::new(FakeGifs)),
            market: Some(TokenFeed {
                data: Arc::new(FakeMarket::default()),
                address: TOKEN.into(),
                global: None,
            }),
            exa: Some(Arc::new(
                ExaClient::new(Secret::new("fake")).with_base_url(NOWHERE),
            )),
        }
    }
}
