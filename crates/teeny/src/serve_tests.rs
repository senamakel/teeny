use std::time::Duration;

use tokio::net::TcpListener;
use tokio::sync::oneshot;

use super::*;
use crate::healthcheck::check;
use crate::http::router;
use crate::http::test_support::services;

#[tokio::test]
async fn serves_until_told_to_stop_and_passes_its_own_healthcheck() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, stopped) = oneshot::channel::<()>();
    let server = tokio::spawn(serve_on(listener, router(services()), async {
        let _ = stopped.await;
    }));

    check(&format!("http://{address}/health")).await.unwrap();

    stop.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .expect("stopped")
        .unwrap()
        .unwrap();
    assert!(check(&format!("http://{address}/health")).await.is_err());
}

#[test]
fn a_dry_run_serve_forces_both_rehearsal_switches() {
    let mut vars = teeny_agent::deps::fake_vars();
    vars.retain(|(key, _)| *key != "DRY_RUN");
    vars.extend([("DRY_RUN", "false"), ("TELEGRAM_DRY_RUN", "false")]);
    let live = teeny_core::Config::from_vars(vars).unwrap();
    assert!(!live.dry_run && !live.telegram.dry_run);

    let rehearsal = force_dry_run(live.clone());
    assert!(rehearsal.dry_run);
    assert!(rehearsal.telegram.dry_run);
}

#[tokio::test]
async fn stops_the_schedule_and_the_transports_then_the_server_then_the_stream() {
    use std::sync::{Arc, Mutex};

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let health = format!("http://{}/health", listener.local_addr().unwrap());
    let (stop, stopped) = oneshot::channel::<()>();
    let log = Arc::new(Mutex::new(Vec::<String>::new()));

    let (before_log, before_health) = (Arc::clone(&log), health.clone());
    let before = async move {
        // The server still answers while the schedule and transports stop.
        let up = check(&before_health).await.is_ok();
        before_log
            .lock()
            .unwrap()
            .push(format!("schedule stopped, server up: {up}"));
    };
    let (after_log, after_health) = (Arc::clone(&log), health.clone());
    let after = async move {
        let up = check(&after_health).await.is_ok();
        after_log
            .lock()
            .unwrap()
            .push(format!("stream stopped, server up: {up}"));
    };
    let server = tokio::spawn(serve_until(
        listener,
        router(services()),
        async {
            let _ = stopped.await;
        },
        before,
        after,
    ));
    check(&health).await.unwrap();
    stop.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .expect("stopped")
        .unwrap()
        .unwrap();

    assert_eq!(
        *log.lock().unwrap(),
        [
            "schedule stopped, server up: true",
            "stream stopped, server up: false"
        ]
    );
}
