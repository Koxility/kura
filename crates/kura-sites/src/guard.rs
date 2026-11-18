//! Keeps sites and hosters from sending Kura into the local network.
//!
//! The pages Kura reads come from strangers: a site hands out links to hosters, a hoster page
//! names a stream, and both may redirect. Without a check, any of them could make Kura send
//! requests to the router, the NAS or Kura itself. Requests only go to public addresses: a
//! name is checked on every lookup, so a name that changes its address in between is caught
//! too, and an address in a link or redirect is checked before it is used.

use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
};

use reqwest::{
    Url,
    dns::{Addrs, Name, Resolve, Resolving},
    redirect,
};

/// Redirects one request may follow, as many as reqwest allows by default.
const MOST_REDIRECTS: usize = 10;

#[derive(Debug, thiserror::Error)]
#[error("{0} is in a local network; Kura only fetches from the internet")]
pub struct Blocked(pub String);

/// Whether `address` is on the internet, not in a local or special network.
pub fn is_public(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public_v4(v4);
            }
            let segments = v6.segments();
            // NAT64 (`64:ff9b::/96`) and 6to4 (`2002::/16`) carry an IPv4 address.
            if segments[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
                let [.., high, low] = segments;
                return is_public_v4(Ipv4Addr::from((u32::from(high) << 16) | u32::from(low)));
            }
            if segments[0] == 0x2002 {
                let v4 = (u32::from(segments[1]) << 16) | u32::from(segments[2]);
                return is_public_v4(Ipv4Addr::from(v4));
            }
            let documentation = segments[0] == 0x2001 && segments[1] == 0x0db8;
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || v6.is_unique_local()
                || v6.is_unicast_link_local()
                || documentation)
        }
    }
}

fn is_public_v4(v4: Ipv4Addr) -> bool {
    let [a, b, ..] = v4.octets();
    let shared = a == 100 && (64..128).contains(&b); // carrier-grade NAT, 100.64.0.0/10
    let this_network = a == 0;
    let benchmarking = a == 198 && (b == 18 || b == 19);
    let reserved = a >= 240;
    let protocol = a == 192 && b == 0 && v4.octets()[2] == 0;
    !(v4.is_loopback()
        || v4.is_private()
        || v4.is_link_local()
        || v4.is_unspecified()
        || v4.is_broadcast()
        || v4.is_multicast()
        || v4.is_documentation()
        || shared
        || this_network
        || benchmarking
        || reserved
        || protocol)
}

/// Whether a request to `url` may be sent: the web, and no address in a local network. A name
/// is checked when it is looked up, by [`PublicOnly`].
pub fn allowed(url: &Url) -> bool {
    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }
    match literal(url) {
        Some(Some(address)) => is_public(address),
        Some(None) => true,
        None => false,
    }
}

/// The address a URL names directly: `Some(None)` for a name, `None` without a host.
fn literal(url: &Url) -> Option<Option<IpAddr>> {
    let host = url.host_str()?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    Some(host.parse().ok())
}

/// Checks a stream address before ffmpeg gets it. ffmpeg looks the name up itself, so the
/// name is looked up here too; every address it has must be public.
pub async fn check_stream(url: &str) -> Result<(), Blocked> {
    let blocked = || Blocked(url.to_owned());
    let parsed = Url::parse(url).map_err(|_| blocked())?;
    if !allowed(&parsed) {
        return Err(blocked());
    }
    let (Some(None), Some(name)) = (literal(&parsed), parsed.host_str()) else {
        return Ok(());
    };
    let port = parsed.port_or_known_default().unwrap_or(443);
    let found: Vec<SocketAddr> = tokio::net::lookup_host((name, port))
        .await
        .map_err(|_| blocked())?
        .collect();
    if found.is_empty() || !found.iter().all(|address| is_public(address.ip())) {
        return Err(blocked());
    }
    Ok(())
}

/// Looks names up and leaves out every address that is not public. The proxies set in the
/// environment are the exception: they are chosen by whoever runs Kura and often sit in the
/// local network, e.g. a VPN container.
pub struct PublicOnly {
    proxies: Vec<String>,
}

impl PublicOnly {
    pub fn from_env() -> Self {
        let variables = ["HTTPS_PROXY", "HTTP_PROXY", "ALL_PROXY"];
        let values = variables
            .iter()
            .flat_map(|name| [name.to_string(), name.to_lowercase()])
            .filter_map(|name| std::env::var(name).ok());
        let proxies = values
            .filter_map(|value| {
                // `host:port` without a scheme is allowed as well.
                let value = if value.contains("://") {
                    value
                } else {
                    format!("http://{value}")
                };
                Some(Url::parse(&value).ok()?.host_str()?.to_lowercase())
            })
            .collect();
        Self { proxies }
    }
}

impl Resolve for PublicOnly {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_lowercase();
        let any_address = self.proxies.contains(&host);
        Box::pin(async move {
            let found = tokio::net::lookup_host((host.as_str(), 0)).await?;
            let usable: Vec<SocketAddr> = found
                .filter(|address| any_address || is_public(address.ip()))
                .collect();
            if usable.is_empty() {
                return Err(Box::new(Blocked(host)) as Box<dyn std::error::Error + Send + Sync>);
            }
            Ok(Box::new(usable.into_iter()) as Addrs)
        })
    }
}

/// Follows redirects like reqwest does, but not to an address in a local network.
pub fn redirects() -> redirect::Policy {
    redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= MOST_REDIRECTS {
            attempt.error("too many redirects")
        } else if !allowed(attempt.url()) {
            let target = attempt.url().to_string();
            attempt.error(Blocked(target))
        } else {
            attempt.follow()
        }
    })
}

/// The resolver for [`crate::http_client`].
pub(crate) fn resolver() -> Arc<PublicOnly> {
    Arc::new(PublicOnly::from_env())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_and_special_addresses_are_not_public() {
        for local in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.178.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "0.1.2.3",
            "255.255.255.255",
            "224.0.0.1",
            "198.18.0.1",
            "192.0.0.8",
            "240.0.0.1",
            "::1",
            "::",
            "fd00::1",
            "fe80::1",
            "::ffff:192.168.1.1",
            "64:ff9b::c0a8:101",
            "2002:c0a8:101::1",
            "2001:db8::1",
        ] {
            assert!(!is_public(local.parse().unwrap()), "{local}");
        }
        for public in [
            "8.8.8.8",
            "172.32.0.1",
            "100.128.0.1",
            "2a00:1450::1",
            "::ffff:8.8.8.8",
        ] {
            assert!(is_public(public.parse().unwrap()), "{public}");
        }
    }

    #[test]
    fn links_to_local_addresses_are_refused() {
        let allowed = |url: &str| allowed(&Url::parse(url).unwrap());
        assert!(allowed("https://voe.sx/e/abc"));
        assert!(allowed("https://8.8.8.8/x"));
        assert!(!allowed("http://127.0.0.1:8787/api/v1/queue"));
        assert!(!allowed("http://[::1]/"));
        assert!(!allowed("http://[::ffff:10.0.0.1]/"));
        assert!(!allowed("http://192.168.178.1/cgi-bin/reboot"));
        assert!(!allowed("file:///etc/passwd"));
        assert!(!allowed("ftp://8.8.8.8/x"));
    }

    #[tokio::test]
    async fn names_of_local_addresses_are_refused() {
        // `localhost` stands for every name that points into the local network.
        assert!(check_stream("https://localhost/master.m3u8").await.is_err());
        assert!(check_stream("http://127.0.0.1:9/x.m3u8").await.is_err());
        assert!(check_stream("not a url").await.is_err());
        let name: Name = "localhost".parse().unwrap();
        let resolver = PublicOnly {
            proxies: Vec::new(),
        };
        assert!(resolver.resolve(name).await.is_err());
        // A proxy from the environment may be local.
        let name: Name = "localhost".parse().unwrap();
        let resolver = PublicOnly {
            proxies: vec!["localhost".to_owned()],
        };
        assert!(resolver.resolve(name).await.is_ok());
    }
}
