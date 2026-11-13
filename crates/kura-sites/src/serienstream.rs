use async_trait::async_trait;
use kura_domain::{Episode, EpisodeId, Genre, HosterLink, Language, Series, SeriesSummary, SiteId};
use scraper::{ElementRef, Html};

use crate::{
    Result, Site, SiteError, absolute, check_slug, first_attr, first_text, get, sel, slug_from,
    text, unique,
};

const BASE: &str = "https://serienstream.to";
const SERIES: &str = "/serie/";

pub struct SerienStream {
    http: reqwest::Client,
}

impl SerienStream {
    pub fn new(http: reqwest::Client) -> Self {
        Self { http }
    }
}

#[async_trait]
impl Site for SerienStream {
    fn id(&self) -> SiteId {
        SiteId::SerienStream
    }

    async fn search(&self, query: &str) -> Result<Vec<SeriesSummary>> {
        let html = self
            .http
            .get(format!("{BASE}/suche"))
            .query(&[("term", query)])
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        Ok(parse_cards(&html, ".search-results a.show-cover"))
    }

    async fn popular(&self) -> Result<Vec<SeriesSummary>> {
        let html = get(&self.http, &format!("{BASE}/beliebte-serien")).await?;
        Ok(parse_cards(&html, "a.show-card"))
    }

    async fn latest(&self) -> Result<Vec<SeriesSummary>> {
        let html = get(&self.http, &format!("{BASE}/neue-episoden")).await?;
        Ok(parse_latest(&html))
    }

    async fn genres(&self) -> Result<Vec<Genre>> {
        Ok(parse_genres(&get(&self.http, BASE).await?))
    }

    async fn by_genre(&self, genre: &str, page: u32) -> Result<Vec<SeriesSummary>> {
        let genre = check_slug(genre)?;
        let html = get(&self.http, &format!("{BASE}/genre/{genre}?page={page}")).await?;
        Ok(parse_genre_page(&html, page))
    }

    async fn all(&self) -> Result<Vec<SeriesSummary>> {
        Ok(parse_all(
            &get(&self.http, &format!("{BASE}/serien")).await?,
        ))
    }

    async fn series(&self, slug: &str) -> Result<Series> {
        let slug = check_slug(slug)?;
        let html = get(&self.http, &format!("{BASE}{SERIES}{slug}")).await?;
        parse_series(slug, &html)
    }

    async fn episodes(&self, slug: &str, season: u32) -> Result<Vec<Episode>> {
        let slug = check_slug(slug)?;
        let html = get(
            &self.http,
            &format!("{BASE}{SERIES}{slug}/staffel-{season}"),
        )
        .await?;
        Ok(parse_episodes(season, &html))
    }

    async fn links(&self, episode: &EpisodeId) -> Result<Vec<HosterLink>> {
        let url = format!(
            "{BASE}{SERIES}{}/staffel-{}/episode-{}",
            check_slug(&episode.slug)?,
            episode.season,
            episode.number
        );
        Ok(parse_links(&get(&self.http, &url).await?))
    }
}

/// Covers are lazy-loaded on some pages and plain on others.
fn cover(scope: ElementRef, css: &'static str) -> Option<String> {
    first_attr(scope, css, "data-src")
        .or_else(|| first_attr(scope, css, "src"))
        .map(|path| absolute(BASE, path))
}

/// Search results and the popular list are both cover links titled by the image's alt text.
fn parse_cards(html: &str, css: &'static str) -> Vec<SeriesSummary> {
    let mut cards: Vec<SeriesSummary> = Vec::new();
    for card in Html::parse_document(html).select(&sel(css)) {
        let summary = (|| {
            Some(SeriesSummary {
                downloaded: false,
                site: SiteId::SerienStream,
                slug: slug_from(card.value().attr("href")?, SERIES)?,
                title: first_attr(card, "img[alt]", "alt")?.to_owned(),
                cover_url: cover(card, "img"),
            })
        })();
        // The popular page shows a series in several rows.
        if let Some(summary) = summary.filter(|new| cards.iter().all(|old| old.slug != new.slug)) {
            cards.push(summary);
        }
    }
    cards
}

/// The site lists new episodes, not new series: the series of those episodes, newest first.
/// The list has no covers.
fn parse_latest(html: &str) -> Vec<SeriesSummary> {
    let doc = Html::parse_document(html);
    let selector = sel("table.new-episodes-table a[href]");
    let links = doc.select(&selector);
    unique(links.filter_map(|link| {
        // `/serie/<slug>/staffel-1/episode-6`
        let path = link.value().attr("href")?.strip_prefix(SERIES)?;
        let slug = check_slug(path.split('/').next()?).ok()?;
        Some(SeriesSummary {
            downloaded: false,
            site: SiteId::SerienStream,
            slug: slug.to_owned(),
            title: text(link),
            cover_url: None,
        })
    }))
}

/// Genre links, which the home page carries in its menu.
fn parse_genres(html: &str) -> Vec<Genre> {
    let mut genres: Vec<Genre> = Vec::new();
    for link in Html::parse_document(html).select(&sel(r#"a[href^="/genre/"]"#)) {
        let slug = link
            .value()
            .attr("href")
            .and_then(|href| slug_from(href, "/genre/"));
        let (Some(slug), name) = (slug, text(link)) else {
            continue;
        };
        if !name.is_empty() && genres.iter().all(|genre| genre.slug != slug) {
            genres.push(Genre { slug, name });
        }
    }
    genres
}

fn parse_all(html: &str) -> Vec<SeriesSummary> {
    let doc = Html::parse_document(html);
    let selector = sel(".series-item a[href]");
    let links = doc.select(&selector);
    unique(links.filter_map(|link| {
        Some(SeriesSummary {
            downloaded: false,
            site: SiteId::SerienStream,
            slug: slug_from(link.value().attr("href")?, SERIES)?,
            title: text(link),
            cover_url: None,
        })
    }))
}

/// A page number past the end shows the last page again; that counts as empty.
fn parse_genre_page(html: &str, page: u32) -> Vec<SeriesSummary> {
    let doc = Html::parse_document(html);
    let shown = first_text(doc.root_element(), ".pagination .page-item.active");
    if shown
        .and_then(|shown| shown.parse::<u32>().ok())
        .is_some_and(|shown| shown != page)
    {
        return Vec::new();
    }
    parse_cards(html, "a.show-card")
}

fn parse_series(slug: &str, html: &str) -> Result<Series> {
    let doc = Html::parse_document(html);
    let root = doc.root_element();
    let title = first_text(root, "h1").ok_or(SiteError::Layout("series title"))?;

    // Other series on the page have covers too; the right one carries this title.
    let cover_url = root
        .select(&sel("img[alt]"))
        .find(|img| img.value().attr("alt") == Some(title.as_str()))
        .and_then(|img| img.value().attr("data-src").or(img.value().attr("src")))
        .map(|path| absolute(BASE, path));

    Ok(Series {
        summary: SeriesSummary {
            downloaded: false,
            site: SiteId::SerienStream,
            slug: slug.to_owned(),
            title,
            cover_url,
        },
        description: first_text(root, ".description-text").unwrap_or_default(),
        seasons: root
            .select(&sel("#season-nav a[data-season-pill]"))
            .filter_map(|pill| pill.value().attr("data-season-pill")?.parse().ok())
            .collect(),
        imdb_id: root
            .select(&sel(r#"a[href*="imdb.com/title/"]"#))
            .find_map(|link| crate::imdb_from_link(link.value().attr("href")?)),
    })
}

fn parse_episodes(season: u32, html: &str) -> Vec<Episode> {
    Html::parse_document(html)
        .select(&sel("tr.episode-row"))
        .filter_map(|row| {
            Some(Episode {
                season,
                number: first_text(row, ".episode-number-cell")?.parse().ok()?,
                // Some episodes have no German title.
                title: first_text(row, ".episode-title-ger")
                    .or_else(|| first_text(row, ".episode-title-eng"))
                    .unwrap_or_default(),
                languages: row
                    .select(&sel("svg.watch-language"))
                    .filter_map(|flag| {
                        flag.value().classes().find_map(|class| match class {
                            "svg-flag-german" => Some(Language::GermanDub),
                            "svg-flag-english" => Some(Language::English),
                            "svg-flag-english-german" => Some(Language::GermanSub),
                            _ => None,
                        })
                    })
                    .collect(),
            })
        })
        .collect()
}

fn parse_links(html: &str) -> Vec<HosterLink> {
    Html::parse_document(html)
        .select(&sel("#episode-links [data-play-url]"))
        .filter_map(|button| {
            let attr = |name| button.value().attr(name);
            let language = match attr("data-language-id")? {
                "1" => Language::GermanDub,
                "2" => Language::English,
                "3" => Language::GermanSub,
                _ => return None,
            };
            Some(HosterLink {
                language,
                hoster: attr("data-provider-name")?.to_lowercase(),
                url: absolute(BASE, attr("data-play-url")?),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_lists_series_with_covers() {
        let series = parse_cards(
            include_str!("../fixtures/serienstream_search.html"),
            ".search-results a.show-cover",
        );
        let slugs: Vec<_> = series.iter().map(|series| series.slug.as_str()).collect();
        assert_eq!(slugs, ["north-pier", "the-pier-keepers"]);
        assert_eq!(series[0].title, "North Pier");
        assert_eq!(
            series[0].cover_url.as_deref(),
            Some("https://serienstream.to/media/images/channel/desktop/north-pier-aaa?format=jpg")
        );
    }

    #[test]
    fn popular_reads_lazy_covers() {
        let series = parse_cards(
            include_str!("../fixtures/serienstream_popular.html"),
            "a.show-card",
        );
        assert_eq!(series.len(), 2);
        assert_eq!(series[1].title, "Glass County");
        assert_eq!(
            series[1].cover_url.as_deref(),
            Some(
                "https://serienstream.to/media/images/channel/desktop/glass-county-ccc?format=jpg"
            )
        );
    }

    #[test]
    fn new_episodes_give_their_series_once() {
        let latest: Vec<_> = parse_latest(include_str!("../fixtures/serienstream_new.html"))
            .into_iter()
            .map(|series| (series.slug, series.title))
            .collect();
        assert_eq!(
            latest,
            [
                ("glass-county".to_owned(), "Glass County".to_owned()),
                ("north-pier".to_owned(), "North Pier".to_owned()),
            ]
        );
    }

    #[test]
    fn home_page_gives_genres_and_the_full_list_every_series() {
        let genres = parse_genres(include_str!("../fixtures/serienstream_home.html"));
        let names: Vec<_> = genres.iter().map(|genre| genre.name.as_str()).collect();
        let slugs: Vec<_> = genres.iter().map(|genre| genre.slug.as_str()).collect();
        assert_eq!(names, ["Abenteuer", "True Crime"]);
        assert_eq!(slugs, ["abenteuer", "true-crime"]);

        let all = parse_all(include_str!("../fixtures/serienstream_all.html"));
        let slugs: Vec<_> = all.iter().map(|series| series.slug.as_str()).collect();
        assert_eq!(slugs, ["glass-county", "north-pier"]);
        assert_eq!(all[0].title, "Glass County");
    }

    #[test]
    fn a_genre_page_past_the_end_is_empty() {
        let html = include_str!("../fixtures/serienstream_popular.html");
        let page = |active: u32| {
            format!(
                r#"<ul class="pagination"><li class="page-item active"><span>{active}</span></li></ul>{html}"#
            )
        };
        assert_eq!(parse_genre_page(&page(2), 2).len(), 2);
        assert!(parse_genre_page(&page(53), 9999).is_empty());
        // Without page numbers (a single page) the cards count.
        assert_eq!(parse_genre_page(html, 1).len(), 2);
    }

    #[test]
    fn series_page_gives_details_and_seasons() {
        let series = parse_series(
            "north-pier",
            include_str!("../fixtures/serienstream_series.html"),
        )
        .unwrap();
        assert_eq!(series.summary.title, "North Pier");
        assert_eq!(
            series.description,
            "A harbour town keeps its ferry running through one long winter."
        );
        assert_eq!(
            series.summary.cover_url.as_deref(),
            Some("https://serienstream.to/media/images/channel/desktop/north-pier-aaa?format=jpg")
        );
        assert_eq!(series.seasons, [0, 1, 2]);
        assert_eq!(series.imdb_id.as_deref(), Some("tt0944947"));
        let without = r#"<h1>X</h1><a href="https://www.imdb.com/title/abc/">IMDb</a>"#;
        assert_eq!(parse_series("x", without).unwrap().imdb_id, None);
    }

    #[test]
    fn season_table_gives_episodes_with_languages() {
        let episodes = parse_episodes(1, include_str!("../fixtures/serienstream_series.html"));
        assert_eq!(
            episodes,
            [
                Episode {
                    season: 1,
                    number: 1,
                    title: "Erste Fahrt".into(),
                    languages: vec![Language::GermanDub, Language::English],
                },
                Episode {
                    season: 1,
                    number: 2,
                    title: "Low Tide".into(),
                    languages: vec![Language::GermanSub],
                },
            ]
        );
    }

    #[test]
    fn episode_page_gives_hoster_links_per_language() {
        let links: Vec<_> = parse_links(include_str!("../fixtures/serienstream_episode.html"))
            .into_iter()
            .map(|link| (link.language, link.hoster, link.url))
            .collect();
        let url = |token| format!("https://serienstream.to/r?t=token-{token}");
        assert_eq!(
            links,
            [
                (Language::GermanDub, "voe".to_owned(), url("a")),
                (Language::English, "vidoza".to_owned(), url("b")),
                (Language::GermanSub, "voe".to_owned(), url("c")),
            ]
        );
    }

    /// Talks to the real site: `cargo test -p kura-sites -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn live_site_still_matches_the_parsers() {
        crate::live_check(
            &SerienStream::new(crate::http_client().unwrap()),
            "one piece",
        )
        .await;
    }
}
