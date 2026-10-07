//! `teeny cron list` and `teeny cron run <name>`: the OpenHuman cron jobs
//! the service registers, their last runs, and one run now.
//!
//! `run` refuses a live run: it goes ahead only under `DRY_RUN` or with
//! `--yes`. A system job's handler lives in the process that registered it,
//! so `run` builds the service's wiring and registers the handlers (only:
//! nothing is upserted or removed) before asking cron to run the job, which
//! records the run as a scheduled one would.

use std::io::Write;
use std::sync::Arc;
use std::time::SystemTime;

use anyhow::bail;
use chrono::{DateTime, SecondsFormat, Utc};
use openhuman_embed::{JobSchedule, JobSpec, Runtime};
use teeny_agent::runtime::{self as oh_runtime, RuntimeSettings};
use teeny_cadence::calendar::load_calendar;
use teeny_core::Config;

use crate::app::App;
use crate::schedule::{self, Dispatch, Handler, RuntimeScheduler, Scheduler};

/// Runs shown under each job.
const RECENT_RUNS: usize = 3;
/// Characters of a run's output shown.
const OUTPUT_SHOWN: usize = 160;

/// One scheduled job, as listed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobView {
    pub name: String,
    pub schedule: String,
    pub enabled: bool,
    pub next_run: Option<SystemTime>,
    pub last_run: Option<SystemTime>,
    pub last_status: Option<String>,
}

/// One past run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunView {
    pub status: String,
    pub output: Option<String>,
    pub started_at: SystemTime,
    pub finished_at: SystemTime,
}

/// The part of OpenHuman's cron these commands use.
#[async_trait::async_trait]
pub trait CronJobs: Send + Sync {
    fn list(&self) -> anyhow::Result<Vec<JobView>>;
    /// The newest `limit` runs of `name`, newest first.
    fn runs(&self, name: &str, limit: usize) -> anyhow::Result<Vec<RunView>>;
    /// Runs `name` now and waits: whether it succeeded, and its output.
    async fn run_now(&self, name: &str) -> anyhow::Result<(bool, String)>;
}

/// `teeny cron <list|run>`
#[derive(Debug, Clone, PartialEq, Eq, clap::Subcommand)]
pub enum CronCommand {
    /// Every registered job, its schedule and time zone, and its last runs.
    List,
    /// Run one job now. Refused unless DRY_RUN is on or --yes is given.
    Run {
        /// The job's name, e.g. teeny_morning.
        name: String,
        /// Run it even though DRY_RUN is off, so it may publish.
        #[arg(long)]
        yes: bool,
    },
}

/// `teeny cron <command>`
#[derive(Debug, Clone, PartialEq, Eq, clap::Args)]
pub struct CronArgs {
    #[command(subcommand)]
    pub command: CronCommand,
}

pub async fn run(
    cron: &dyn CronJobs,
    command: &CronCommand,
    dry_run: bool,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    match command {
        CronCommand::List => {
            let jobs = cron.list()?;
            if jobs.is_empty() {
                writeln!(out, "No jobs are scheduled.")?;
            }
            for job in jobs {
                writeln!(out, "{}", job.name)?;
                writeln!(
                    out,
                    "  {}  {}  next {}  last {} {}",
                    job.schedule,
                    if job.enabled { "enabled" } else { "disabled" },
                    when(job.next_run),
                    job.last_status.as_deref().unwrap_or("never"),
                    when(job.last_run),
                )?;
                for past in cron.runs(&job.name, RECENT_RUNS)? {
                    let output: String = past
                        .output
                        .as_deref()
                        .unwrap_or("")
                        .lines()
                        .next()
                        .unwrap_or("")
                        .chars()
                        .take(OUTPUT_SHOWN)
                        .collect();
                    writeln!(
                        out,
                        "    {}  {}  {output}",
                        when(Some(past.started_at)),
                        past.status
                    )?;
                }
            }
            Ok(())
        }
        CronCommand::Run { name, yes } => {
            if !dry_run && !yes {
                bail!(
                    "refusing to run {name} for real while DRY_RUN is off; pass --yes to run it anyway"
                );
            }
            let mode = if dry_run { " (DRY_RUN)" } else { "" };
            writeln!(out, "Running {name}{mode} ...")?;
            let (success, output) = cron.run_now(name).await?;
            writeln!(out, "{output}")?;
            if !success {
                bail!("{name} failed");
            }
            Ok(())
        }
    }
}

/// The command, from the environment.
pub async fn execute(args: &CronArgs) -> anyhow::Result<()> {
    let config = Config::from_env()?;
    match &args.command {
        CronCommand::List => {
            let runtime =
                oh_runtime::global(RuntimeSettings::from_config(&config.openhuman)).await?;
            run(
                &RuntimeCron(runtime),
                &args.command,
                config.dry_run,
                &mut std::io::stdout(),
            )
            .await
        }
        CronCommand::Run { .. } => {
            let dry_run = config.dry_run;
            let calendar = config
                .calendar
                .enabled
                .then(|| load_calendar(&config.calendar.path))
                .transpose()?;
            let app = App::build(config).await?;
            let jobs = schedule::job_table(&schedule::for_role(&app.config), calendar.as_ref())?;
            schedule::install(
                &HandlersOnly(RuntimeScheduler(&app.runtime)),
                &jobs,
                Arc::new(Dispatch::new(Arc::clone(&app.ctx))),
                tokio::runtime::Handle::current(),
            )?;
            run(
                &RuntimeCron(Arc::clone(&app.runtime)),
                &args.command,
                dry_run,
                &mut std::io::stdout(),
            )
            .await
        }
    }
}

/// The runtime's cron.
pub struct RuntimeCron(pub Arc<Runtime>);

#[async_trait::async_trait]
impl CronJobs for RuntimeCron {
    fn list(&self) -> anyhow::Result<Vec<JobView>> {
        Ok(self
            .0
            .cron()
            .list()?
            .into_iter()
            .map(|job| JobView {
                schedule: describe(&job.schedule),
                name: job.name,
                enabled: job.enabled,
                next_run: Some(job.next_run),
                last_run: job.last_run,
                last_status: job.last_status,
            })
            .collect())
    }

    fn runs(&self, name: &str, limit: usize) -> anyhow::Result<Vec<RunView>> {
        Ok(self
            .0
            .cron()
            .runs(name, limit)?
            .into_iter()
            .map(|past| RunView {
                status: past.status,
                output: past.output,
                started_at: past.started_at,
                finished_at: past.finished_at,
            })
            .collect())
    }

    async fn run_now(&self, name: &str) -> anyhow::Result<(bool, String)> {
        let ran = self.0.cron().run_now(name).await?;
        Ok((ran.success, ran.output))
    }
}

/// Registers the handlers and leaves the job store alone: the service that
/// owns the schedule upserts and prunes it.
struct HandlersOnly<'a>(RuntimeScheduler<'a>);

impl Scheduler for HandlersOnly<'_> {
    fn on_job(&self, name: &str, handler: Handler) -> anyhow::Result<()> {
        self.0.on_job(name, handler)
    }

    fn upsert(&self, _spec: JobSpec) -> anyhow::Result<()> {
        Ok(())
    }

    fn names(&self) -> anyhow::Result<Vec<String>> {
        Ok(Vec::new())
    }

    fn remove(&self, _name: &str) -> anyhow::Result<()> {
        Ok(())
    }
}

fn describe(schedule: &JobSchedule) -> String {
    match schedule {
        JobSchedule::Cron { expr, tz } => format!("{expr} ({})", tz.as_deref().unwrap_or("UTC")),
        JobSchedule::Every { ms } => format!("every {}s", ms / 1000),
        JobSchedule::At { at } => format!("once at {}", when(Some(*at))),
    }
}

fn when(at: Option<SystemTime>) -> String {
    at.map(|at| DateTime::<Utc>::from(at).to_rfc3339_opts(SecondsFormat::Secs, true))
        .unwrap_or_else(|| "-".to_owned())
}

#[cfg(test)]
#[path = "cron_tests.rs"]
mod tests;
