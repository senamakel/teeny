use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use super::*;
use crate::cmd::test_support::printed;

#[derive(Default)]
struct FakeCron {
    ran: Mutex<Vec<String>>,
    fails: bool,
}

fn at(seconds: u64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)
}

#[async_trait::async_trait]
impl CronJobs for FakeCron {
    fn list(&self) -> anyhow::Result<Vec<JobView>> {
        Ok(vec![JobView {
            name: "teeny_morning".into(),
            schedule: "0 7 * * * (Europe/Berlin)".into(),
            enabled: true,
            next_run: Some(at(1_790_000_000)),
            last_run: Some(at(1_789_900_000)),
            last_status: Some("ok".into()),
        }])
    }

    fn runs(&self, name: &str, _limit: usize) -> anyhow::Result<Vec<RunView>> {
        assert_eq!(name, "teeny_morning");
        Ok(vec![RunView {
            status: "error".into(),
            output: Some("the morning post: no image".into()),
            started_at: at(1_789_900_000),
            finished_at: at(1_789_900_042),
        }])
    }

    async fn run_now(&self, name: &str) -> anyhow::Result<(bool, String)> {
        self.ran.lock().unwrap().push(name.to_owned());
        Ok((!self.fails, "done".into()))
    }
}

#[tokio::test]
async fn list_prints_each_job_its_schedule_and_its_last_runs() {
    let cron = FakeCron::default();
    let mut out = Vec::new();

    run(&cron, &CronCommand::List, false, &mut out)
        .await
        .unwrap();

    let printed = printed(out);
    assert!(printed.contains("teeny_morning"), "{printed}");
    assert!(printed.contains("0 7 * * * (Europe/Berlin)"), "{printed}");
    assert!(printed.contains("last ok"), "{printed}");
    assert!(
        printed.contains("error  the morning post: no image"),
        "{printed}"
    );
}

#[tokio::test]
async fn run_refuses_a_live_run_without_yes() {
    let cron = FakeCron::default();

    let error = run(
        &cron,
        &CronCommand::Run {
            name: "teeny_morning".into(),
            yes: false,
        },
        false,
        &mut Vec::new(),
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("--yes"), "{error}");
    assert!(cron.ran.lock().unwrap().is_empty());
}

#[tokio::test]
async fn run_goes_ahead_under_dry_run_or_with_yes() {
    for (dry_run, yes) in [(true, false), (false, true)] {
        let cron = FakeCron::default();
        let mut out = Vec::new();

        run(
            &cron,
            &CronCommand::Run {
                name: "teeny_morning".into(),
                yes,
            },
            dry_run,
            &mut out,
        )
        .await
        .unwrap();

        assert_eq!(*cron.ran.lock().unwrap(), vec!["teeny_morning".to_owned()]);
        assert!(printed(out).contains("done"));
    }
}

#[tokio::test]
async fn a_failed_run_is_an_error() {
    let cron = FakeCron {
        fails: true,
        ..FakeCron::default()
    };

    let error = run(
        &cron,
        &CronCommand::Run {
            name: "teeny_morning".into(),
            yes: true,
        },
        false,
        &mut Vec::new(),
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("teeny_morning"), "{error}");
}
