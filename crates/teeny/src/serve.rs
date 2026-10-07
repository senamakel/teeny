//! `teeny serve`: builds the services from the configuration and runs the
//! HTTP server on `PORT` until SIGINT or SIGTERM.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use axum::Router;
use teeny_core::Config;
use teeny_memory::CortexMemory;
use teeny_safety::gate::{SafetyGate, SafetyGateOptions};
use teeny_store::{
    HoldLog, MongoHoldLog, MongoJournal, MongoLedger, MongoMentionQueue, MongoPeople,
};
use teeny_x::XApi;
use teeny_x::intake::ActivityIntake;
use tokio::net::TcpListener;

use crate::http::article_store::FileArticleStore;
use crate::http::brain::BrainReader;
use crate::http::{
    BrainSource, Forwarder, HttpForwarder, Mind, MindStatus, Review, Services, router,
};
use crate::mentions::{Mentions, QueueAdapter, QueuedXPost};
use crate::sinks::{MemorySink, PeopleRecord};

/// Serves `app` on `listener` until `shutdown` resolves, then lets requests
/// in flight finish.
pub async fn serve_on(
    listener: TcpListener,
    app: Router,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> anyhow::Result<()> {
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await?;
    Ok(())
}

/// Builds the services from `config` and serves them on `PORT` until the
/// process is told to stop.
pub async fn serve(config: &Config) -> anyhow::Result<()> {
    let services = services(config).await?;
    let listener = TcpListener::bind(("0.0.0.0", config.server.port))
        .await
        .with_context(|| format!("could not listen on port {}", config.server.port))?;
    tracing::info!(port = config.server.port, "listening");
    serve_on(listener, router(services), shutdown_signal()).await
}

/// Everything the routes reach, over the real stores where they exist.
///
/// TODO(teeny-agent): wire `agent` (the operator agent's `AgentRunner`),
/// `critic` (`AlgoCritic` over the critic model), the safety gate's judge,
/// and the review's `release` (the publisher) once teeny-agent provides them.
/// TODO(browser): wire `browser` for `/health` when the browser module lands.
async fn services(config: &Config) -> anyhow::Result<Services> {
    let db = teeny_store::connect(&config.store).await?;
    let queue: Arc<Mentions> = Arc::new(MongoMentionQueue::<QueuedXPost>::with_collection(
        &db,
        &config.store.mentions_collection,
    ));
    let holds: Arc<dyn HoldLog> = Arc::new(MongoHoldLog::with_collection(
        &db,
        &config.store.holds_collection,
    ));
    let memory = CortexMemory::from_config(&config.cortex)?;
    let intake = ActivityIntake::new(Arc::new(MemorySink(Arc::new(memory))))
        .with_queue(
            Arc::new(QueueAdapter(queue.clone())),
            Arc::new(XApi::from_config(&config.x)),
            &config.x.bot_username,
        )
        .with_people(Arc::new(PeopleRecord(Arc::new(MongoPeople::new(&db)))));
    let gate = SafetyGate::new(SafetyGateOptions {
        holds: Some(holds.clone()),
        extra_allowed_links: config.safety.link_allowlist.clone(),
        ..SafetyGateOptions::default()
    });

    let mut services = Services::new(config.x.api_secret.expose(), Arc::new(intake));
    services.forwarder = config.webhook.forward_url.as_ref().map(|url| {
        Arc::new(HttpForwarder::new(
            url.clone(),
            config
                .webhook
                .forward_token
                .as_ref()
                .map(|token| token.expose().to_owned()),
            FORWARD_TIMEOUT,
        )) as Arc<dyn Forwarder>
    });
    services.agent_token = config
        .agent
        .api_token
        .as_ref()
        .map(|token| token.expose().to_owned());
    services.queue = Some(queue);
    services.mind = Some(Mind {
        journal: Arc::new(MongoJournal::new(&db)),
        status: MindStatus {
            mode: config.agent.mode.as_str().to_owned(),
            model: config.llm.model.clone(),
            dry_run: config.dry_run,
        },
    });
    services.ledger = Some(Arc::new(MongoLedger::new(&db)));
    services.brain =
        config.brain.data_dir.as_ref().map(|dir| {
            Arc::new(BrainReader::new(dir, &config.brain.company)) as Arc<dyn BrainSource>
        });
    services.articles = Some(Arc::new(FileArticleStore::new(&config.article.dir)));
    services.safety = Some(Arc::new(gate));
    services.review = Some(Review {
        holds,
        release: None,
    });
    Ok(services)
}

/// How long one forwarded copy may take.
const FORWARD_TIMEOUT: Duration = Duration::from_secs(10);

/// SIGINT, or SIGTERM from the container runtime.
async fn shutdown_signal() {
    let interrupt = async {
        if let Err(failure) = tokio::signal::ctrl_c().await {
            tracing::warn!(err = %failure, "could not listen for SIGINT");
            std::future::pending::<()>().await;
        }
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(failure) => {
                tracing::warn!(err = %failure, "could not listen for SIGTERM");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = interrupt => {}
        () = terminate => {}
    }
    tracing::info!("shutting down");
}

#[cfg(test)]
#[path = "serve_tests.rs"]
mod tests;
