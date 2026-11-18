//! Reverse proxies Kura may believe. Behind one, every connection comes from the proxy, so
//! the address of the person, which the login brake counts by, is in `X-Forwarded-For`.
//! Anyone can send that header, so it only counts when the connection comes from a proxy
//! named in `KURA_TRUSTED_PROXIES`.

use std::{
    net::IpAddr,
    sync::atomic::{AtomicBool, Ordering},
};

use axum::http::HeaderMap;

/// The addresses and networks from `KURA_TRUSTED_PROXIES`, e.g. `172.18.0.0/16, ::1`.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct TrustedProxies(Vec<(IpAddr, u8)>);

impl std::fmt::Display for TrustedProxies {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let entries: Vec<String> = self
            .0
            .iter()
            .map(|(address, bits)| format!("{address}/{bits}"))
            .collect();
        f.write_str(&entries.join(", "))
    }
}

/// What Kura knows about who is asking.
#[derive(Debug, PartialEq)]
pub struct Client {
    /// The person's address: the connection's, or the one a trusted proxy names.
    pub address: IpAddr,
    /// Whether the person reached the proxy over HTTPS, as a trusted proxy says.
    pub https: bool,
}

/// Logged once, so the hint does not fill the log.
static HINTED: AtomicBool = AtomicBool::new(false);

impl TrustedProxies {
    /// Reads `KURA_TRUSTED_PROXIES`; unset or empty trusts nobody.
    pub fn from_env() -> Result<Self, String> {
        match std::env::var("KURA_TRUSTED_PROXIES") {
            Ok(list) => Self::parse(&list),
            Err(_) => Ok(Self::default()),
        }
    }

    /// A comma- or space-separated list of addresses and networks.
    pub fn parse(list: &str) -> Result<Self, String> {
        let entries = list.split([',', ' ']).filter(|entry| !entry.is_empty());
        entries
            .map(|entry| {
                let bad = || format!("{entry:?} is not an address or a network like 10.0.0.0/8");
                let (address, bits) = match entry.split_once('/') {
                    Some((address, bits)) => (address, Some(bits)),
                    None => (entry, None),
                };
                let address: IpAddr = address.parse().map_err(|_| bad())?;
                let address = canonical(address);
                let longest = if address.is_ipv4() { 32 } else { 128 };
                let bits = match bits {
                    Some(bits) => bits.parse().ok().filter(|bits| *bits <= longest),
                    None => Some(longest),
                };
                Ok((address, bits.ok_or_else(bad)?))
            })
            .collect::<Result<_, _>>()
            .map(Self)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn contains(&self, address: IpAddr) -> bool {
        let address = canonical(address);
        self.0
            .iter()
            .any(|(network, bits)| in_network(address, *network, *bits))
    }

    /// Who is asking over a connection from `peer`. The headers count only when `peer` is a
    /// trusted proxy; then the person is the last address in `X-Forwarded-For` that is not a
    /// trusted proxy itself, since every proxy adds the address it was reached from at the end.
    pub fn client(&self, peer: IpAddr, headers: &HeaderMap) -> Client {
        let peer = canonical(peer);
        let forwarded: Vec<&str> = headers
            .get_all("x-forwarded-for")
            .iter()
            .filter_map(|value| value.to_str().ok())
            .flat_map(|value| value.split(','))
            .map(str::trim)
            .collect();
        if !self.contains(peer) {
            if !forwarded.is_empty() && !HINTED.swap(true, Ordering::Relaxed) {
                tracing::warn!(
                    "a request from {peer} names the address it was forwarded for, which is \
                     ignored; if {peer} is your reverse proxy, add it to KURA_TRUSTED_PROXIES"
                );
            }
            return Client {
                address: peer,
                https: false,
            };
        }

        let mut address = peer;
        for entry in forwarded.iter().rev() {
            // A proxy that sends something unreadable is not followed any further.
            let Some(hop) = parse_hop(entry) else { break };
            address = hop;
            if !self.contains(hop) {
                break;
            }
        }
        let proto = headers.get("x-forwarded-proto");
        let proto = proto.and_then(|value| value.to_str().ok());
        // Like the addresses, the last value is the one the nearest proxy set.
        let proto = proto.and_then(|value| value.rsplit(',').next());
        Client {
            address,
            https: proto.is_some_and(|proto| proto.trim().eq_ignore_ascii_case("https")),
        }
    }
}

/// `1.2.3.4`, `1.2.3.4:5678`, `2001:db8::1` or `[2001:db8::1]:5678`.
fn parse_hop(entry: &str) -> Option<IpAddr> {
    let address = entry
        .parse::<IpAddr>()
        .ok()
        .or_else(|| entry.parse::<std::net::SocketAddr>().ok().map(|at| at.ip()))
        .or_else(|| entry.strip_prefix('[')?.strip_suffix(']')?.parse().ok())?;
    Some(canonical(address))
}

/// An IPv4 address that arrives as IPv6 (`::ffff:1.2.3.4`) is the IPv4 address.
fn canonical(address: IpAddr) -> IpAddr {
    match address {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(address, IpAddr::V4),
        v4 => v4,
    }
}

fn in_network(address: IpAddr, network: IpAddr, bits: u8) -> bool {
    let masked = |value: u128, width: u32| {
        let bits = u32::from(bits);
        if bits == 0 {
            0
        } else {
            value >> (width - bits)
        }
    };
    match (address, network) {
        (IpAddr::V4(a), IpAddr::V4(n)) => {
            masked(u32::from(a).into(), 32) == masked(u32::from(n).into(), 32)
        }
        (IpAddr::V6(a), IpAddr::V6(n)) => masked(a.into(), 128) == masked(n.into(), 128),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.append(*name, value.parse().unwrap());
        }
        map
    }

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    #[test]
    fn the_list_takes_addresses_and_networks() {
        let proxies = TrustedProxies::parse("172.18.0.0/16, ::1  10.0.0.5").unwrap();
        assert!(proxies.contains(ip("172.18.4.2")));
        assert!(!proxies.contains(ip("172.19.0.1")));
        assert!(proxies.contains(ip("::1")));
        assert!(proxies.contains(ip("10.0.0.5")));
        assert!(proxies.contains(ip("::ffff:10.0.0.5")));
        assert!(!proxies.contains(ip("10.0.0.6")));
        assert!(TrustedProxies::parse("").unwrap().is_empty());
        assert!(
            TrustedProxies::parse("0.0.0.0/0")
                .unwrap()
                .contains(ip("8.8.8.8"))
        );
        for bad in ["haproxy", "10.0.0.0/33", "::/129", "10.0.0.0/", "1.2.3"] {
            assert!(TrustedProxies::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn forwarded_addresses_count_only_from_a_trusted_proxy() {
        let proxies = TrustedProxies::parse("10.0.0.5").unwrap();
        let forwarded = headers(&[
            ("x-forwarded-for", "203.0.113.9"),
            ("x-forwarded-proto", "https"),
        ]);
        // Straight from the internet, the headers are made up and ignored.
        let direct = proxies.client(ip("198.51.100.1"), &forwarded);
        assert_eq!(
            direct,
            Client {
                address: ip("198.51.100.1"),
                https: false
            }
        );
        // Through the proxy, they name the person.
        let proxied = proxies.client(ip("10.0.0.5"), &forwarded);
        assert_eq!(
            proxied,
            Client {
                address: ip("203.0.113.9"),
                https: true
            }
        );
        // Nobody trusted: nothing changes.
        let none = TrustedProxies::default().client(ip("10.0.0.5"), &forwarded);
        assert_eq!(none.address, ip("10.0.0.5"));
    }

    #[test]
    fn a_made_up_address_in_front_does_not_count() {
        let proxies = TrustedProxies::parse("10.0.0.0/24").unwrap();
        // The person sent "1.1.1.1"; the proxy added the address it really saw.
        let chain = headers(&[("x-forwarded-for", "1.1.1.1, 203.0.113.9")]);
        assert_eq!(
            proxies.client(ip("10.0.0.5"), &chain).address,
            ip("203.0.113.9")
        );
        // Two proxies in a row, the second one trusted too; also in separate headers.
        let chain = headers(&[
            ("x-forwarded-for", "1.1.1.1, 203.0.113.9"),
            ("x-forwarded-for", "10.0.0.7"),
        ]);
        assert_eq!(
            proxies.client(ip("10.0.0.5"), &chain).address,
            ip("203.0.113.9")
        );
        // Ports and brackets are allowed, garbage stops the walk at the last good hop.
        let chain = headers(&[("x-forwarded-for", "[2001:db8::7]:4711")]);
        assert_eq!(
            proxies.client(ip("10.0.0.5"), &chain).address,
            ip("2001:db8::7")
        );
        let chain = headers(&[("x-forwarded-for", "203.0.113.9, unknown")]);
        assert_eq!(
            proxies.client(ip("10.0.0.5"), &chain).address,
            ip("10.0.0.5")
        );
        // No header at all: the proxy itself, and not HTTPS.
        let alone = proxies.client(ip("10.0.0.5"), &HeaderMap::new());
        assert_eq!(
            alone,
            Client {
                address: ip("10.0.0.5"),
                https: false
            }
        );
        let http = headers(&[("x-forwarded-proto", "https, http")]);
        assert!(!proxies.client(ip("10.0.0.5"), &http).https);
    }
}
