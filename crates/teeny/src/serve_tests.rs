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
