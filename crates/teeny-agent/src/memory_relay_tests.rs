use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::*;

#[tokio::test]
async fn an_https_or_loopback_store_is_used_as_it_is() {
    for url in [
        "https://store.example",
        "http://127.0.0.1:3143",
        "http://localhost:3143/",
        "http://[::1]:3143",
    ] {
        assert_eq!(engine_endpoint(url).await.unwrap(), url);
    }
    assert!(engine_endpoint("not a url").await.is_err());
}

#[tokio::test]
async fn a_store_on_the_private_network_is_reached_through_a_loopback_relay() {
    // The "store": answers whatever it is sent, upper-cased.
    let store = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = store.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let (mut socket, _) = store.accept().await.unwrap();
            tokio::spawn(async move {
                let mut buffer = [0u8; 64];
                let read = socket.read(&mut buffer).await.unwrap();
                let answer = buffer[..read].to_ascii_uppercase();
                socket.write_all(&answer).await.unwrap();
            });
        }
    });

    // Named the way compose names it: not loopback, so relayed.
    let endpoint = engine_endpoint(&format!("http://store.invalid:{port}/base"))
        .await
        .unwrap();
    let relayed: Url = endpoint.parse().unwrap();
    assert_eq!(relayed.host_str(), Some("127.0.0.1"));
    assert_eq!(relayed.path(), "/base");
    assert_ne!(relayed.port(), Some(port));

    // The relay itself, to an address that resolves here.
    let local = relay(format!("127.0.0.1:{port}")).await.unwrap();
    assert!(local.ip().is_loopback());
    for message in ["gm", "wen chart"] {
        let mut client = TcpStream::connect(local).await.unwrap();
        client.write_all(message.as_bytes()).await.unwrap();
        let mut answer = Vec::new();
        client.read_to_end(&mut answer).await.unwrap();
        assert_eq!(answer, message.to_uppercase().as_bytes());
    }
}
