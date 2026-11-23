//! A small HTTP proxy on 127.0.0.1 that ffmpeg and ffprobe go through.
//!
//! Kura checks a stream's address before it hands it to ffmpeg, but ffmpeg then fetches on
//! its own: it follows redirects, reads the parts an HLS playlist names and asks DNS again.
//! A hoster could point any of these at a machine in the local network. Through this relay,
//! every connection ffmpeg makes is checked: the name is resolved here, and the relay
//! connects to a checked address itself, so a second DNS answer cannot change it.
//!
//! Where the environment names a proxy (`HTTPS_PROXY` and the like), the relay still checks
//! every address and then goes on through that proxy, so downloads work as before.

use std::{
    io,
    net::{IpAddr, SocketAddr},
    sync::Arc,
};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::OnceCell,
};

use crate::guard::is_public;

/// The longest request head the relay reads; ffmpeg's are a few hundred bytes.
const MOST_HEAD_BYTES: usize = 16 * 1024;

/// Which addresses the relay may connect to.
pub type Allowed = fn(IpAddr) -> bool;

/// How the relay reaches the internet: directly, or through the proxy the environment names.
#[derive(Clone)]
struct Route {
    allowed: Allowed,
    /// `host:port` of that proxy.
    upstream: Option<String>,
}

/// The address of the relay for downloads, started on first use and kept for the life of
/// the server.
pub async fn address() -> io::Result<SocketAddr> {
    static RELAY: OnceCell<SocketAddr> = OnceCell::const_new();
    let upstream = crate::guard::environment_proxy();
    RELAY
        .get_or_try_init(|| start_with(is_public, upstream))
        .await
        .copied()
}

/// Starts a relay that connects only to addresses `allowed` accepts.
pub async fn start(allowed: Allowed) -> io::Result<SocketAddr> {
    start_with(allowed, None).await
}

/// Like [`start`], going on through the HTTP proxy at `upstream` (`host:port`), if any.
async fn start_with(allowed: Allowed, upstream: Option<String>) -> io::Result<SocketAddr> {
    let listener = TcpListener::bind((IpAddr::from([127, 0, 0, 1]), 0)).await?;
    let address = listener.local_addr()?;
    let route = Arc::new(Route { allowed, upstream });
    tokio::spawn(async move {
        loop {
            let Ok((client, _)) = listener.accept().await else {
                continue;
            };
            let route = route.clone();
            tokio::spawn(async move {
                if let Err(err) = serve(client, &route).await {
                    tracing::debug!("download relay: {err}");
                }
            });
        }
    });
    Ok(address)
}

/// One client connection: a `CONNECT host:port` tunnel (HTTPS) or one plain HTTP request in
/// absolute form (`GET http://host/path`).
async fn serve(mut client: TcpStream, route: &Route) -> io::Result<()> {
    let mut head = Vec::new();
    let end = loop {
        let mut chunk = [0u8; 4096];
        let read = client.read(&mut chunk).await?;
        if read == 0 {
            return Ok(());
        }
        head.extend_from_slice(&chunk[..read]);
        if let Some(end) = find(&head, b"\r\n\r\n") {
            break end + 4;
        }
        if head.len() > MOST_HEAD_BYTES {
            return refuse(&mut client, "431 Request Header Fields Too Large").await;
        }
    };
    let text = String::from_utf8_lossy(&head[..end]).into_owned();
    let first = text.lines().next().unwrap_or_default();
    let mut words = first.split(' ');
    let (Some(method), Some(target), Some(version)) = (words.next(), words.next(), words.next())
    else {
        return refuse(&mut client, "400 Bad Request").await;
    };

    if method.eq_ignore_ascii_case("CONNECT") {
        let Some((host, port)) = host_port(target, 443) else {
            return refuse(&mut client, "400 Bad Request").await;
        };
        let Some(mut server) = tunnel(&host, port, route).await else {
            return refuse(&mut client, "403 Forbidden").await;
        };
        client
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .await?;
        // Whatever the client sent after the head already belongs to the tunnel.
        server.write_all(&head[end..]).await?;
        tokio::io::copy_bidirectional(&mut client, &mut server).await?;
        return Ok(());
    }

    let Some(rest) = target.strip_prefix("http://") else {
        return refuse(&mut client, "400 Bad Request").await;
    };
    let (authority, path) = match rest.find('/') {
        Some(slash) => (&rest[..slash], &rest[slash..]),
        None => (rest, "/"),
    };
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    let Some((host, port)) = host_port(authority, 80) else {
        return refuse(&mut client, "400 Bad Request").await;
    };
    if !checked(&host, port, route).await {
        return refuse(&mut client, "403 Forbidden").await;
    }
    let mut server = match &route.upstream {
        // The proxy takes the request as it is, in absolute form.
        Some(upstream) => {
            let mut server = TcpStream::connect(upstream.as_str()).await?;
            server.write_all(&head).await?;
            server
        }
        None => {
            let Some(mut server) = connect(&host, port, route.allowed).await else {
                return refuse(&mut client, "403 Forbidden").await;
            };
            // The same request in origin form, with the rest of the head and any body.
            let first_end = text.find("\r\n").unwrap_or(text.len());
            let mut forwarded = format!("{method} {path} {version}").into_bytes();
            forwarded.extend_from_slice(&head[first_end..]);
            server.write_all(&forwarded).await?;
            server
        }
    };
    tokio::io::copy_bidirectional(&mut client, &mut server).await?;
    Ok(())
}

/// A connection to `host:port` for a tunnel: directly, or through the upstream proxy once
/// the address passed the check.
async fn tunnel(host: &str, port: u16, route: &Route) -> Option<TcpStream> {
    let Some(upstream) = &route.upstream else {
        return connect(host, port, route.allowed).await;
    };
    if !checked(host, port, route).await {
        return None;
    }
    let mut server = TcpStream::connect(upstream.as_str()).await.ok()?;
    let target = if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };
    let ask = format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n\r\n");
    server.write_all(ask.as_bytes()).await.ok()?;
    // The proxy's answer, byte by byte up to the empty line, so nothing of the tunnel is read.
    let mut answer = Vec::new();
    let mut byte = [0u8; 1];
    while !answer.ends_with(b"\r\n\r\n") {
        if answer.len() > MOST_HEAD_BYTES || server.read(&mut byte).await.ok()? == 0 {
            return None;
        }
        answer.push(byte[0]);
    }
    let status = String::from_utf8_lossy(&answer);
    let status = status.split(' ').nth(1).unwrap_or_default();
    (status == "200").then_some(server)
}

/// Whether `host` may be reached through the upstream proxy: an address must be allowed, and
/// so must every address a name has here. A name that does not resolve here is the proxy's
/// to find; it cannot be one of this network's machines that only this host knows.
async fn checked(host: &str, port: u16, route: &Route) -> bool {
    if let Ok(address) = host.parse::<IpAddr>() {
        return (route.allowed)(address);
    }
    if route.upstream.is_none() {
        // Checked again, and connected to, by `connect`.
        return true;
    }
    match tokio::net::lookup_host((host, port)).await {
        Ok(found) => {
            let found: Vec<SocketAddr> = found.collect();
            let fine = found.iter().all(|address| (route.allowed)(address.ip()));
            if !fine {
                tracing::warn!("a download wanted {host}, which is not on the internet; refused");
            }
            fine
        }
        Err(_) => true,
    }
}

/// Connects to the first address of `host` that `allowed` accepts, and only if it accepts
/// every address the name has, as the guard does for the stream itself.
async fn connect(host: &str, port: u16, allowed: Allowed) -> Option<TcpStream> {
    let found: Vec<SocketAddr> = match host.parse::<IpAddr>() {
        Ok(address) => vec![SocketAddr::new(address, port)],
        Err(_) => tokio::net::lookup_host((host, port)).await.ok()?.collect(),
    };
    if found.is_empty() || !found.iter().all(|address| allowed(address.ip())) {
        tracing::warn!("a download wanted {host}:{port}, which is not on the internet; refused");
        return None;
    }
    for address in found {
        if let Ok(stream) = TcpStream::connect(address).await {
            return Some(stream);
        }
    }
    None
}

/// `example.com:443`, `[2001:db8::1]:443` or `example.com` (then `default` is the port).
fn host_port(authority: &str, default: u16) -> Option<(String, u16)> {
    if let Some(rest) = authority.strip_prefix('[') {
        let (host, after) = rest.split_once(']')?;
        let port = match after.strip_prefix(':') {
            Some(port) => port.parse().ok()?,
            None if after.is_empty() => default,
            None => return None,
        };
        return Some((host.to_owned(), port));
    }
    match authority.rsplit_once(':') {
        Some((host, port)) => Some((host.to_owned(), port.parse().ok()?)),
        None if !authority.is_empty() => Some((authority.to_owned(), default)),
        None => None,
    }
}

async fn refuse(client: &mut TcpStream, status: &str) -> io::Result<()> {
    let answer = format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    client.write_all(answer.as_bytes()).await
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 127.0.0.1 stands for the internet here, 127.0.0.2 for the local network.
    fn only_first(address: IpAddr) -> bool {
        address == IpAddr::from([127, 0, 0, 1])
    }

    async fn ask(relay: SocketAddr, request: &str) -> String {
        let mut stream = TcpStream::connect(relay).await.unwrap();
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut answer = String::new();
        stream.read_to_string(&mut answer).await.unwrap();
        answer
    }

    async fn web_server(at: IpAddr) -> SocketAddr {
        let listener = TcpListener::bind((at, 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let mut buffer = [0u8; 1024];
                let read = stream.read(&mut buffer).await.unwrap_or(0);
                let first = String::from_utf8_lossy(&buffer[..read])
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .to_owned();
                let body = format!("you asked: {first}");
                let answer = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(answer.as_bytes()).await;
            }
        });
        address
    }

    #[tokio::test]
    async fn passes_allowed_addresses_and_refuses_the_rest() {
        let relay = start(only_first).await.unwrap();
        let public = web_server(IpAddr::from([127, 0, 0, 1])).await;
        let local = web_server(IpAddr::from([127, 0, 0, 2])).await;

        let plain = ask(
            relay,
            &format!("GET http://{public}/a.ts HTTP/1.1\r\nHost: {public}\r\n\r\n"),
        )
        .await;
        assert!(plain.contains("you asked: GET /a.ts HTTP/1.1"), "{plain}");

        let tunnel = ask(
            relay,
            &format!("CONNECT {public} HTTP/1.1\r\n\r\nGET /b HTTP/1.1\r\nHost: x\r\n\r\n"),
        )
        .await;
        assert!(
            tunnel.starts_with("HTTP/1.1 200 Connection Established"),
            "{tunnel}"
        );
        assert!(tunnel.contains("you asked: GET /b HTTP/1.1"), "{tunnel}");

        for request in [
            format!("GET http://{local}/secret HTTP/1.1\r\n\r\n"),
            format!("CONNECT {local} HTTP/1.1\r\n\r\n"),
        ] {
            let answer = ask(relay, &request).await;
            assert!(answer.starts_with("HTTP/1.1 403"), "{request}: {answer}");
        }
        let odd = ask(relay, "GET /relative HTTP/1.1\r\n\r\n").await;
        assert!(odd.starts_with("HTTP/1.1 400"), "{odd}");
    }

    #[test]
    fn reads_host_and_port() {
        assert_eq!(
            host_port("example.com:8443", 443),
            Some(("example.com".into(), 8443))
        );
        assert_eq!(
            host_port("example.com", 80),
            Some(("example.com".into(), 80))
        );
        assert_eq!(
            host_port("[2001:db8::1]:443", 80),
            Some(("2001:db8::1".into(), 443))
        );
        assert_eq!(host_port("[::1]", 80), Some(("::1".into(), 80)));
        assert_eq!(host_port("example.com:x", 80), None);
        assert_eq!(host_port("", 80), None);
    }

    /// ffmpeg through the relay: a public video loads, while a playlist part and a redirect
    /// that point at the "local network" never reach it.
    #[tokio::test]
    async fn ffmpeg_cannot_reach_the_local_network_through_it() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static LOCAL_HITS: AtomicUsize = AtomicUsize::new(0);
        let local = TcpListener::bind((IpAddr::from([127, 0, 0, 2]), 0))
            .await
            .unwrap();
        let local_address = local.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = local.accept().await {
                LOCAL_HITS.fetch_add(1, Ordering::SeqCst);
                let _ = stream
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\nsecret")
                    .await;
            }
        });

        // A real, short video on the "internet".
        let clip = std::env::temp_dir().join(format!("kura-relay-{}.ts", std::process::id()));
        let made = tokio::process::Command::new("ffmpeg")
            .args(["-loglevel", "error", "-y", "-f", "lavfi", "-i"])
            .args([
                "testsrc=duration=1:size=64x64:rate=10",
                "-c:v",
                "mpeg2video",
            ])
            .args(["-f", "mpegts"])
            .arg(&clip)
            .status()
            .await
            .expect("ffmpeg is installed");
        assert!(made.success());
        let video = std::fs::read(&clip).unwrap();
        let _ = std::fs::remove_file(&clip);

        let hoster = TcpListener::bind((IpAddr::from([127, 0, 0, 1]), 0))
            .await
            .unwrap();
        let hoster_address = hoster.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = hoster.accept().await {
                let mut buffer = [0u8; 2048];
                let read = stream.read(&mut buffer).await.unwrap_or(0);
                let head = String::from_utf8_lossy(&buffer[..read]).into_owned();
                let (kind, body) = if head.contains("/clip.ts") {
                    ("video/mp2t", video.clone())
                } else if head.contains("/redirect") {
                    let answer = format!(
                        "HTTP/1.1 302 Found\r\nLocation: http://{local_address}/from-redirect\r\n\
                         Content-Length: 0\r\n\r\n"
                    );
                    let _ = stream.write_all(answer.as_bytes()).await;
                    continue;
                } else {
                    let list = format!(
                        "#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2,\n\
                         http://{local_address}/from-playlist.ts\n#EXT-X-ENDLIST\n"
                    );
                    ("application/vnd.apple.mpegurl", list.into_bytes())
                };
                let start = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\n\
                     Connection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(start.as_bytes()).await;
                let _ = stream.write_all(&body).await;
            }
        });

        let relay = start(only_first).await.unwrap();
        let ffmpeg = |path: &str| {
            let mut command = tokio::process::Command::new("ffmpeg");
            command
                .args(["-nostdin", "-hide_banner", "-loglevel", "quiet"])
                .args(["-rw_timeout", "5000000"])
                .args(["-http_proxy", &format!("http://{relay}")])
                .args(["-i", &format!("http://{hoster_address}/{path}")])
                .args(["-c", "copy", "-f", "null", "-"])
                .env_remove("http_proxy");
            command
        };
        let public = ffmpeg("clip.ts").status().await.unwrap();
        assert!(public.success(), "a public video loads through the relay");
        for path in ["stream.m3u8", "redirect.mp4"] {
            let status = ffmpeg(path).status().await.unwrap();
            assert!(!status.success(), "{path} should fail");
        }
        assert_eq!(LOCAL_HITS.load(Ordering::SeqCst), 0);
    }

    /// With a proxy in the environment, allowed connections go on through it and refused ones
    /// never reach it.
    #[tokio::test]
    async fn goes_on_through_the_environment_proxy() {
        let upstream = TcpListener::bind((IpAddr::from([127, 0, 0, 1]), 0))
            .await
            .unwrap();
        let upstream_address = upstream.local_addr().unwrap();
        let (seen_tx, mut seen) = tokio::sync::mpsc::unbounded_channel::<String>();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = upstream.accept().await {
                let mut buffer = [0u8; 2048];
                let read = stream.read(&mut buffer).await.unwrap_or(0);
                let head = String::from_utf8_lossy(&buffer[..read]).into_owned();
                let first = head.lines().next().unwrap_or_default().to_owned();
                let _ = seen_tx.send(first.clone());
                let answer = if first.starts_with("CONNECT") {
                    "HTTP/1.1 200 Connection Established\r\n\r\nthrough the tunnel".to_owned()
                } else {
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{first}",
                        first.len()
                    )
                };
                let _ = stream.write_all(answer.as_bytes()).await;
            }
        });
        let relay = start_with(only_first, Some(upstream_address.to_string()))
            .await
            .unwrap();

        let plain = ask(relay, "GET http://127.0.0.1:81/a HTTP/1.1\r\n\r\n").await;
        assert!(
            plain.ends_with("GET http://127.0.0.1:81/a HTTP/1.1"),
            "{plain}"
        );
        let tunnel = ask(relay, "CONNECT 127.0.0.1:443 HTTP/1.1\r\n\r\n").await;
        assert!(tunnel.ends_with("through the tunnel"), "{tunnel}");
        assert_eq!(
            seen.recv().await.unwrap(),
            "GET http://127.0.0.1:81/a HTTP/1.1"
        );
        assert_eq!(seen.recv().await.unwrap(), "CONNECT 127.0.0.1:443 HTTP/1.1");

        for request in [
            "GET http://127.0.0.2/secret HTTP/1.1\r\n\r\n",
            "CONNECT 127.0.0.2:443 HTTP/1.1\r\n\r\n",
        ] {
            let answer = ask(relay, request).await;
            assert!(answer.starts_with("HTTP/1.1 403"), "{request}: {answer}");
        }
        assert!(
            seen.try_recv().is_err(),
            "a refused address reached the proxy"
        );
    }
}
