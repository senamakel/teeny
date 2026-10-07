//! Fetching an image a stranger pointed at, without letting the stranger
//! point this server at its own network.
//!
//! The URL guard ([`is_fetchable_image_url`]) reads only the string, and a
//! string can lie: a public name can resolve to `10.0.0.5`, or answer with a
//! redirect to the metadata endpoint. So the fetcher has a client of its own
//! that
//!
//! - re-runs the URL guard on every redirect, and follows at most three;
//! - resolves names itself and refuses a name if any address it resolves to
//!   is private, loopback, link-local, carrier-grade NAT, unique-local,
//!   multicast or otherwise not on the public internet. The connection goes
//!   to the addresses vetted here, so DNS rebinding cannot swap them;
//! - ignores proxy settings, which would resolve the name somewhere else;
//! - stops reading past [`MAX_IMAGE_BYTES`].
//!
//! What goes back to the model is generic: it is a stranger's tool call, and
//! the details (an internal status, a content type, where a redirect went)
//! are logged instead.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use reqwest::redirect;
use url::{Origin, Url};

use crate::error::{Error, Result};
use crate::types::MimeType;
use crate::vision::{MAX_IMAGE_BYTES, is_fetchable_image_url, sniff};

const FETCH_TIMEOUT: Duration = Duration::from_secs(20);
/// Redirects followed before giving up: enough for a CDN, not for a maze.
const MAX_REDIRECTS: usize = 3;

const REFUSED: &str = "refusing to fetch that URL: only public https image URLs are read";
const UNREACHABLE: &str = "could not fetch that image";
const NOT_AN_IMAGE: &str = "that URL did not return an image";

/// Whether an address is on the public internet: not private, loopback,
/// link-local, carrier-grade NAT, unique-local, multicast, reserved or a
/// documentation range, and not an IPv4 address in IPv6 clothing that is
/// one of those.
pub fn is_public_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_public_v4(ip),
        IpAddr::V6(ip) => is_public_v6(ip),
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_documentation()
        || ip.is_unspecified()
        || ip.is_multicast()
        || a == 0
        // Carrier-grade NAT, 100.64.0.0/10.
        || (a == 100 && (64..128).contains(&b))
        // IETF protocol assignments, 192.0.0.0/24.
        || (a == 192 && b == 0 && c == 0)
        // Benchmarking, 198.18.0.0/15.
        || (a == 198 && (18..20).contains(&b))
        // Reserved, 240.0.0.0/4.
        || a >= 240)
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return is_public_v4(v4);
    }
    let segments = ip.segments();
    // NAT64, 64:ff9b::/96: the IPv4 address is the last 32 bits.
    if segments[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
        let [.., high, low] = segments;
        return is_public_v4(Ipv4Addr::from((u32::from(high) << 16) | u32::from(low)));
    }
    !(ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        // IPv4-compatible, ::/96, deprecated and never public.
        || segments[..6] == [0; 6]
        // Unique local, fc00::/7.
        || (segments[0] & 0xfe00) == 0xfc00
        // Link-local, fe80::/10, and the old site-local, fec0::/10.
        || (segments[0] & 0xffc0) == 0xfe80
        || (segments[0] & 0xffc0) == 0xfec0
        // Documentation, 2001:db8::/32.
        || segments[..2] == [0x2001, 0xdb8])
}

/// Resolves names, refusing any that lead off the public internet.
pub(crate) struct PublicOnlyResolver;

impl Resolve for PublicOnlyResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_owned();
        Box::pin(async move {
            let addrs: Vec<SocketAddr> =
                tokio::net::lookup_host((host.as_str(), 0)).await?.collect();
            if addrs.is_empty() {
                return Err(format!("{host} resolves to nothing").into());
            }
            if let Some(refused) = addrs.iter().find(|addr| !is_public_address(addr.ip())) {
                return Err(
                    format!("{host} resolves to a non-public address {}", refused.ip()).into(),
                );
            }
            Ok(Box::new(addrs.into_iter()) as Addrs)
        })
    }
}

/// Downloads images from public https URLs, and from nowhere else however
/// the URL, its DNS or its redirects are dressed up.
#[derive(Debug, Clone)]
pub struct ImageFetcher {
    client: reqwest::Client,
    /// The one origin let past the guard: a test's loopback server. Only a
    /// test build can set it.
    trusted: Option<Origin>,
}

impl Default for ImageFetcher {
    fn default() -> Self {
        Self::new()
    }
}

impl ImageFetcher {
    /// The fetcher with every check on.
    pub fn new() -> Self {
        Self::with_trusted(None)
    }

    /// A fetcher that also lets `origin` (a wiremock server on loopback)
    /// through: the guard and the resolver would rightly refuse it.
    #[cfg(test)]
    pub(crate) fn trusting(origin: &str) -> Self {
        let origin = Url::parse(origin).expect("a test origin").origin();
        Self::with_trusted(Some(origin))
    }

    fn with_trusted(trusted: Option<Origin>) -> Self {
        let hop_trusted = trusted.clone();
        let policy = redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() > MAX_REDIRECTS {
                return attempt.error("too many redirects");
            }
            if allowed(attempt.url(), hop_trusted.as_ref()) {
                attempt.follow()
            } else {
                let refused = format!("refused a redirect to {}", attempt.url());
                attempt.error(refused)
            }
        });
        let client = reqwest::Client::builder()
            .redirect(policy)
            .dns_resolver(Arc::new(PublicOnlyResolver))
            .no_proxy()
            .timeout(FETCH_TIMEOUT)
            .build()
            .expect("the image fetch client builds");
        Self { client, trusted }
    }

    /// The image at `url` and its type, read from the header or, when the
    /// header is vague, from the bytes.
    pub async fn fetch(&self, url: &str) -> Result<(Vec<u8>, MimeType)> {
        let parsed = Url::parse(url).ok();
        if !parsed
            .as_ref()
            .is_some_and(|parsed| allowed(parsed, self.trusted.as_ref()))
        {
            return Err(Error::failed(REFUSED));
        }
        let mut response = self.client.get(url).send().await.map_err(|error| {
            tracing::warn!(url, error = %error, "image fetch refused or failed");
            Error::failed(UNREACHABLE)
        })?;
        let status = response.status();
        if !status.is_success() {
            tracing::warn!(
                url,
                status = status.as_u16(),
                "image fetch answered an error"
            );
            return Err(Error::failed(UNREACHABLE));
        }
        let too_big = || {
            Error::failed(format!(
                "that image is over the {MAX_IMAGE_BYTES} byte limit"
            ))
        };
        if response
            .content_length()
            .is_some_and(|length| length > MAX_IMAGE_BYTES as u64)
        {
            return Err(too_big());
        }
        let declared = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .map(|value| value.trim().to_lowercase())
            .unwrap_or_default();
        let mut data = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|error| {
            tracing::warn!(url, error = %error, "image fetch broke off");
            Error::failed(UNREACHABLE)
        })? {
            if data.len() + chunk.len() > MAX_IMAGE_BYTES {
                return Err(too_big());
            }
            data.extend_from_slice(&chunk);
        }
        let Some(mime_type) = MimeType::image_from_header(&declared).or_else(|| sniff(&data))
        else {
            tracing::warn!(
                url,
                content_type = declared,
                "image fetch returned something else"
            );
            return Err(Error::failed(NOT_AN_IMAGE));
        };
        Ok((data, mime_type))
    }
}

/// Whether one hop may be fetched: the guard's answer, or the trusted
/// origin.
fn allowed(url: &Url, trusted: Option<&Origin>) -> bool {
    trusted.is_some_and(|trusted| url.origin() == *trusted) || is_fetchable_image_url(url.as_str())
}

#[cfg(test)]
#[path = "fetch_tests.rs"]
mod tests;
