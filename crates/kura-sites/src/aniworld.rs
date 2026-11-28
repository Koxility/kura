use std::{
    collections::HashMap,
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, Instant},
};

use async_trait::async_trait;
use kura_domain::{
    CalendarDay, CalendarEntry, Episode, EpisodeId, Genre, HosterLink, Language, NewEpisode,
    Series, SeriesSummary, SiteId,
};
use scraper::Html;
use serde::Deserialize;

use crate::{
    Result, Site, SiteError, absolute, check_slug, first_attr, first_text, get, sel, slug_from,
    text, unique,
};

const BASE: &str = "https://aniworld.to";
const SERIES: &str = "/anime/stream/";

/// The search sends no covers, so this many hits get theirs from their series page.
const COVER_LOOKUPS: usize = 24;

/// The home page lists the newest episodes; one fetch serves for this long.
const HOME_TTL: Duration = Duration::from_secs(5 * 60);

pub struct AniWorld {
    http: reqwest::Client,
    /// Covers read from series pages, by slug, so a search again needs no page.
    covers: Mutex<HashMap<String, String>>,
    /// The home page and when it was fetched.
    home: Mutex<Option<(Instant, Arc<String>)>>,
}

impl AniWorld {
    pub fn new(http: reqwest::Client) -> Self {
        Self {
            http,
            covers: Mutex::default(),
            home: Mutex::default(),
        }
    }

    /// The home page, from memory while it is fresh.
    async fn home(&self) -> Result<Arc<String>> {
        let lock = || self.home.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((at, html)) = lock().as_ref()
            && at.elapsed() < HOME_TTL
        {
            return Ok(html.clone());
        }
        let html = Arc::new(get(&self.http, &format!("{BASE}/")).await?);
        *lock() = Some((Instant::now(), html.clone()));
        Ok(html)
    }

    /// Fills in the covers of the first hits from the cache, or from their series pages,
    /// all at once. A page that fails leaves its hit without a cover.
    async fn add_covers(&self, list: &mut [SeriesSummary]) {
        let missing = {
            let covers = self.covers.lock().unwrap_or_else(PoisonError::into_inner);
            covers_from(&covers, list)
        };
        let lookups: Vec<_> = missing
            .into_iter()
            .map(|index| {
                let http = self.http.clone();
                let slug = list[index].slug.clone();
                let page = tokio::spawn(async move {
                    let html = get(&http, &format!("{BASE}{SERIES}{slug}")).await.ok()?;
                    parse_series(&slug, &html).ok()?.summary.cover_url
                });
                (index, page)
            })
            .collect();
        for (index, page) in lookups {
            if let Ok(Some(cover)) = page.await {
                let mut covers = self.covers.lock().unwrap_or_else(PoisonError::into_inner);
                covers.insert(list[index].slug.clone(), cover.clone());
                list[index].cover_url = Some(cover);
            }
        }
    }
}

/// Sets the covers the cache knows and returns the places of the first hits still without
/// one.
fn covers_from(covers: &HashMap<String, String>, list: &mut [SeriesSummary]) -> Vec<usize> {
    let mut missing = Vec::new();
    for (index, series) in list.iter_mut().enumerate().take(COVER_LOOKUPS) {
        if series.cover_url.is_some() {
            continue;
        }
        match covers.get(&series.slug) {
            Some(cover) => series.cover_url = Some(cover.clone()),
            None => missing.push(index),
        }
    }
    missing
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
        let mut found = parse_search(hits);
        self.add_covers(&mut found).await;
        Ok(found)
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

    async fn new_episodes(&self) -> Result<Vec<NewEpisode>> {
        let found = parse_new_episodes(&self.home().await?);
        if found.is_empty() {
            return Err(SiteError::Layout("newest episodes"));
        }
        Ok(found)
    }

    async fn feed(&self) -> Result<Vec<NewEpisode>> {
        let html = get(&self.http, &format!("{BASE}/neue-episoden")).await?;
        let found = parse_new_episodes(&html);
        if found.is_empty() {
            return Err(SiteError::Layout("new episodes"));
        }
        Ok(found)
    }

    async fn calendar(&self) -> Result<Vec<CalendarDay>> {
        let html = get(&self.http, &format!("{BASE}/animekalender")).await?;
        let days = parse_calendar(&html);
        if days.is_empty() {
            return Err(SiteError::Layout("calendar"));
        }
        Ok(days)
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
/// It sends no covers; [`AniWorld::add_covers`] adds them.
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
            // Lazy-loaded on most pages, plain on some.
            cover_url: first_attr(root, ".seriesCoverBox img", "data-src")
                .or_else(|| first_attr(root, ".seriesCoverBox img", "src"))
                .filter(|path| !path.starts_with("data:"))
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
                languages: row_languages(row),
            })
        })
        .collect()
}

/// The flags of a season table row. Lazily loaded ones carry the real address in `data-src`
/// and a placeholder in `src`; each language counts once, also when a `<noscript>` copy
/// repeats it.
fn row_languages(row: scraper::ElementRef) -> Vec<Language> {
    let mut languages = Vec::new();
    for flag in row.select(&sel("img.flag")) {
        let flag = flag.value();
        let src = flag.attr("data-src").or_else(|| flag.attr("src"));
        if let Some(language) = src.and_then(flag_language)
            && !languages.contains(&language)
        {
            languages.push(language);
        }
    }
    languages
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

/// The list "Die 50 neuesten Episoden" on the home page, and the page "Neue Episoden", which
/// has the same rows and reaches further back.
fn parse_new_episodes(html: &str) -> Vec<NewEpisode> {
    Html::parse_document(html)
        .select(&sel(".newEpisodeList .row"))
        .filter_map(|row| {
            let href = first_attr(row, "a[href]", "href")?;
            let (slug, season, number) = episode_from_href(href)?;
            // `Sa, 21.11.2026 19:05 Uhr`: the weekday and the unit go.
            let when = first_text(row, ".elementFloatRight")?;
            let when = when
                .split_once(", ")
                .map_or(when.as_str(), |(_, rest)| rest);
            let released = when.trim_end_matches("Uhr").trim().to_owned();
            // The flags load lazily; the real address is in `data-src`.
            let flag = first_attr(row, "img.flag[data-src]", "data-src")
                .or_else(|| first_attr(row, "img.flag[src]", "src"));
            Some(NewEpisode {
                site: SiteId::AniWorld,
                slug,
                title: first_text(row, "strong")?,
                season,
                number,
                language: flag.and_then(flag_language),
                released,
            })
        })
        .collect()
}

/// The page "Animekalender": one section per day, today first, then the next six days.
fn parse_calendar(html: &str) -> Vec<CalendarDay> {
    Html::parse_document(html)
        .select(&sel("section.calendarList[id]"))
        .filter_map(|section| {
            let weekday = weekday(section.value().id()?)?;
            // `Samstag, 21.11.2026 (heute)`.
            let heading = first_text(section, "h3")?;
            let date = heading
                .split_whitespace()
                .find(|word| word.len() == 10 && word.chars().filter(|c| *c == '.').count() == 2)?
                .to_owned();
            Some(CalendarDay {
                weekday,
                date,
                today: heading.contains("(heute)"),
                entries: section
                    .select(&sel(".seriesListContainer a[href]"))
                    .filter_map(calendar_entry)
                    .collect(),
            })
        })
        .collect()
}

fn calendar_entry(card: scraper::ElementRef) -> Option<CalendarEntry> {
    let href = card.value().attr("href")?;
    let path = href.strip_prefix(BASE).unwrap_or(href);
    // An episode that is out links to itself, one that is not to its series.
    let slug =
        slug_from(path, SERIES).or_else(|| episode_from_href(path).map(|(slug, ..)| slug))?;
    let smalls: Vec<_> = card.select(&sel("small")).collect();
    let (season, number) = episode_code(&text(*smalls.first()?))?;
    let time = text(*smalls.get(1)?);
    let time = time.trim_start_matches('~').trim_end_matches("Uhr").trim();
    // The covers load lazily; the real address is in `data-src`.
    let cover = first_attr(card, "img[data-src*='/cover/']", "data-src")
        .or_else(|| first_attr(card, "img[src*='/cover/']", "src"))
        .filter(|src| src.starts_with('/') || src.starts_with("http"));
    let flag = smalls[0].select(&sel("img.flag")).find_map(|img| {
        let value = img.value();
        value.attr("data-src").or_else(|| value.attr("src"))
    });
    Some(CalendarEntry {
        slug,
        title: first_text(card, "h3.seriesTitle")?,
        cover_url: cover.map(|src| absolute(BASE, src)),
        season,
        number,
        time: time.to_owned(),
        language: flag.and_then(flag_language),
        online: card
            .select(&sel("[title='Stream online!']"))
            .next()
            .is_some(),
    })
}

/// The section ids name the weekdays in German.
fn weekday(id: &str) -> Option<u8> {
    let days = [
        "montag",
        "dienstag",
        "mittwoch",
        "donnerstag",
        "freitag",
        "samstag",
        "sonntag",
    ];
    let index = days.iter().position(|day| *day == id)?;
    u8::try_from(index + 1).ok()
}

/// `S02E07` is season 2, episode 7; `Film 22` is film 22, kept as season 0.
fn episode_code(text: &str) -> Option<(u32, u32)> {
    let code = text.split_whitespace().collect::<Vec<_>>();
    if let ["Film", number, ..] = code.as_slice() {
        return Some((0, number.parse().ok()?));
    }
    let (season, number) = code.first()?.strip_prefix('S')?.split_once('E')?;
    Some((season.parse().ok()?, number.parse().ok()?))
}

/// `/anime/stream/<slug>/staffel-2/episode-8`, or `/filme/film-3` for season 0.
fn episode_from_href(href: &str) -> Option<(String, u32, u32)> {
    let rest = href.strip_prefix(SERIES)?;
    let mut parts = rest.split('/');
    let slug = check_slug(parts.next()?).ok()?.to_owned();
    let season = match parts.next()? {
        "filme" => 0,
        season => season.strip_prefix("staffel-")?.parse().ok()?,
    };
    let number = parts.next()?.rsplit('-').next()?.parse().ok()?;
    Some((slug, season, number))
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
    fn search_hits_take_known_covers_and_list_the_rest() {
        let hits = serde_json::from_str(include_str!("../fixtures/aniworld_search.json")).unwrap();
        let mut found = parse_search(hits);
        let cover = "https://aniworld.to/public/img/cover/lantern-road_220x330.jpg";
        let covers = HashMap::from([("lantern-road".to_owned(), cover.to_owned())]);
        assert_eq!(covers_from(&covers, &mut found), [1]);
        assert_eq!(found[0].cover_url.as_deref(), Some(cover));
        assert_eq!(found[1].cover_url, None);
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
                // Flags that load lazily.
                Episode {
                    season: 1,
                    number: 3,
                    title: "Die Brücke".into(),
                    languages: vec![Language::GermanDub, Language::GermanSub],
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

    #[test]
    fn home_page_gives_the_newest_episodes() {
        let found = parse_new_episodes(include_str!("../fixtures/aniworld_home.html"));
        assert_eq!(found.len(), 3);
        assert_eq!(
            found[0],
            NewEpisode {
                site: SiteId::AniWorld,
                slug: "lantern-road".into(),
                title: "Lantern Road: The Long Way Home".into(),
                season: 1,
                number: 8,
                language: Some(Language::EnglishSub),
                released: "21.11.2026 19:05".into(),
            }
        );
        assert_eq!(
            (found[1].number, found[1].language),
            (1150, Some(Language::GermanDub))
        );
        // Films are season 0.
        assert_eq!(
            (found[2].season, found[2].number, found[2].language),
            (0, 16, Some(Language::GermanSub))
        );
    }

    #[test]
    fn new_episodes_page_gives_one_entry_per_episode_and_language() {
        let found = parse_new_episodes(include_str!("../fixtures/aniworld_feed.html"));
        let entries: Vec<_> = found
            .iter()
            .map(|entry| {
                let at = (entry.slug.as_str(), entry.season, entry.number);
                (at, entry.language, entry.released.as_str())
            })
            .collect();
        assert_eq!(
            entries,
            [
                (
                    ("lantern-road", 1, 9),
                    Some(Language::GermanDub),
                    "28.11.2026"
                ),
                (
                    ("lantern-road", 1, 9),
                    Some(Language::GermanSub),
                    "28.11.2026"
                ),
                (
                    ("glass-harbor", 2, 12),
                    Some(Language::GermanSub),
                    "27.11.2026"
                ),
                (
                    ("glass-harbor", 2, 1),
                    Some(Language::GermanSub),
                    "27.11.2026"
                ),
                (
                    ("lantern-road", 0, 3),
                    Some(Language::EnglishSub),
                    "26.11.2026"
                ),
            ]
        );
        assert_eq!(found[2].title, "Glass Harbor: The Second Tide");
    }

    #[test]
    fn calendar_page_gives_the_days_with_their_episodes() {
        let days = parse_calendar(include_str!("../fixtures/aniworld_calendar.html"));
        let summary: Vec<_> = days
            .iter()
            .map(|day| (day.weekday, day.date.as_str(), day.today, day.entries.len()))
            .collect();
        assert_eq!(
            summary,
            [
                (6, "21.11.2026", true, 2),
                (7, "22.11.2026", false, 1),
                (4, "26.11.2026", false, 1)
            ]
        );
        assert_eq!(
            days[0].entries[0],
            CalendarEntry {
                slug: "keroro".into(),
                title: "KERORO☆".into(),
                cover_url: Some(
                    "https://aniworld.to/public/img/cover/keroro-stream-cover-GsXmMdD4FdyHojPhXtl49AUHkeQ5nH9v_200x300.png"
                        .into()
                ),
                season: 1,
                number: 8,
                time: "02:10".into(),
                language: Some(Language::GermanSub),
                online: true,
            }
        );
        // The same episode once more, in another language.
        assert_eq!(days[0].entries[1].language, Some(Language::EnglishSub));
        // Not out yet: the link names only the series.
        let upcoming = &days[1].entries[0];
        assert_eq!(
            (
                upcoming.slug.as_str(),
                upcoming.season,
                upcoming.number,
                upcoming.online
            ),
            ("blue-box", 2, 7, false)
        );
        assert_eq!(upcoming.language, Some(Language::GermanDub));
        let film = &days[2].entries[0];
        assert_eq!((film.season, film.number), (0, 22));
    }

    #[test]
    fn episode_codes_name_season_and_episode() {
        assert_eq!(episode_code("S02E07"), Some((2, 7)));
        assert_eq!(episode_code("S32E155 "), Some((32, 155)));
        assert_eq!(episode_code("Film 22"), Some((0, 22)));
        assert_eq!(episode_code("Folge"), None);
    }

    #[test]
    fn episode_links_give_slug_season_and_number() {
        assert_eq!(
            episode_from_href("/anime/stream/one-piece/staffel-3/episode-12"),
            Some(("one-piece".into(), 3, 12))
        );
        assert_eq!(
            episode_from_href("/anime/stream/one-piece/filme/film-2"),
            Some(("one-piece".into(), 0, 2))
        );
        assert_eq!(episode_from_href("/anime/stream/one-piece"), None);
    }

    /// Talks to the real site: `cargo test -p kura-sites -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn live_site_still_matches_the_parsers() {
        crate::live_check(&AniWorld::new(crate::http_client().unwrap()), "one piece").await;
    }
}
