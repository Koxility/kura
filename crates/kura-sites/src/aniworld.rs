use async_trait::async_trait;
use kura_domain::{Episode, EpisodeId, Genre, HosterLink, Language, Series, SeriesSummary, SiteId};
use scraper::Html;
use serde::Deserialize;

use crate::{
    Result, Site, SiteError, absolute, check_slug, first_attr, first_text, get, sel, slug_from,
    text, unique,
};

const BASE: &str = "https://aniworld.to";
const SERIES: &str = "/anime/stream/";

pub struct AniWorld {
    http: reqwest::Client,
}

impl AniWorld {
    pub fn new(http: reqwest::Client) -> Self {
        Self { http }
    }
}

#[async_trait]
impl Site for AniWorld {
    fn id(&self) -> SiteId {
        SiteId::AniWorld
    }

    async fn search(&self, query: &str) -> Result<Vec<SeriesSummary>> {
        let hits = self
            .http
            .post(format!("{BASE}/ajax/search"))
            .form(&[("keyword", query)])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(parse_search(hits))
    }

    async fn popular(&self) -> Result<Vec<SeriesSummary>> {
        let html = get(&self.http, &format!("{BASE}/beliebte-animes")).await?;
        Ok(parse_popular(&html))
    }

    async fn latest(&self) -> Result<Vec<SeriesSummary>> {
        let html = get(&self.http, &format!("{BASE}/neu")).await?;
        Ok(parse_popular(&html))
    }

    async fn genres(&self) -> Result<Vec<Genre>> {
        Ok(parse_genres(
            &get(&self.http, &format!("{BASE}/animes")).await?,
        ))
    }

    async fn by_genre(&self, genre: &str, page: u32) -> Result<Vec<SeriesSummary>> {
        let genre = check_slug(genre)?;
        let answer = self
            .http
            .get(format!("{BASE}/genre/{genre}/{page}"))
            .send()
            .await?;
        // A page past the end is a 404.
        if answer.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(Vec::new());
        }
        let html = answer.error_for_status()?.text().await?;
        Ok(parse_popular(&html))
    }

    async fn all(&self) -> Result<Vec<SeriesSummary>> {
        Ok(parse_all(
            &get(&self.http, &format!("{BASE}/animes")).await?,
        ))
    }

    async fn series(&self, slug: &str) -> Result<Series> {
        let slug = check_slug(slug)?;
        let html = get(&self.http, &format!("{BASE}{SERIES}{slug}")).await?;
        parse_series(slug, &html)
    }

    async fn episodes(&self, slug: &str, season: u32) -> Result<Vec<Episode>> {
        let html = get(&self.http, &season_url(check_slug(slug)?, season)).await?;
        Ok(parse_episodes(season, &html))
    }

    async fn links(&self, episode: &EpisodeId) -> Result<Vec<HosterLink>> {
        let season = season_url(check_slug(&episode.slug)?, episode.season);
        let kind = if episode.season == 0 {
            "film"
        } else {
            "episode"
        };
        let html = get(&self.http, &format!("{season}/{kind}-{}", episode.number)).await?;
        Ok(parse_links(&html))
    }
}

/// Films live under their own path instead of a season number.
fn season_url(slug: &str, season: u32) -> String {
    if season == 0 {
        format!("{BASE}{SERIES}{slug}/filme")
    } else {
        format!("{BASE}{SERIES}{slug}/staffel-{season}")
    }
}

#[derive(Deserialize)]
struct Hit {
    title: String,
    link: String,
}

/// The search also returns episodes and help pages; only links to a series page are kept.
/// It sends no covers.
fn parse_search(hits: Vec<Hit>) -> Vec<SeriesSummary> {
    hits.into_iter()
        .filter_map(|hit| {
            Some(SeriesSummary {
                downloaded: false,
                site: SiteId::AniWorld,
                slug: slug_from(&hit.link, SERIES)?,
                // Titles carry <em> around the match and HTML entities.
                title: Html::parse_fragment(&hit.title)
                    .root_element()
                    .text()
                    .collect(),
                cover_url: None,
            })
        })
        .collect()
}

fn parse_popular(html: &str) -> Vec<SeriesSummary> {
    Html::parse_document(html)
        .select(&sel(".seriesListContainer a[href]"))
        .filter_map(|card| {
            Some(SeriesSummary {
                downloaded: false,
                site: SiteId::AniWorld,
                slug: slug_from(card.value().attr("href")?, SERIES)?,
                title: first_text(card, "h3")?,
                cover_url: first_attr(card, "img[data-src]", "data-src")
                    .map(|path| absolute(BASE, path)),
            })
        })
        .collect()
}

/// The full list groups its series under genre headings.
fn parse_genres(html: &str) -> Vec<Genre> {
    Html::parse_document(html)
        .select(&sel(".seriesGenreList h3"))
        .map(text)
        .filter(|name| !name.is_empty())
        .map(|name| Genre {
            slug: genre_slug(&name),
            name,
        })
        .collect()
}

/// The site's genre addresses are the lower-case name with umlauts written out and spaces
/// as hyphens: "Romantische Komödie" is `romantische-komoedie`.
fn genre_slug(name: &str) -> String {
    name.to_lowercase()
        .replace('ä', "ae")
        .replace('ö', "oe")
        .replace('ü', "ue")
        .replace('ß', "ss")
        .replace(' ', "-")
}

fn parse_all(html: &str) -> Vec<SeriesSummary> {
    let doc = Html::parse_document(html);
    let selector = sel("#seriesContainer a[href]");
    let links = doc.select(&selector);
    unique(links.filter_map(|link| {
        Some(SeriesSummary {
            downloaded: false,
            site: SiteId::AniWorld,
            slug: slug_from(link.value().attr("href")?, SERIES)?,
            title: text(link),
            cover_url: None,
        })
    }))
}

fn parse_series(slug: &str, html: &str) -> Result<Series> {
    let doc = Html::parse_document(html);
    let root = doc.root_element();
    let title = first_text(root, ".series-title h1").ok_or(SiteError::Layout("series title"))?;

    let mut seasons = Vec::new();
    for link in root.select(&sel("#stream a[href]")) {
        let last = link
            .value()
            .attr("href")
            .and_then(|href| href.rsplit('/').next())
            .unwrap_or_default();
        let season = match last.strip_prefix("staffel-") {
            Some(number) => number.parse().ok(),
            None => (last == "filme").then_some(0),
        };
        if let Some(season) = season.filter(|season| !seasons.contains(season)) {
            seasons.push(season);
        }
    }

    Ok(Series {
        summary: SeriesSummary {
            downloaded: false,
            site: SiteId::AniWorld,
            slug: slug.to_owned(),
            title,
            cover_url: first_attr(root, ".seriesCoverBox img", "data-src")
                .map(|path| absolute(BASE, path)),
        },
        description: first_attr(root, "p.seri_des", "data-full-description")
            .unwrap_or_default()
            .to_owned(),
        seasons,
        imdb_id: first_attr(root, "a.imdb-link[data-imdb]", "data-imdb")
            .and_then(|id| crate::imdb_from_link(&format!("/title/{id}"))),
    })
}

fn parse_episodes(season: u32, html: &str) -> Vec<Episode> {
    Html::parse_document(html)
        .select(&sel("table.seasonEpisodesList tbody tr"))
        .filter_map(|row| {
            // Ends in `episode-7` or `film-7`.
            let href = first_attr(row, "a[href]", "href")?;
            let number = href.rsplit('-').next()?.parse().ok()?;
            Some(Episode {
                season,
                number,
                // Some episodes have no German title.
                title: first_text(row, ".seasonEpisodeTitle strong")
                    .or_else(|| first_text(row, ".seasonEpisodeTitle span"))
                    .unwrap_or_default(),
                languages: row
                    .select(&sel("img.flag[src]"))
                    .filter_map(|flag| flag_language(flag.value().attr("src")?))
                    .collect(),
            })
        })
        .collect()
}

fn flag_language(src: &str) -> Option<Language> {
    // Order matters: the subtitle flags contain the plain language names.
    [
        ("japanese-german", Language::GermanSub),
        ("japanese-english", Language::EnglishSub),
        ("german", Language::GermanDub),
        ("english", Language::English),
    ]
    .into_iter()
    .find_map(|(name, language)| src.contains(name).then_some(language))
}

fn parse_links(html: &str) -> Vec<HosterLink> {
    Html::parse_document(html)
        .select(&sel("li[data-link-target]"))
        .filter_map(|item| {
            let language = match item.value().attr("data-lang-key")? {
                "1" => Language::GermanDub,
                "2" => Language::EnglishSub,
                "3" => Language::GermanSub,
                _ => return None,
            };
            Some(HosterLink {
                language,
                hoster: first_text(item, "h4")?.to_lowercase(),
                url: absolute(BASE, item.value().attr("data-link-target")?),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_keeps_series_and_cleans_titles() {
        let hits = serde_json::from_str(include_str!("../fixtures/aniworld_search.json")).unwrap();
        let found: Vec<_> = parse_search(hits)
            .into_iter()
            .map(|series| (series.slug, series.title))
            .collect();
        assert_eq!(
            found,
            [
                ("lantern-road".to_owned(), "Lantern Road".to_owned()),
                (
                    "paper-lantern-keeper".to_owned(),
                    "Tale of the Paper Lantern's Keeper".to_owned()
                ),
            ]
        );
    }

    #[test]
    fn popular_lists_series_with_covers() {
        let series = parse_popular(include_str!("../fixtures/aniworld_popular.html"));
        assert_eq!(series.len(), 2);
        assert_eq!(series[0].slug, "lantern-road");
        assert_eq!(series[0].title, "Lantern Road");
        assert_eq!(
            series[0].cover_url.as_deref(),
            Some("https://aniworld.to/public/img/cover/lantern-road_200x300.png")
        );
    }

    #[test]
    fn full_list_gives_genres_and_every_series_once() {
        let html = include_str!("../fixtures/aniworld_all.html");
        let genres: Vec<_> = parse_genres(html)
            .into_iter()
            .map(|genre| (genre.slug, genre.name))
            .collect();
        assert_eq!(
            genres,
            [
                ("abenteuer".to_owned(), "Abenteuer".to_owned()),
                (
                    "romantische-komoedie".to_owned(),
                    "Romantische Komödie".to_owned()
                ),
            ]
        );

        let all: Vec<_> = parse_all(html)
            .into_iter()
            .map(|series| (series.slug, series.title))
            .collect();
        assert_eq!(
            all,
            [
                ("lantern-road".to_owned(), "Lantern Road".to_owned()),
                ("quiet-harbor".to_owned(), "Quiet Harbor".to_owned()),
            ]
        );
    }

    #[test]
    fn series_page_gives_details_and_seasons() {
        let series = parse_series(
            "lantern-road",
            include_str!("../fixtures/aniworld_series.html"),
        )
        .unwrap();
        assert_eq!(series.summary.title, "Lantern Road");
        assert_eq!(
            series.description,
            "A courier walks the old road between two towns."
        );
        assert_eq!(
            series.summary.cover_url.as_deref(),
            Some("https://aniworld.to/public/img/cover/lantern-road_220x330.jpg")
        );
        assert_eq!(series.seasons, [0, 1, 2]);
        assert_eq!(series.imdb_id.as_deref(), Some("tt0388629"));
        let without = r#"<div class="series-title"><h1><span>X</span></h1></div>"#;
        assert_eq!(parse_series("x", without).unwrap().imdb_id, None);
    }

    #[test]
    fn a_page_without_a_title_is_a_layout_error() {
        assert!(matches!(
            parse_series("x", "<html></html>"),
            Err(SiteError::Layout(_))
        ));
    }

    #[test]
    fn season_table_gives_episodes_with_languages() {
        let episodes = parse_episodes(1, include_str!("../fixtures/aniworld_series.html"));
        assert_eq!(
            episodes,
            [
                Episode {
                    season: 1,
                    number: 1,
                    title: "Der erste Schritt".into(),
                    languages: vec![
                        Language::GermanDub,
                        Language::GermanSub,
                        Language::EnglishSub
                    ],
                },
                Episode {
                    season: 1,
                    number: 2,
                    title: "Rain on the Pass".into(),
                    languages: vec![Language::GermanSub],
                },
            ]
        );
    }

    #[test]
    fn episode_page_gives_hoster_links_per_language() {
        let links: Vec<_> = parse_links(include_str!("../fixtures/aniworld_episode.html"))
            .into_iter()
            .map(|link| (link.language, link.hoster, link.url))
            .collect();
        let url = |id| format!("https://aniworld.to/redirect/{id}");
        assert_eq!(
            links,
            [
                (Language::GermanDub, "voe".to_owned(), url(501)),
                (Language::GermanDub, "filemoon".to_owned(), url(502)),
                (Language::GermanSub, "voe".to_owned(), url(503)),
                (Language::EnglishSub, "vidmoly".to_owned(), url(504)),
            ]
        );
    }

    #[test]
    fn films_have_their_own_urls() {
        assert_eq!(
            season_url("one-piece", 0),
            "https://aniworld.to/anime/stream/one-piece/filme"
        );
        assert_eq!(
            season_url("one-piece", 3),
            "https://aniworld.to/anime/stream/one-piece/staffel-3"
        );
    }

    /// Talks to the real site: `cargo test -p kura-sites -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn live_site_still_matches_the_parsers() {
        crate::live_check(&AniWorld::new(crate::http_client().unwrap()), "one piece").await;
    }
}
