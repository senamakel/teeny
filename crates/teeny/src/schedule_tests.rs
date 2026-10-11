use std::sync::{Arc, Mutex};

use openhuman_embed::{JobSchedule, JobSpec};
use teeny_agent::deps::fake_vars;
use teeny_cadence::calendar::parse_calendar;
use teeny_core::Config;
use teeny_jobs::ctx::JobCtx;
use teeny_jobs::runner::ScriptedRunner;

use super::*;

fn config(extra: &[(&'static str, &'static str)]) -> Config {
    let mut vars = fake_vars();
    vars.retain(|(key, _)| !extra.iter().any(|(k, _)| k == key));
    vars.extend_from_slice(extra);
    Config::from_vars(vars).unwrap()
}

fn table(config: &Config) -> Vec<(String, String, Option<String>)> {
    job_table(&for_role(config), None)
        .unwrap()
        .into_iter()
        .map(|job| match job.schedule {
            JobSchedule::Cron { expr, tz } => (job.name, expr, tz),
            other => panic!("{other:?}"),
        })
        .collect()
}

fn names(config: &Config) -> Vec<String> {
    table(config).into_iter().map(|(name, _, _)| name).collect()
}

const CALENDAR: &str = r#"{
  "timezone": "Europe/Berlin",
  "items": [
    { "name": "gm", "cron": "0 7 * * *", "channels": ["x"], "brief": "A good-morning post, warm." },
    { "name": "ship-friday", "cron": "0 16 * * 5", "channels": ["x"], "brief": "Friday, building in the open." },
    { "name": "weekdays", "cron": "30 9 * * 1-5", "channels": ["telegram"], "brief": "A weekday note to the group." },
    { "name": "off", "cron": "0 9 * * *", "channels": ["x"], "brief": "Switched off for now.", "enabled": false }
  ]
}"#;

#[test]
fn every_job_of_the_default_role_on_its_configured_cron() {
    let everything = config(&[
        ("MORNING_ENABLED", "true"),
        ("QUOTE_ENABLED", "true"),
        ("LORE_ENABLED", "true"),
        ("BRAIN_DATA_DIR", "brain"),
        ("LEARNINGS_ENABLED", "true"),
    ]);
    assert_eq!(
        table(&everything),
        vec![
            ("teeny_ingest".into(), "*/5 * * * *".into(), None),
            ("teeny_brain_usage".into(), "* * * * *".into(), None),
            ("teeny_heat".into(), "*/20 * * * *".into(), None),
            ("teeny_cortex_health".into(), "17 * * * *".into(), None),
            ("teeny_learnings".into(), "0 5 * * *".into(), None),
            ("teeny_morning".into(), "0 7 * * *".into(), None),
            ("teeny_afternoon".into(), "0 15 * * *".into(), None),
            ("teeny_evening".into(), "0 23 * * *".into(), None),
        ]
    );
}

#[test]
fn the_publishing_jobs_are_off_until_switched_on_and_the_reply_worker_is_not_a_cron() {
    // The defaults: no publishing job is on, the brain is a directory that
    // may be absent (its fold declines then), and REPLY_MODE=worker drains
    // the queue in a loop rather than on a timer. The store's health is read
    // in every process, and the brief is on by default.
    assert_eq!(
        names(&config(&[("LEARNINGS_ENABLED", "true")])),
        [
            "teeny_ingest",
            "teeny_brain_usage",
            "teeny_heat",
            "teeny_cortex_health",
            "teeny_learnings"
        ]
    );
    assert_eq!(
        names(&config(&[
            ("BRAIN_DATA_DIR", ""),
            ("HEAT_ENABLED", "false"),
            ("LEARNINGS_ENABLED", "false"),
        ])),
        ["teeny_ingest", "teeny_cortex_health"]
    );
    let cron = table(&config(&[
        ("REPLY_MODE", "cron"),
        ("REPLY_CRON", "*/2 * * * *"),
    ]));
    assert!(cron.contains(&("teeny_reply".into(), "*/2 * * * *".into(), None)));
}

#[test]
fn a_single_role_registers_only_its_own_jobs() {
    let on = [
        ("MORNING_ENABLED", "true"),
        ("MOON_ENABLED", "true"),
        ("LORE_ENABLED", "true"),
        ("NARRATIVE_ENABLED", "true"),
        ("QUOTE_ENABLED", "true"),
        ("LEARNINGS_ENABLED", "true"),
    ];
    // Every process reads the store's health; the rest is the role's own.
    let role = |role: &'static str| {
        let mut vars = on.to_vec();
        vars.push(("SERVICE_ROLE", role));
        let mut names = names(&config(&vars));
        assert!(names.contains(&"teeny_cortex_health".to_owned()), "{role}");
        names.retain(|name| name != "teeny_cortex_health");
        names
    };
    assert_eq!(role("ingest"), ["teeny_ingest", "teeny_brain_usage"]);
    assert_eq!(role("reply"), ["teeny_heat"]);
    // One writer of the brief, the morning's, as every process reads it.
    assert_eq!(role("morning"), ["teeny_learnings", "teeny_morning"]);
    // The slots are dispatchers; a single role only dispatches to its own job.
    assert_eq!(role("quote"), ["teeny_evening"]);
    assert_eq!(role("moon"), ["teeny_afternoon"]);
    assert_eq!(role("lore"), ["teeny_afternoon"]);
    assert_eq!(role("narrative"), ["teeny_evening"]);
    assert!(role("api").is_empty());
    assert!(role("chart").is_empty(), "the chart is off");

    let moon = for_role(&config(&[("SERVICE_ROLE", "moon"), on[1], on[2]]));
    assert!(moon.moon.enabled);
    assert!(
        !moon.lore.enabled,
        "the lore role's job, not this process's"
    );
    assert!(!moon.cadence.heat_enabled);
    assert!(!moon.learnings.enabled, "the morning role writes the brief");
}

#[test]
fn each_slot_is_one_cron_whatever_owns_it_and_the_old_per_job_crons_are_ignored() {
    let chart_first = config(&[
        ("CHART_ENABLED", "true"),
        ("CHART_CRON", "0 14 * * *"),
        ("LORE_ENABLED", "true"),
    ]);
    assert!(table(&chart_first).contains(&("teeny_afternoon".into(), "0 15 * * *".into(), None)));
    let moved = config(&[
        ("QUOTE_ENABLED", "true"),
        ("QUOTE_CRON", "0 1 * * *"),
        ("AFTERNOON_CRON", "0 16 * * *"),
        ("EVENING_CRON", "0 22 * * *"),
        ("MOON_ENABLED", "true"),
    ]);
    let moved = table(&moved);
    assert!(moved.contains(&("teeny_afternoon".into(), "0 16 * * *".into(), None)));
    assert!(moved.contains(&("teeny_evening".into(), "0 22 * * *".into(), None)));
    assert!(!moved.iter().any(|(name, _, _)| name == "teeny_quote"));
}

#[test]
fn reads_every_cron_in_calendar_tz() {
    let berlin = config(&[
        ("CALENDAR_TZ", "Europe/Berlin"),
        ("MORNING_ENABLED", "true"),
    ]);
    assert!(
        table(&berlin)
            .iter()
            .all(|(_, _, tz)| tz.as_deref() == Some("Europe/Berlin"))
    );
    let wrong = config(&[("CALENDAR_TZ", "Mars/Olympus")]);
    let refused = job_table(&wrong, None).unwrap_err();
    assert!(refused.to_string().contains("CALENDAR_TZ"), "{refused}");
}

#[test]
fn one_job_per_enabled_calendar_item_with_named_weekdays_in_the_calendars_zone() {
    let calendar = parse_calendar(CALENDAR).unwrap();
    let on = for_role(&config(&[
        ("CALENDAR_ENABLED", "true"),
        ("SERVICE_ROLE", "calendar"),
    ]));
    let jobs: Vec<_> = job_table(&on, Some(&calendar))
        .unwrap()
        .into_iter()
        .filter(|job| job.work != Work::CortexHealth)
        .map(|job| (job.name, job.schedule))
        .collect();
    let berlin = |expr: &str| JobSchedule::Cron {
        expr: expr.into(),
        tz: Some("Europe/Berlin".into()),
    };
    assert_eq!(
        jobs,
        vec![
            ("teeny_calendar_gm".into(), berlin("0 7 * * *")),
            ("teeny_calendar_ship-friday".into(), berlin("0 16 * * FRI")),
            ("teeny_calendar_weekdays".into(), berlin("30 9 * * MON-FRI")),
        ]
    );

    // The morning job owns the good morning when this process runs it.
    let both = for_role(&config(&[
        ("CALENDAR_ENABLED", "true"),
        ("MORNING_ENABLED", "true"),
    ]));
    let names: Vec<_> = job_table(&both, Some(&calendar))
        .unwrap()
        .into_iter()
        .map(|job| job.name)
        .collect();
    assert!(names.contains(&"teeny_morning".to_owned()));
    assert!(!names.contains(&"teeny_calendar_gm".to_owned()));
    assert!(names.contains(&"teeny_calendar_ship-friday".to_owned()));
}

/// Records what the installer asks of the scheduler.
#[derive(Default)]
struct FakeScheduler {
    stored: Mutex<Vec<String>>,
    handlers: Mutex<Vec<String>>,
    upserts: Mutex<Vec<JobSpec>>,
    removed: Mutex<Vec<String>>,
    /// Every call, in order; shared with the start future.
    events: Arc<Mutex<Vec<String>>>,
}

impl Scheduler for FakeScheduler {
    fn on_job(&self, name: &str, _handler: Handler) -> anyhow::Result<()> {
        self.handlers.lock().unwrap().push(name.to_owned());
        self.events.lock().unwrap().push(format!("handler {name}"));
        Ok(())
    }

    fn upsert(&self, spec: JobSpec) -> anyhow::Result<()> {
        assert!(
            self.handlers.lock().unwrap().contains(&spec.name),
            "a handler is in place before its job can fire"
        );
        self.stored.lock().unwrap().push(spec.name.clone());
        self.events
            .lock()
            .unwrap()
            .push(format!("upsert {}", spec.name));
        self.upserts.lock().unwrap().push(spec);
        Ok(())
    }

    fn names(&self) -> anyhow::Result<Vec<String>> {
        Ok(self.stored.lock().unwrap().clone())
    }

    fn remove(&self, name: &str) -> anyhow::Result<()> {
        self.stored.lock().unwrap().retain(|stored| stored != name);
        self.removed.lock().unwrap().push(name.to_owned());
        self.events.lock().unwrap().push(format!("remove {name}"));
        Ok(())
    }

    fn start(&self) -> BoxFuture<'static, ()> {
        let events = Arc::clone(&self.events);
        Box::pin(async move { events.lock().unwrap().push("start".into()) })
    }
}

#[tokio::test]
async fn installs_each_job_once_and_unretried_and_drops_stale_teeny_jobs() {
    let scheduler = FakeScheduler::default();
    scheduler.stored.lock().unwrap().extend([
        "teeny_ingest".to_owned(),
        "teeny_calendar_retired".to_owned(),
        "someone_elses".to_owned(),
    ]);
    let jobs = job_table(&for_role(&config(&[])), None).unwrap();
    let dispatch = Arc::new(Dispatch::new(Arc::new(JobCtx::fake(Arc::new(
        ScriptedRunner::new(),
    )))));

    install(
        &scheduler,
        &jobs,
        dispatch,
        tokio::runtime::Handle::current(),
    )
    .unwrap();

    let upserts = scheduler.upserts.lock().unwrap();
    assert_eq!(upserts.len(), jobs.len());
    for spec in upserts.iter() {
        assert_eq!(spec.retries, Some(0), "{}", spec.name);
        assert!(spec.single_flight, "{}", spec.name);
    }
    assert_eq!(
        *scheduler.removed.lock().unwrap(),
        ["teeny_calendar_retired"]
    );
    assert!(
        scheduler
            .stored
            .lock()
            .unwrap()
            .contains(&"someone_elses".to_owned())
    );
}

#[tokio::test]
async fn boot_starts_cron_only_after_every_handler_job_and_prune() {
    let scheduler = FakeScheduler::default();
    scheduler
        .stored
        .lock()
        .unwrap()
        .push("teeny_calendar_retired".to_owned());
    let jobs = job_table(&for_role(&config(&[])), None).unwrap();
    let dispatch = Arc::new(Dispatch::new(Arc::new(JobCtx::fake(Arc::new(
        ScriptedRunner::new(),
    )))));

    boot(
        &scheduler,
        &jobs,
        dispatch,
        tokio::runtime::Handle::current(),
    )
    .await
    .unwrap();

    let mut expected: Vec<String> = jobs
        .iter()
        .flat_map(|job| {
            [
                format!("handler {}", job.name),
                format!("upsert {}", job.name),
            ]
        })
        .collect();
    expected.push("remove teeny_calendar_retired".into());
    expected.push("start".into());
    assert_eq!(*scheduler.events.lock().unwrap(), expected);
}

#[tokio::test]
async fn a_failed_install_never_starts_cron() {
    struct Refusing(FakeScheduler);
    impl Scheduler for Refusing {
        fn on_job(&self, name: &str, handler: Handler) -> anyhow::Result<()> {
            self.0.on_job(name, handler)
        }
        fn upsert(&self, _spec: JobSpec) -> anyhow::Result<()> {
            anyhow::bail!("the store is read-only")
        }
        fn names(&self) -> anyhow::Result<Vec<String>> {
            self.0.names()
        }
        fn remove(&self, name: &str) -> anyhow::Result<()> {
            self.0.remove(name)
        }
        fn start(&self) -> BoxFuture<'static, ()> {
            self.0.start()
        }
    }
    let scheduler = Refusing(FakeScheduler::default());
    let jobs = job_table(&for_role(&config(&[])), None).unwrap();
    let dispatch = Arc::new(Dispatch::new(Arc::new(JobCtx::fake(Arc::new(
        ScriptedRunner::new(),
    )))));
    let booted = boot(
        &scheduler,
        &jobs,
        dispatch,
        tokio::runtime::Handle::current(),
    )
    .await;
    assert!(booted.is_err());
    assert!(
        !scheduler
            .0
            .events
            .lock()
            .unwrap()
            .contains(&"start".to_owned())
    );
}

#[tokio::test]
async fn a_run_reports_a_failed_job_as_an_error_for_cron_to_record() {
    let wired = Dispatch::new(Arc::new(JobCtx::fake(Arc::new(ScriptedRunner::new()))));
    assert_eq!(wired.run(&Work::Ingest).await, Ok(()));
    assert_eq!(
        wired.run(&Work::BrainUsage).await,
        Ok(()),
        "no brain declines"
    );

    // The conversation side is not wired: the poll cannot run.
    let mut bare = JobCtx::fake(Arc::new(ScriptedRunner::new()));
    bare.conversation = None;
    let failed = Dispatch::new(Arc::new(bare)).run(&Work::Ingest).await;
    assert!(failed.unwrap_err().contains("not wired"));
}

/// Boots OpenHuman on a temp workspace (no network: the backend points at a
/// closed port), installs the default table and a probe, lists them, and
/// runs the probe now through the same path a scheduled run takes.
#[test]
#[ignore = "boots the OpenHuman runtime on a temp workspace"]
fn installs_on_the_real_runtime_and_runs_a_job_now() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use futures::FutureExt as _;
    use teeny_agent::runtime::{self, RuntimeSettings};

    let executor = runtime::executor();
    executor
        .block_on(executor.spawn(async {
            let data_dir =
                std::env::temp_dir().join(format!("teeny-schedule-{}", std::process::id()));
            let runtime = runtime::global(RuntimeSettings {
                data_dir: data_dir.clone(),
                chrome_path: None,
                unattended_actions: vec![],
                allowed_domains: vec![],
                tinycomputer_module: None,
                backend_url: Some("http://127.0.0.1:9".into()),
            })
            .await
            .expect("the runtime boots");
            let scheduler = RuntimeScheduler(&runtime);

            let jobs = job_table(
                &for_role(&config(&[("CALENDAR_TZ", "Europe/Berlin")])),
                None,
            )
            .unwrap();
            let dispatch = Arc::new(Dispatch::new(Arc::new(JobCtx::fake(Arc::new(
                ScriptedRunner::new(),
            )))));
            install(
                &scheduler,
                &jobs,
                dispatch,
                tokio::runtime::Handle::current(),
            )
            .unwrap();

            let ran = Arc::new(AtomicUsize::new(0));
            let counter = Arc::clone(&ran);
            scheduler
                .on_job(
                    "teeny_probe",
                    Arc::new(move || {
                        counter.fetch_add(1, Ordering::SeqCst);
                        async { Ok(()) }.boxed()
                    }),
                )
                .unwrap();
            scheduler
                .upsert(
                    JobSpec::system(
                        "teeny_probe",
                        "teeny_probe",
                        JobSchedule::Cron {
                            expr: "0 0 1 1 *".into(),
                            tz: Some("Europe/Berlin".into()),
                        },
                    )
                    .retries(0)
                    .single_flight(true),
                )
                .unwrap();

            let listed = runtime.cron().list().unwrap();
            for job in &jobs {
                let stored = listed
                    .iter()
                    .find(|stored| stored.name == job.name)
                    .unwrap_or_else(|| panic!("{} is stored", job.name));
                assert_eq!(stored.schedule, job.schedule);
                assert_eq!(stored.retries, Some(0));
                assert!(stored.single_flight);
            }
            let run = runtime.cron().run_now("teeny_probe").await.unwrap();
            assert!(run.success, "{}", run.output);
            assert_eq!(ran.load(Ordering::SeqCst), 1);
            assert_eq!(
                runtime.cron().runs("teeny_probe", 5).unwrap()[0].status,
                "ok"
            );

            // A second boot with the probe gone from the table removes it.
            install(
                &scheduler,
                &jobs,
                Arc::new(Dispatch::new(Arc::new(JobCtx::fake(Arc::new(
                    ScriptedRunner::new(),
                ))))),
                tokio::runtime::Handle::current(),
            )
            .unwrap();
            assert!(
                !scheduler
                    .names()
                    .unwrap()
                    .contains(&"teeny_probe".to_owned())
            );
            // The workspace is left in place: the runtime is the process's
            // one, and the other runtime test may still be using it.
            let _ = data_dir;
        }))
        .unwrap();
}

/// The boot-order gap, on the real runtime: a system job that falls due
/// before cron is started must not be dispatched (nor recorded "ok" with no
/// handler); once [`boot`] starts cron it fires its handler exactly once.
#[test]
#[ignore = "boots the OpenHuman runtime on a temp workspace and waits on cron"]
fn a_job_due_before_the_scheduler_starts_fires_once_after_boot() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, SystemTime};

    use futures::FutureExt as _;
    use teeny_agent::runtime::{self, RuntimeSettings};

    let executor = runtime::executor();
    executor
        .block_on(executor.spawn(async {
            let data_dir =
                std::env::temp_dir().join(format!("teeny-schedule-{}", std::process::id()));
            let runtime = runtime::global(RuntimeSettings {
                data_dir: data_dir.clone(),
                chrome_path: None,
                unattended_actions: vec![],
                allowed_domains: vec![],
                tinycomputer_module: None,
                backend_url: Some("http://127.0.0.1:9".into()),
            })
            .await
            .expect("the runtime boots");

            let ran = Arc::new(AtomicUsize::new(0));
            let counter = Arc::clone(&ran);
            let scheduler = RuntimeScheduler(&runtime);
            // Stored before its handler exists, as a job from a previous
            // process is: due one second from now. Not `teeny_`-named, so
            // the other runtime test's prune leaves it alone.
            scheduler
                .upsert(
                    JobSpec::system(
                        "boot_order_probe",
                        "boot_order_probe",
                        JobSchedule::At {
                            at: SystemTime::now() + Duration::from_secs(1),
                        },
                    )
                    .retries(0)
                    .single_flight(true),
                )
                .unwrap();
            // Past due and past a scheduler poll (15 s): nothing may have run it.
            tokio::time::sleep(Duration::from_secs(20)).await;
            assert!(
                runtime
                    .cron()
                    .runs("boot_order_probe", 5)
                    .unwrap()
                    .is_empty(),
                "a due job was dispatched before cron was started"
            );

            scheduler
                .on_job(
                    "boot_order_probe",
                    Arc::new(move || {
                        counter.fetch_add(1, Ordering::SeqCst);
                        async { Ok(()) }.boxed()
                    }),
                )
                .unwrap();
            assert_eq!(ran.load(Ordering::SeqCst), 0);
            // What [`boot`] does last.
            scheduler.start().await;

            for _ in 0..80 {
                if ran.load(Ordering::SeqCst) > 0 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            tokio::time::sleep(Duration::from_secs(7)).await;
            assert_eq!(ran.load(Ordering::SeqCst), 1, "fires exactly once");
            runtime.stop_services();
            // The workspace is left in place: the runtime is the process's
            // one, and the other runtime test may still be using it.
            let _ = data_dir;
        }))
        .unwrap();
}
