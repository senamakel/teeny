//! The rota on OpenHuman's cron: every Teeny job is a cron **system job**
//! whose handler is host code (`teeny_jobs::<job>::run`), upserted at boot by
//! name, so the schedule in the store always matches the configuration.
//!
//! | Job | Cron | Runs in role |
//! |---|---|---|
//! | `teeny_ingest` | `INGEST_CRON` (`*/5 * * * *`) | all, ingest |
//! | `teeny_brain_usage` | `BRAIN_USAGE_CRON` (`* * * * *`), with a brain dir | all, ingest |
//! | `teeny_heat` | `HEAT_CRON` (`*/20 * * * *`), `HEAT_ENABLED` | all, reply |
//! | `teeny_reply` | `REPLY_CRON`, only with `REPLY_MODE=cron` | all, reply |
//! | `teeny_cortex_health` | `CORTEX_HEALTH_CRON` (`17 * * * *`), and once at boot | every role |
//! | `teeny_learnings` | `LEARNINGS_CRON` (`0 5 * * *`), `LEARNINGS_ENABLED` | all, morning |
//! | `teeny_morning` | `MORNING_CRON` (`0 7 * * *`) | all, morning |
//! | `teeny_afternoon` | `AFTERNOON_CRON` (`0 15 * * *`): chart, moon, else lore | all, chart, moon, lore |
//! | `teeny_evening` | `EVENING_CRON` (`0 23 * * *`): article, serial, else quote | all, article, narrative, quote |
//! | `teeny_calendar_<item>` | the item's cron, weekdays named | all, calendar |
//!
//! The day is three posts on a three-day rota: the morning every day, then
//! the two slots, each one cron that runs the day's owner and hands the day
//! to its fallback when the owner is off or does not post
//! ([`teeny_cadence::slot`]). The per-job crons the four-slot calendar had
//! (`CHART_CRON`, `MOON_CRON`, `LORE_CRON`, `QUOTE_CRON`) are gone.
//!
//! With `REPLY_MODE=worker` (the default) the reply job is not a cron: the
//! reply worker drains the queue in a loop, so a mention is answered in
//! seconds (see `serve`).
//!
//! **Boot order.** The runtime is built with cron stopped; [`boot`] installs
//! every handler, upserts the table, prunes the stale jobs, and only then
//! starts the scheduler, so a job already due at boot runs its handler.
//!
//! Every job is `retries(0)` (a retry re-runs a turn that may have posted)
//! and `single_flight` (a slot that comes due mid-run is skipped and
//! recorded). A handler's `Err` is the run's recorded error.
//!
//! **Roles.** `SERVICE_ROLE` decides which jobs this process owns, as
//! `legacy/src/index.ts` did: [`for_role`] switches off every publishing job
//! the role does not run, so the afternoon and evening dispatchers only ever
//! hand a slot to a job this process owns. The learnings brief has one
//! writer, the morning's: every process reads the file, and two writers
//! would race to rewrite it. A stored `teeny_*` job the table no longer names
//! (`teeny_quote`, since the quote became the evening's fallback) is removed.
//!
//! **Time zones.** Every cron is read in `CALENDAR_TZ` (UTC when unset); a
//! `timezone` in `calendar.json` wins for its items. Numeric weekdays are
//! written as names first ([`to_named_weekdays`]): OpenHuman's cron counts
//! Sunday as 1, POSIX as 0, and names mean the same to both.

use std::sync::Arc;

use anyhow::Context as _;
use futures::FutureExt as _;
use futures::future::BoxFuture;
use openhuman_embed::{JobSchedule, JobSpec, Runtime};
use teeny_cadence::calendar::{Calendar, CalendarItem, MORNING_CALENDAR_SLOT};
use teeny_cadence::slot::SlotOutcome;
use teeny_core::Config;
use teeny_core::config::{ReplyMode, ServiceRole};
use teeny_jobs::calendar::{CalendarJob, TelegramSender, to_named_weekdays};
use teeny_jobs::ctx::{JobCtx, JobReport};

/// Every Teeny job's name starts with this; anything else in the store is
/// somebody else's and is left alone.
pub const PREFIX: &str = "teeny_";

/// What a job does when it fires.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Work {
    Ingest,
    BrainUsage,
    Heat,
    Reply,
    CortexHealth,
    Learnings,
    Morning,
    Afternoon,
    Evening,
    Calendar(Box<CalendarItem>),
}

/// One row of the table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobDef {
    pub name: String,
    pub schedule: JobSchedule,
    pub work: Work,
}

/// Whether `role` runs the ingest side: the poll, the brain fold, the
/// activity stream and Telegram (one poller per token, so it rides with the
/// role that runs in exactly one place).
pub fn runs_ingest(role: ServiceRole) -> bool {
    matches!(role, ServiceRole::All | ServiceRole::Ingest)
}

/// Whether `role` runs the reply side: the reply job and the heat reading.
pub fn runs_replies(role: ServiceRole) -> bool {
    matches!(role, ServiceRole::All | ServiceRole::Reply)
}

/// `config` with every job this process's role does not run switched off.
pub fn for_role(config: &Config) -> Config {
    let role = config.server.service_role;
    let owns = |job: ServiceRole| role == ServiceRole::All || role == job;
    let mut config = config.clone();
    config.morning.enabled &= owns(ServiceRole::Morning);
    config.moon.enabled &= owns(ServiceRole::Moon);
    config.chart.enabled &= owns(ServiceRole::Chart);
    config.lore.enabled &= owns(ServiceRole::Lore);
    config.quote.enabled &= owns(ServiceRole::Quote);
    config.narrative.enabled &= owns(ServiceRole::Narrative);
    config.article.enabled &= owns(ServiceRole::Article);
    config.calendar.enabled &= owns(ServiceRole::Calendar);
    config.cadence.heat_enabled &= runs_replies(role);
    config.learnings.enabled &= owns(ServiceRole::Morning);
    config
}

fn zone(name: &str) -> anyhow::Result<String> {
    name.parse::<chrono_tz::Tz>()
        .map(|tz| tz.name().to_owned())
        .map_err(|_| anyhow::anyhow!("CALENDAR_TZ: {name:?} is not an IANA time zone"))
}

fn cron(expr: &str, tz: &Option<String>) -> JobSchedule {
    JobSchedule::Cron {
        expr: to_named_weekdays(expr),
        tz: tz.clone(),
    }
}

/// A store-safe job name for a calendar item.
fn calendar_name(item: &str) -> String {
    let safe: String = item
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    format!("{PREFIX}calendar_{safe}")
}

/// The jobs `config` (already through [`for_role`]) asks for, in a fixed
/// order. `calendar` is the loaded `calendar.json` when the calendar is on.
pub fn job_table(config: &Config, calendar: Option<&Calendar>) -> anyhow::Result<Vec<JobDef>> {
    let tz = config.calendar.timezone.as_deref().map(zone).transpose()?;
    let role = config.server.service_role;
    let mut jobs = Vec::new();
    let mut add = |name: &str, expr: &str, work: Work| {
        jobs.push(JobDef {
            name: format!("{PREFIX}{name}"),
            schedule: cron(expr, &tz),
            work,
        });
    };
    if runs_ingest(role) {
        add("ingest", &config.reply.ingest_cron, Work::Ingest);
        if config.brain.data_dir.is_some() {
            add("brain_usage", &config.brain.usage_cron, Work::BrainUsage);
        }
    }
    if runs_replies(role) {
        if config.cadence.heat_enabled {
            add("heat", &config.cadence.heat_cron, Work::Heat);
        }
        if config.reply.mode == ReplyMode::Cron {
            add("reply", &config.reply.cron, Work::Reply);
        }
    }
    // The store's own report, in every process: an embedding outage degrades
    // every recall without failing any of them, so nothing else would say so.
    add(
        "cortex_health",
        &config.cortex.health_cron,
        Work::CortexHealth,
    );
    if config.learnings.enabled {
        add("learnings", &config.learnings.cron, Work::Learnings);
    }
    if config.morning.enabled {
        add("morning", &config.morning.cron, Work::Morning);
    }
    if config.chart.enabled || config.moon.enabled || config.lore.enabled {
        add("afternoon", &config.rota.afternoon_cron, Work::Afternoon);
    }
    if config.article.enabled || config.narrative.enabled || config.quote.enabled {
        add("evening", &config.rota.evening_cron, Work::Evening);
    }
    if config.calendar.enabled
        && let Some(calendar) = calendar
    {
        let item_tz = calendar
            .timezone
            .map(|tz| tz.name().to_owned())
            .or_else(|| tz.clone());
        for item in calendar.items.iter().filter(|item| item.enabled) {
            if config.morning.enabled && item.name == MORNING_CALENDAR_SLOT {
                tracing::warn!(
                    item = item.name,
                    cron = item.cron,
                    "calendar: skipping the item, the morning job posts the good morning"
                );
                continue;
            }
            jobs.push(JobDef {
                name: calendar_name(&item.name),
                schedule: cron(&item.cron, &item_tz),
                work: Work::Calendar(Box::new(item.clone())),
            });
        }
    }
    Ok(jobs)
}

/// What fires a job: the host's code, with its result as the run's.
pub type Handler = Arc<dyn Fn() -> BoxFuture<'static, Result<(), String>> + Send + Sync>;

/// The part of OpenHuman's cron the installer uses.
pub trait Scheduler {
    /// Registers the handler for system job `name`.
    fn on_job(&self, name: &str, handler: Handler) -> anyhow::Result<()>;
    fn upsert(&self, spec: JobSpec) -> anyhow::Result<()>;
    /// Every stored job's name.
    fn names(&self) -> anyhow::Result<Vec<String>>;
    fn remove(&self, name: &str) -> anyhow::Result<()>;
    /// Starts the scheduler: from here on, due jobs fire.
    fn start(&self) -> BoxFuture<'static, ()>;
}

/// The runtime's cron.
pub struct RuntimeScheduler<'a>(pub &'a Arc<Runtime>);

impl Scheduler for RuntimeScheduler<'_> {
    fn on_job(&self, name: &str, handler: Handler) -> anyhow::Result<()> {
        Ok(self.0.on_system_job(name, move |_ctx| handler())?)
    }

    fn upsert(&self, spec: JobSpec) -> anyhow::Result<()> {
        self.0.cron().upsert(spec)?;
        Ok(())
    }

    fn names(&self) -> anyhow::Result<Vec<String>> {
        Ok(self
            .0
            .cron()
            .list()?
            .into_iter()
            .map(|job| job.name)
            .collect())
    }

    fn remove(&self, name: &str) -> anyhow::Result<()> {
        self.0.cron().remove(name)?;
        Ok(())
    }

    fn start(&self) -> BoxFuture<'static, ()> {
        teeny_agent::runtime::start_services(Arc::clone(self.0)).boxed()
    }
}

/// Runs a job's work against the one [`JobCtx`] the process holds.
pub struct Dispatch {
    pub ctx: Arc<JobCtx>,
    /// Sends a calendar item's Telegram half; absent, that half fails.
    pub telegram: Option<Arc<dyn TelegramSender>>,
}

impl Dispatch {
    pub fn new(ctx: Arc<JobCtx>) -> Self {
        Self {
            ctx,
            telegram: None,
        }
    }

    pub fn with_telegram(mut self, telegram: Arc<dyn TelegramSender>) -> Self {
        self.telegram = Some(telegram);
        self
    }

    /// One run: `Err` carries why, for cron to record.
    pub async fn run(&self, work: &Work) -> Result<(), String> {
        let ctx = self.ctx.as_ref();
        let ran: anyhow::Result<JobReport> = match work {
            Work::Ingest => teeny_jobs::ingest::run(ctx).await,
            Work::BrainUsage => teeny_jobs::brain_fold::run(ctx).await,
            Work::Heat => teeny_jobs::heat::run(ctx).await,
            Work::Reply => teeny_jobs::reply::run(ctx).await,
            Work::CortexHealth => {
                teeny_memory::health::report_cortex_health(&ctx.config.cortex.api_url).await;
                return Ok(());
            }
            Work::Learnings => teeny_jobs::learnings::run(ctx).await,
            Work::Morning => teeny_jobs::morning::run(ctx).await,
            Work::Afternoon => {
                return slot_done("afternoon", teeny_jobs::afternoon::run(ctx).await);
            }
            Work::Evening => return slot_done("evening", teeny_jobs::evening::run(ctx).await),
            Work::Calendar(item) => {
                let mut job = CalendarJob::from_ctx(ctx);
                if let Some(telegram) = &self.telegram {
                    job = job.with_telegram(Arc::clone(telegram));
                }
                job.run(ctx, item, false).await
            }
        };
        match ran {
            Ok(report) => {
                tracing::info!(job = report.job, ran = report.ran, outcome = ?report.outcome, note = ?report.note, "job done");
                Ok(())
            }
            Err(error) => Err(format!("{error:#}")),
        }
    }
}

/// A slot's run, for cron to record: an error only when the slot's last
/// word was a failure (an owner's failure its fallback answered is logged,
/// not raised).
fn slot_done(label: &str, outcome: anyhow::Result<SlotOutcome>) -> Result<(), String> {
    match outcome {
        Err(error) => Err(format!("{error:#}")),
        Ok(outcome) => match outcome.failure() {
            Some((job, error)) => Err(format!("{job}: {error}")),
            None => {
                tracing::info!(
                    slot = label,
                    posted = outcome.posted(),
                    ?outcome,
                    "slot done"
                );
                Ok(())
            }
        },
    }
}

/// Puts `jobs` on `scheduler`: each one's handler first (so a job never
/// fires into nothing), then the job, unretried and single-flight; then
/// every stored `teeny_*` job the table no longer names goes. Handlers run
/// their work on `handle`, the service's own runtime, not the core's.
pub fn install(
    scheduler: &dyn Scheduler,
    jobs: &[JobDef],
    dispatch: Arc<Dispatch>,
    handle: tokio::runtime::Handle,
) -> anyhow::Result<()> {
    for job in jobs {
        let (dispatch, handle, work, name) = (
            Arc::clone(&dispatch),
            handle.clone(),
            job.work.clone(),
            job.name.clone(),
        );
        let handler: Handler = Arc::new(move || {
            let (dispatch, work, name) = (Arc::clone(&dispatch), work.clone(), name.clone());
            let task = handle.spawn(async move {
                tracing::info!(job = name, "cron: running");
                let ran = dispatch.run(&work).await;
                if let Err(error) = &ran {
                    tracing::error!(job = name, %error, "cron: the job failed");
                }
                ran
            });
            async move {
                task.await
                    .unwrap_or_else(|error| Err(format!("the job's task ended badly: {error}")))
            }
            .boxed()
        });
        scheduler
            .on_job(&job.name, handler)
            .with_context(|| format!("could not register the handler for {}", job.name))?;
        scheduler
            .upsert(
                JobSpec::system(&job.name, &job.name, job.schedule.clone())
                    .retries(0)
                    .single_flight(true),
            )
            .with_context(|| format!("could not schedule {}", job.name))?;
    }
    for stale in scheduler.names()? {
        if stale.starts_with(PREFIX) && !jobs.iter().any(|job| job.name == stale) {
            tracing::info!(
                job = stale,
                "cron: removing a job that is no longer configured"
            );
            scheduler.remove(&stale)?;
        }
    }
    Ok(())
}

/// The boot order: [`install`] every handler and job (and prune the stale
/// ones), and only then start the scheduler. The runtime is built with cron
/// stopped, so a stored job already due at boot cannot fire before its
/// handler exists (it would be recorded "ok" without running). A failed
/// install leaves cron stopped.
pub async fn boot(
    scheduler: &dyn Scheduler,
    jobs: &[JobDef],
    dispatch: Arc<Dispatch>,
    handle: tokio::runtime::Handle,
) -> anyhow::Result<()> {
    install(scheduler, jobs, dispatch, handle)?;
    scheduler.start().await;
    Ok(())
}

#[cfg(test)]
#[path = "schedule_tests.rs"]
mod tests;
