//! Hoster extractors: hoster page in, direct stream URL out.
//!
//! Hosters know nothing about sites. Each module reads its hoster's player page with pure
//! functions, which the tests run against saved markup in `fixtures/`.

mod vidmoly;
mod voe;

use async_trait::async_trait;
use kura_domain::Stream;

pub use vidmoly::Vidmoly;
pub use voe::Voe;

#[derive(Debug, thiserror::Error)]
pub enum HosterError {
    #[error("request failed: {0}")]
    Http(#[from] reqwest::Error),
    /// The hoster refused the page, e.g. 404 for a removed video or 503 while it is busy.
    #[error("the hoster answered {0}")]
    Status(reqwest::StatusCode),
    /// The video was removed or the hoster changed its player page.
    #[error("no stream found on the {0} page")]
    NoStream(&'static str),
    /// The link points into a local network, see `kura_sites::guard`.
    #[error(transparent)]
    Blocked(#[from] kura_sites::guard::Blocked),
}

impl HosterError {
    /// Whether asking again may work. Hosters also answer 404 for a moment for videos that
    /// are there, so only a page without a stream is final.
    pub fn is_temporary(&self) -> bool {
        !matches!(self, Self::NoStream(_) | Self::Blocked(_))
    }
}

pub type Result<T> = std::result::Result<T, HosterError>;

#[async_trait]
pub trait Hoster: Send + Sync {
    /// Lower-case name as the sites spell it.
    fn name(&self) -> &'static str;

    /// `url` is the link a site handed out; redirects to the hoster are followed.
    async fn resolve(&self, url: &str) -> Result<Stream>;
}

/// Every hoster Kura can extract from.
pub fn all(http: &reqwest::Client) -> Vec<Box<dyn Hoster>> {
    vec![
        Box::new(Voe::new(http.clone())),
        Box::new(Vidmoly::new(http.clone())),
    ]
}

async fn get(http: &reqwest::Client, url: &str) -> Result<String> {
    // Names and redirects are checked by the client; an address in the link is checked here.
    let parsed = reqwest::Url::parse(url).map_err(|_| kura_sites::guard::Blocked(url.into()))?;
    if !kura_sites::guard::allowed(&parsed) {
        return Err(kura_sites::guard::Blocked(url.to_owned()).into());
    }
    let answer = http.get(parsed).send().await?;
    if !answer.status().is_success() {
        return Err(HosterError::Status(answer.status()));
    }
    Ok(answer.text().await?)
}

#[cfg(test)]
mod tests {
    use kura_domain::{EpisodeId, SiteId};
    use kura_sites::{AniWorld, Site};

    use super::*;

    /// A hoster page that redirects into the local network, as a hostile site or hoster
    /// could. Before the guard, the service behind it got the request.
    #[tokio::test]
    async fn links_and_redirects_into_the_local_network_are_not_followed() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        // Plays the router: counts every request it gets.
        let router = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let router_url = format!("http://{}/cgi-bin/reboot", router.local_addr().unwrap());
        let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = hits.clone();
        tokio::spawn(async move {
            while let Ok((mut connection, _)) = router.accept().await {
                counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let _ = connection.write_all(b"HTTP/1.1 200 OK\r\n\r\n").await;
            }
        });

        let http = kura_sites::http_client().unwrap();
        for hoster in all(&http) {
            let answer = hoster.resolve(&router_url).await;
            assert!(matches!(answer, Err(HosterError::Blocked(_))), "{answer:?}");
            assert!(!answer.unwrap_err().is_temporary());
        }
        let answer = get(&http, "http://localhost:9/x").await;
        assert!(answer.is_err());

        // A redirect to the router: the client stops at the redirect.
        let hoster = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = hoster.local_addr().unwrap().port();
        let location = router_url.clone();
        tokio::spawn(async move {
            let (mut connection, _) = hoster.accept().await.unwrap();
            let mut request = [0u8; 1024];
            let _ = connection.read(&mut request).await;
            let answer = format!("HTTP/1.1 302 Found\r\nLocation: {location}\r\n\r\n");
            let _ = connection.write_all(answer.as_bytes()).await;
        });
        // The test's own hoster is local too, so the first hop goes out on a client that only
        // has the redirect check; the redirect is what is under test.
        let lenient = reqwest::Client::builder()
            .redirect(kura_sites::guard::redirects())
            .build()
            .unwrap();
        let answer = lenient
            .get(format!("http://127.0.0.1:{port}/e/abc"))
            .send()
            .await;
        assert!(answer.is_err(), "{answer:?}");
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    /// Talks to a real site and the real hosters: `cargo test -p kura-hosters -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn live_hosters_still_resolve() {
        let http = kura_sites::http_client().unwrap();
        let episode = EpisodeId {
            site: SiteId::AniWorld,
            slug: "one-piece".into(),
            season: 1,
            number: 1,
        };
        let links = AniWorld::new(http.clone()).links(&episode).await.unwrap();

        for hoster in all(&http) {
            let link = links
                .iter()
                .find(|link| link.hoster == hoster.name())
                .unwrap_or_else(|| panic!("the episode has no {} link", hoster.name()));
            let stream = hoster.resolve(&link.url).await.unwrap();
            let playlist = get(&http, &stream.url).await.unwrap();
            assert!(playlist.starts_with("#EXTM3U"), "{}", hoster.name());
        }
    }
}
