//! File names from the naming template, e.g. `{title} {episodenumber} {language}` →
//! `One Piece S01E001 German Dub`.

use kura_domain::{Language, Settings};

use crate::{CoreError, Result, safe_name};

pub const DEFAULT_TEMPLATE: &str = "{title} {episodenumber} {language}";

const VARIABLES: [&str; 6] = [
    "title",
    "episodenumber",
    "language",
    "season",
    "episode",
    "imdbid",
];

/// The template that applies: the one from the settings, else `KURA_NAMING_TEMPLATE`, else
/// the built-in one.
pub fn effective_template(settings: &Settings) -> String {
    settings
        .naming_template
        .clone()
        .unwrap_or_else(default_template)
}

/// What applies while the settings name no template.
pub fn default_template() -> String {
    match std::env::var("KURA_NAMING_TEMPLATE") {
        Ok(template) if check_template(&template).is_ok() => template,
        Ok(template) => {
            tracing::warn!("KURA_NAMING_TEMPLATE {template:?} is not usable; using the default");
            DEFAULT_TEMPLATE.to_owned()
        }
        Err(_) => DEFAULT_TEMPLATE.to_owned(),
    }
}

/// Refuses templates that would give two episodes the same name or name nothing Kura knows.
pub fn check_template(template: &str) -> Result<()> {
    let names = variables(template).ok_or(CoreError::Invalid(
        "the naming template has an unclosed { or a stray }",
    ))?;
    if names.iter().any(|name| !VARIABLES.contains(&name.as_str())) {
        return Err(CoreError::Invalid(
            "the naming template knows {title}, {episodenumber}, {language}, {season}, \
             {episode} and {imdbid}",
        ));
    }
    let has = |name: &str| names.iter().any(|other| other == name);
    if !has("episodenumber") && !(has("season") && has("episode")) {
        return Err(CoreError::Invalid(
            "the naming template needs {episodenumber}, or {season} and {episode}",
        ));
    }
    Ok(())
}

/// The names inside `{…}`; `None` when braces do not pair up.
fn variables(template: &str) -> Option<Vec<String>> {
    let mut names = Vec::new();
    let mut rest = template;
    while let Some(open) = rest.find(['{', '}']) {
        if rest[open..].starts_with('}') {
            return None;
        }
        let close = rest[open..].find('}')? + open;
        let name = &rest[open + 1..close];
        if name.contains('{') {
            return None;
        }
        names.push(name.to_owned());
        rest = &rest[close + 1..];
    }
    Some(names)
}

/// What one episode's file is called, without the extension.
pub struct NameParts<'a> {
    pub title: &'a str,
    pub season: u32,
    pub episode: u32,
    pub language: Language,
    pub imdb_id: Option<&'a str>,
}

/// Fills in the template. The result is safe as a file name; `None` only when nothing usable
/// is left, which a checked template cannot produce.
pub fn render(template: &str, parts: &NameParts) -> Option<String> {
    let season = format!("{:02}", parts.season);
    let episode = format!("{:03}", parts.episode);
    let filled = template
        .replace("{title}", parts.title)
        .replace("{episodenumber}", &format!("S{season}E{episode}"))
        .replace("{language}", language_name(parts.language))
        .replace("{season}", &season)
        .replace("{episode}", &episode)
        .replace("{imdbid}", parts.imdb_id.unwrap_or("none"));
    safe_name(&filled)
}

pub fn language_name(language: Language) -> &'static str {
    match language {
        Language::GermanDub => "German Dub",
        Language::GermanSub => "German Sub",
        Language::EnglishSub => "English Sub",
        Language::English => "English",
    }
}

/// The language a file name carries, as `{language}` writes it: `One Piece S01E001 German
/// Dub.mkv` is German dub. Case and separators do not matter. Plain `English` only counts as
/// the last word, so a title like `English Teacher` is not taken for one.
pub fn language_in(name: &str) -> Option<Language> {
    let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    let words: Vec<String> = stem
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect();
    let pairs = [
        (Language::GermanDub, ["german", "dub"]),
        (Language::GermanSub, ["german", "sub"]),
        (Language::EnglishSub, ["english", "sub"]),
    ];
    let found = pairs.into_iter().find(|(_, pair)| {
        words
            .windows(2)
            .any(|window| window[0] == pair[0] && window[1] == pair[1])
    });
    match found {
        Some((language, _)) => Some(language),
        None => (words.last().map(String::as_str) == Some("english")).then_some(Language::English),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts(season: u32, episode: u32) -> NameParts<'static> {
        NameParts {
            title: "One Piece",
            season,
            episode,
            language: Language::GermanDub,
            imdb_id: Some("tt0388629"),
        }
    }

    #[test]
    fn episode_numbers_have_two_season_and_three_episode_digits() {
        let name = |season, episode| render(DEFAULT_TEMPLATE, &parts(season, episode));
        assert_eq!(name(1, 1).as_deref(), Some("One Piece S01E001 German Dub"));
        assert_eq!(
            name(10, 120).as_deref(),
            Some("One Piece S10E120 German Dub")
        );
        assert_eq!(name(0, 3).as_deref(), Some("One Piece S00E003 German Dub"));
        assert_eq!(
            name(1, 1100).as_deref(),
            Some("One Piece S01E1100 German Dub")
        );
    }

    #[test]
    fn every_variable_is_filled_and_path_characters_are_removed() {
        let template = "{title} - {season}x{episode} [{imdbid}] ({language})";
        let english = NameParts {
            language: Language::EnglishSub,
            imdb_id: None,
            ..parts(2, 7)
        };
        let name = render(template, &english);
        assert_eq!(
            name.as_deref(),
            Some("One Piece - 02x007 [none] (English Sub)")
        );

        let sneaky = NameParts {
            title: "../../etc/Re:Zero",
            ..parts(1, 1)
        };
        assert_eq!(
            render("{title}/{episodenumber}", &sneaky).as_deref(),
            Some("etc Re Zero S01E001")
        );
    }

    #[test]
    fn the_language_is_read_back_from_a_file_name() {
        for language in [
            Language::GermanDub,
            Language::GermanSub,
            Language::EnglishSub,
            Language::English,
        ] {
            let name = render(
                DEFAULT_TEMPLATE,
                &NameParts {
                    language,
                    ..parts(1, 1)
                },
            );
            assert_eq!(
                language_in(&format!("{}.mkv", name.unwrap())),
                Some(language)
            );
        }
        assert_eq!(
            language_in("show.s01e02.german.dub.mp4"),
            Some(Language::GermanDub)
        );
        assert_eq!(language_in("English Teacher S01E01.mkv"), None);
        assert_eq!(language_in("One Piece S01E001.mkv"), None);
    }

    #[test]
    fn templates_that_would_clash_or_are_unknown_are_refused() {
        for good in [
            DEFAULT_TEMPLATE,
            "{title} - {episodenumber}",
            "{season}{episode}",
        ] {
            assert!(check_template(good).is_ok(), "{good}");
        }
        for bad in [
            "",
            "{title}",
            "{title} {season}",
            "{title} {episodenumber} {quality}",
            "{title {episodenumber}",
            "{title} {episodenumber}}",
        ] {
            assert!(check_template(bad).is_err(), "{bad:?} was accepted");
        }
    }

    #[test]
    fn the_setting_wins_over_the_default() {
        let mut settings = Settings::default();
        assert!(check_template(&effective_template(&settings)).is_ok());
        settings.naming_template = Some("{title} {episodenumber}".to_owned());
        assert_eq!(effective_template(&settings), "{title} {episodenumber}");
    }
}
