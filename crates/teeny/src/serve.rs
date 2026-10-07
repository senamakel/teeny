//! `teeny serve`: builds the [`App`] from the configuration and runs it
//! until SIGINT or SIGTERM.
//!
//! On boot, in order: the configuration is cut to this process's role
//! ([`schedule::for_role`]); the App is built (Mongo, the clients, the
//! OpenHuman runtime and every agent); the port is bound, so a clash fails
//! before anything is scheduled; the rota is installed on OpenHuman's cron
//! (every handler, then every job) and only then is cron started;
//! the reply worker (`REPLY_MODE=worker`), the activity stream and Telegram
//! start for the roles that own them; the HTTP server takes requests.
//!
//! On shutdown: the cron scheduler and the runtime's services stop, then
//! Telegram (answering what is waiting) and the reply worker (finishing its
//! batch), then the HTTP server lets its requests finish, then the stream
//! connection drops.
//!
//! `--dry-run` forces `DRY_RUN` and `TELEGRAM_DRY_RUN` on, whatever the
//! environment says.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use axum::Router;
use teeny_cadence::calendar::load_calendar;
use teeny_core::Config;
use teeny_core::config::ReplyMode;
use teeny_jobs::reply::worker::ReplyWorker;
use teeny_x::intake::ActivityIntake;
use tokio::net::TcpListener;

use crate::adapters::{BrowserState, OperatorAgent};
use crate::app::App;
use crate::http::article_store::FileArticleStore;
use crate::http::brain::BrainReader;
use crate::http::{
    AgentRunner, BrainSource, BrowserStatus, Forwarder, HttpForwarder, Mind, MindStatus, Review,
    Services, router,
};
use crate::release::{ChatSend, HoldRelease};
use crate::schedule::{self, Dispatch, RuntimeScheduler, runs_ingest, runs_replies};
use crate::stream::StreamTask;
use crate::telegram::{self, api::TelegramApi};

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

/// Serves until `signal`, then runs `before_server` (the schedule and the
/// transports stop while the server still answers), stops the server, and
/// runs `after_server` (the stream).
pub async fn serve_until(
    listener: TcpListener,
    app: Router,
    signal: impl Future<Output = ()> + Send + 'static,
    before_server: impl Future<Output = ()>,
    after_server: impl Future<Output = ()>,
) -> anyhow::Result<()> {
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(serve_on(listener, app, async {
        let _ = stopped.await;
    }));
    signal.await;
    before_server.await;
    let _ = stop.send(());
    let served = server.await.context("the HTTP server task ended badly")?;
    after_server.await;
    served
}

/// `config` with both rehearsal switches on: nothing is posted to X and
/// nothing is sent on Telegram.
pub fn force_dry_run(mut config: Config) -> Config {
    config.dry_run = true;
    config.telegram.dry_run = true;
    config
}

/// Builds everything from `config` and serves it on `PORT` until the
/// process is told to stop.
pub async fn serve(config: Config, dry_run: bool) -> anyhow::Result<()> {
    let config = if dry_run {
        force_dry_run(config)
    } else {
        config
    };
    let app = App::build(schedule::for_role(&config)).await?;
    let config = Arc::clone(&app.config);
    let role = config.server.service_role;

    let listener = TcpListener::bind(("0.0.0.0", config.server.port))
        .await
        .with_context(|| format!("could not listen on port {}", config.server.port))?;

    let telegram_api = config
        .telegram
        .bot_token
        .as_ref()
        .map(|token| Arc::new(TelegramApi::new(token.expose())));
    // One intake for both transports, so an event on both is handled once.
    let intake = Arc::new(app.intake());
    let services = services(&app, Arc::clone(&intake), telegram_api.clone());

    let calendar = config
        .calendar
        .enabled
        .then(|| load_calendar(&config.calendar.path))
        .transpose()?;
    let jobs = schedule::job_table(&config, calendar.as_ref())?;
    let mut dispatch = Dispatch::new(Arc::clone(&app.ctx));
    if let Some(api) = &telegram_api {
        dispatch = dispatch.with_telegram(api.clone());
    }
    // Handlers, jobs, prune, then cron starts: nothing fires into nothing.
    schedule::boot(
        &RuntimeScheduler(&app.runtime),
        &jobs,
        Arc::new(dispatch),
        tokio::runtime::Handle::current(),
    )
    .await?;
    for job in &jobs {
        tracing::info!(job = job.name, schedule = ?job.schedule, "cron: scheduled");
    }

    let worker = (runs_replies(role) && config.reply.mode == ReplyMode::Worker)
        .then(|| ReplyWorker::for_replies(Arc::clone(&app.ctx)).start());
    let stream = match (
        runs_ingest(role) && config.x.activity_stream_enabled,
        &config.x.bearer_token,
    ) {
        (true, Some(bearer)) => Some(StreamTask::start(bearer.clone(), Arc::clone(&intake))),
        (true, None) => {
            tracing::info!("activity stream: off, no X_BEARER_TOKEN");
            None
        }
        (false, _) => None,
    };
    let telegram = telegram::start(&app, telegram_api).await?;

    tracing::info!(
        port = config.server.port,
        role = role.as_str(),
        dry_run = config.dry_run,
        telegram_dry_run = config.telegram.dry_run,
        jobs = jobs.len(),
        replies = if worker.is_some() {
            "worker"
        } else {
            "cron or off"
        },
        stream = stream.is_some(),
        telegram = telegram.is_some(),
        "teeny started"
    );

    let runtime = Arc::clone(&app.runtime);
    let before_server = async move {
        runtime.stop_services();
        tracing::info!("cron and the runtime's services stopped");
        if let Some(telegram) = telegram {
            telegram.stop().await;
        }
        if let Some(worker) = worker {
            worker.stop().await;
        }
    };
    let after_server = async move {
        if let Some(stream) = stream {
            stream.stop().await;
        }
    };
    serve_until(
        listener,
        router(services),
        shutdown_signal(),
        before_server,
        after_server,
    )
    .await
}

/// Everything the routes reach.
fn services(
    app: &App,
    intake: Arc<ActivityIntake>,
    telegram: Option<Arc<TelegramApi>>,
) -> Services {
    let config = &app.config;
    let mut services = Services::new(config.x.api_secret.expose(), intake);
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
    services.queue = Some(app.queue.clone());
    services.mind = Some(Mind {
        journal: Arc::clone(&app.journal),
        status: MindStatus {
            mode: config.agent.mode.as_str().to_owned(),
            model: config.llm.model.clone(),
            dry_run: config.dry_run,
        },
    });
    services.ledger = Some(Arc::clone(&app.ledger));
    services.brain =
        config.brain.data_dir.as_ref().map(|dir| {
            Arc::new(BrainReader::new(dir, &config.brain.company)) as Arc<dyn BrainSource>
        });
    services.articles = Some(Arc::new(FileArticleStore::new(&config.article.dir)));
    services.safety = Some(Arc::clone(&app.deps.safety));
    services.review = Some(Review {
        holds: Arc::clone(&app.holds),
        release: Some(Arc::new(HoldRelease {
            x: Arc::clone(&app.deps.x),
            cadence: Arc::clone(&app.deps.cadence),
            dry_run: config.dry_run,
            telegram: telegram.map(|api| api as Arc<dyn ChatSend>),
            telegram_dry_run: config.telegram.dry_run,
        })),
    });
    services.critic = app.deps.critic.clone();
    services.agent = Some(Arc::new(OperatorAgent(Arc::clone(&app.runner))) as Arc<dyn AgentRunner>);
    services.browser =
        Some(Arc::new(BrowserState::from_settings(&app.settings)) as Arc<dyn BrowserStatus>);
    services
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
