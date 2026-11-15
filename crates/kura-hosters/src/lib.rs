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
}

impl HosterError {
    /// Whether asking again may work. Hosters also answer 404 for a moment for videos that
    /// are there, so only a page without a stream is final.
    pub fn is_temporary(&self) -> bool {
        !matches!(self, Self::NoStream(_))
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
    let answer = http.get(url).send().await?;
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
