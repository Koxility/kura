use async_trait::async_trait;
use kura_domain::Stream;

use crate::{Hoster, HosterError, Result, get};

pub struct Vidmoly {
    http: reqwest::Client,
}

impl Vidmoly {
    pub fn new(http: reqwest::Client) -> Self {
        Self { http }
    }
}

#[async_trait]
impl Hoster for Vidmoly {
    fn name(&self) -> &'static str {
        "vidmoly"
    }

    async fn resolve(&self, url: &str) -> Result<Stream> {
        let html = get(&self.http, url).await?;
        let url = source(&html).ok_or(HosterError::NoStream("Vidmoly"))?;
        Ok(Stream {
            url: url.to_owned(),
            headers: Vec::new(),
        })
    }
}

/// The playlist URL from the player setup: `sources: [{ file: '...' }]`.
fn source(html: &str) -> Option<&str> {
    let (_, rest) = html.split_once("sources:")?;
    let (_, rest) = rest.split_once("file:")?;
    let rest = rest.trim_start();
    let quote = rest.chars().next().filter(|c| matches!(c, '\'' | '"'))?;
    let (url, _) = rest[1..].split_once(quote)?;
    url.starts_with("https://").then_some(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn player_setup_gives_the_playlist() {
        assert_eq!(
            source(include_str!("../fixtures/vidmoly.html")),
            Some("https://cdn.example/hls2/abc123/master.m3u8?t=token&s=1")
        );
    }

    #[test]
    fn a_page_without_a_player_has_no_stream() {
        assert_eq!(source("<html>File was deleted</html>"), None);
        assert_eq!(source("sources: [{ file: '/relative.m3u8' }]"), None);
    }
}
