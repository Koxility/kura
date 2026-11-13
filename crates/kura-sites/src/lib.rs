//! Site scrapers: search, popular list, series pages and the hoster links of an episode.
//!
//! Sites host no video. Each module fetches the pages of one site and reads them with pure
//! functions, which the tests run against saved markup in `fixtures/`.

mod aniworld;
mod serienstream;

use std::time::Duration;

use async_trait::async_trait;
use kura_domain::{Episode, EpisodeId, Genre, HosterLink, Series, SeriesSummary, SiteId};
use scraper::{ElementRef, Selector};

pub use aniworld::AniWorld;
pub use serienstream::SerienStream;

#[derive(Debug, thiserror::Error)]
pub enum SiteError {
    #[error("request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("not a valid series name: {0}")]
    BadSlug(String),
    #[error("the page layout changed: no {0} found")]
    Layout(&'static str),
}

pub type Result<T> = std::result::Result<T, SiteError>;

#[async_trait]
pub trait Site: Send + Sync {
    fn id(&self) -> SiteId;

    async fn search(&self, query: &str) -> Result<Vec<SeriesSummary>>;

    async fn popular(&self) -> Result<Vec<SeriesSummary>>;

    /// Recently added titles, newest first.
    async fn latest(&self) -> Result<Vec<SeriesSummary>>;

    async fn genres(&self) -> Result<Vec<Genre>>;

    /// One page of a genre, counted from 1. Empty past the last page.
    async fn by_genre(&self, genre: &str, page: u32) -> Result<Vec<SeriesSummary>>;

    /// Every series of the site, without covers. A large page; callers should keep the result.
    async fn all(&self) -> Result<Vec<SeriesSummary>>;

    async fn series(&self, slug: &str) -> Result<Series>;

    /// Season 0 holds films and specials.
    async fn episodes(&self, slug: &str, season: u32) -> Result<Vec<Episode>>;

    /// The links are redirects through the site and may expire, so fetch them right before use.
    async fn links(&self, episode: &EpisodeId) -> Result<Vec<HosterLink>>;
}

/// The HTTP client all sites share.
pub fn http_client() -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        // Both sites answer a browser user agent; others are not guaranteed.
        .user_agent("Mozilla/5.0 (X11; Linux x86_64; rv:140.0) Gecko/20100101 Firefox/140.0")
        .timeout(Duration::from_secs(30))
        .build()
}

async fn get(http: &reqwest::Client, url: &str) -> Result<String> {
    Ok(http
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?)
}

/// Slugs end up in request URLs, so anything that could change the path is refused.
fn check_slug(slug: &str) -> Result<&str> {
    let valid = !slug.is_empty()
        && slug
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if valid {
        Ok(slug)
    } else {
        Err(SiteError::BadSlug(slug.to_owned()))
    }
}

/// The slug of a link to a series page; `None` for links to anything else.
fn slug_from(href: &str, prefix: &str) -> Option<String> {
    let slug = href.strip_prefix(prefix)?.trim_end_matches('/');
    check_slug(slug).ok().map(str::to_owned)
}

/// Keeps the first entry of every series.
fn unique(series: impl IntoIterator<Item = SeriesSummary>) -> Vec<SeriesSummary> {
    let mut seen = std::collections::HashSet::new();
    let fresh = series
        .into_iter()
        .filter(|series| seen.insert(series.slug.clone()));
    fresh.collect()
}

/// The IMDb ID from a link to the series' IMDb page.
fn imdb_from_link(href: &str) -> Option<String> {
    let id = href.split("/title/").nth(1)?.split(['/', '?']).next()?;
    (id != "none" && kura_domain::is_imdb_id(id)).then(|| id.to_owned())
}

fn sel(css: &'static str) -> Selector {
    Selector::parse(css).expect("selector literals are valid CSS")
}

/// Text of an element with whitespace collapsed.
fn text(el: ElementRef) -> String {
    el.text()
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Text of the first match that has any.
fn first_text(scope: ElementRef, css: &'static str) -> Option<String> {
    scope
        .select(&sel(css))
        .map(text)
        .find(|text| !text.is_empty())
}

fn first_attr<'a>(scope: ElementRef<'a>, css: &'static str, name: &str) -> Option<&'a str> {
    scope.select(&sel(css)).find_map(|el| el.value().attr(name))
}

fn absolute(base: &str, path: &str) -> String {
    if path.starts_with("http") {
        path.to_owned()
    } else {
        format!("{base}{path}")
    }
}

/// Walks one real series from search to hoster links.
#[cfg(test)]
async fn live_check(site: &dyn Site, query: &str) {
    assert!(!site.latest().await.unwrap().is_empty(), "latest is empty");
    assert!(
        site.all().await.unwrap().len() > 500,
        "the full list is short"
    );
    let genres = site.genres().await.unwrap();
    assert!(genres.len() > 10, "few genres: {genres:?}");
    for genre in [&genres[0], &genres[genres.len() - 1]] {
        let listed = site.by_genre(&genre.slug, 1).await.unwrap();
        assert!(!listed.is_empty(), "genre {genre:?} is empty");
    }
    // The first genre on both sites is large enough for a second page.
    let second = site.by_genre(&genres[0].slug, 2).await.unwrap();
    let first = site.by_genre(&genres[0].slug, 1).await.unwrap();
    assert!(
        !second.is_empty() && second[0] != first[0],
        "page 2 repeats page 1"
    );
    assert!(
        site.by_genre(&genres[0].slug, 9999)
            .await
            .unwrap()
            .is_empty()
    );

    let found = site.search(query).await.unwrap();
    assert!(!found.is_empty(), "search found nothing");
    assert!(
        !site.popular().await.unwrap().is_empty(),
        "popular is empty"
    );

    let series = site.series(&found[0].slug).await.unwrap();
    assert!(
        series.imdb_id.is_some(),
        "no IMDb ID for {}",
        series.summary.title
    );
    let season = *series.seasons.iter().find(|season| **season > 0).unwrap();
    let episodes = site.episodes(&found[0].slug, season).await.unwrap();
    assert!(
        !episodes[0].languages.is_empty(),
        "episode has no languages"
    );

    let episode = EpisodeId {
        site: site.id(),
        slug: found[0].slug.clone(),
        season,
        number: episodes[0].number,
    };
    let links = site.links(&episode).await.unwrap();
    assert!(!links.is_empty(), "episode has no hoster links");
    println!("{series:#?}\n{:#?}\n{links:#?}", episodes[0]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_cannot_leave_their_path() {
        assert!(check_slug("one-piece").is_ok());
        for bad in ["", "..", "a/b", "a?b=c", "a#b", "a b"] {
            assert!(check_slug(bad).is_err(), "{bad:?} was accepted");
        }
        assert_eq!(
            slug_from("/serie/north-pier", "/serie/").as_deref(),
            Some("north-pier")
        );
        assert_eq!(slug_from("/serie/north-pier/staffel-1", "/serie/"), None);
        assert_eq!(slug_from("/faqs", "/serie/"), None);
    }
}
