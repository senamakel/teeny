//! A loopback relay to the shared store.
//!
//! TinyMemory's CortexDB engine refuses cleartext HTTP to any host that is
//! not loopback, since it sends a bearer. The dedicated store is a container
//! on the stack's own network (`http://teeny-cortexdb:3141`), with no TLS in
//! front of it, which is how the legacy store was always reached too. Rather
//! than weaken that check, the runtime gives the engine a loopback address
//! ([`engine_endpoint`]) and relays it, byte for byte, to the store:
//! the bearer travels exactly as far as it did before, over the private
//! network, and the engine's check still holds for anything else.
//!
//! The relay listens on `127.0.0.1` only, inside the bot's container, and
//! resolves the store's name on every connection, so a recreated store is
//! found at its new address.

use std::net::{IpAddr, SocketAddr};

use reqwest::Url;
use tokio::net::{TcpListener, TcpStream};

/// The endpoint to hand the engine for `url`: `url` itself when it is
/// HTTPS or already loopback, otherwise a loopback relay to it, with the
/// path kept.
pub async fn engine_endpoint(url: &str) -> std::io::Result<String> {
    let parsed = Url::parse(url).map_err(|error| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("TEENY_MEMORY_URL {url:?}: {error}"),
        )
    })?;
    if parsed.scheme() != "http" || is_loopback(&parsed) {
        return Ok(url.to_owned());
    }
    let host = parsed.host_str().unwrap_or_default();
    let port = parsed.port_or_known_default().unwrap_or(80);
    let local = relay(format!("{host}:{port}")).await?;
    let mut relayed = format!("http://{local}{}", parsed.path());
    if let Some(query) = parsed.query() {
        relayed.push('?');
        relayed.push_str(query);
    }
    tracing::info!(store = %host, port, relay = %local, "[memory] relaying the shared store from loopback");
    Ok(relayed)
}

fn is_loopback(url: &Url) -> bool {
    let host = url.host_str().unwrap_or_default();
    host.eq_ignore_ascii_case("localhost")
        || host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

/// Listens on a free loopback port and relays every connection to
/// `target` (`host:port`), for the life of the tokio runtime it is called
/// on. Returns the address it listens on.
pub async fn relay(target: String) -> std::io::Result<SocketAddr> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let local = listener.local_addr()?;
    tokio::spawn(async move {
        loop {
            let (mut inbound, _) = match listener.accept().await {
                Ok(accepted) => accepted,
                Err(error) => {
                    tracing::warn!(%error, "[memory] relay accept failed");
                    continue;
                }
            };
            let target = target.clone();
            tokio::spawn(async move {
                match TcpStream::connect(&target).await {
                    Ok(mut outbound) => {
                        let _ = tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await;
                    }
                    Err(error) => {
                        tracing::warn!(%error, %target, "[memory] the shared store is unreachable");
                    }
                }
            });
        }
    });
    Ok(local)
}

#[cfg(test)]
#[path = "memory_relay_tests.rs"]
mod tests;
