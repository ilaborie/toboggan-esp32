//! Resolves the toboggan server address once, with a fallback host.
//!
//! `TOBOGGAN_HOST` is normally an mDNS `.local` name, which only resolves on a
//! network that forwards multicast to `224.0.0.251`. `TOBOGGAN_HOST_FALLBACK` —
//! typically a literal IP — covers the networks that do not, such as guest WiFi
//! or a phone hotspot.

use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::OnceLock;

use anyhow::bail;
use log::{info, warn};

use crate::config::env::{TOBOGGAN_HOST, TOBOGGAN_HOST_FALLBACK};
use crate::state::error_chain;

/// The winning address, kept for the whole run so the API and WebSocket threads
/// can never end up talking to two different servers.
static SERVER_ADDR: OnceLock<SocketAddr> = OnceLock::new();

/// Hosts to try, in order.
fn candidates() -> impl Iterator<Item = &'static str> {
    [Some(TOBOGGAN_HOST), TOBOGGAN_HOST_FALLBACK]
        .into_iter()
        .flatten()
        .filter(|host| !host.is_empty())
}

/// Resolves `host` to a single address, preferring IPv4.
///
/// The box reaches the server over the LAN, where an mDNS answer often carries
/// both families but only the v4 address is routable for it.
fn resolve_one(host: &str, port: u16) -> anyhow::Result<SocketAddr> {
    let mut addresses = (host, port)
        .to_socket_addrs()
        .map_err(anyhow::Error::new)?
        .peekable();

    if addresses.peek().is_none() {
        bail!("no address");
    }

    let mut first = None;
    for address in addresses {
        if address.is_ipv4() {
            return Ok(address);
        }
        first.get_or_insert(address);
    }
    first.ok_or_else(|| anyhow::anyhow!("no address"))
}

/// Resolves the server address, caching the first success for the whole run.
///
/// Every attempt is logged, so a boot that fell back to the second host says so
/// in the serial output rather than silently working from a different address.
///
/// # Errors
/// Returns an error naming every candidate tried when none of them resolve.
pub fn server_addr(port: u16) -> anyhow::Result<SocketAddr> {
    if let Some(address) = SERVER_ADDR.get() {
        return Ok(*address);
    }

    let mut failures = Vec::new();
    for (index, host) in candidates().enumerate() {
        info!("🔍 Resolving {host}:{port}");
        match resolve_one(host, port) {
            Ok(address) => {
                if index == 0 {
                    info!("🔍 {host} resolved to {address}");
                } else {
                    warn!("🔍 Falling back to {host} ({address}): {TOBOGGAN_HOST} did not resolve");
                }
                // A concurrent caller may have won the race; either way both end
                // up returning the same address.
                return Ok(*SERVER_ADDR.get_or_init(|| address));
            }
            Err(error) => {
                let reason = error_chain(&error);
                warn!("🔍 Could not resolve {host}: {reason}");
                failures.push(format!("{host}: {reason}"));
            }
        }
    }

    bail!("resolve {}", failures.join(", "))
}
