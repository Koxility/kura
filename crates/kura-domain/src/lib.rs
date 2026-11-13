//! Plain types shared by every Kura crate. No I/O.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// A stored or submitted value that matches no variant of a text enum.
#[derive(Debug, thiserror::Error)]
#[error("unknown value: {0}")]
pub struct UnknownText(pub String);

/// Declares an enum whose variants each have one fixed text form, used in JSON and in the database.
macro_rules! text_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident => $text:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub enum $name {
            $(#[serde(rename = $text)] $variant),+
        }

        impl $name {
            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $text),+
                }
            }
        }

        impl std::str::FromStr for $name {
            type Err = UnknownText;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s {
                    $($text => Ok(Self::$variant),)+
                    _ => Err(UnknownText(s.to_owned())),
                }
            }
        }
    };
}

text_enum! {
    /// A site that lists series and links to hosters. It hosts no video itself.
    SiteId { AniWorld => "aniworld", SerienStream => "serienstream" }
}

text_enum! {
    /// Audio and subtitle combination an episode is offered in.
    Language {
        GermanDub => "german_dub",
        GermanSub => "german_sub",
        EnglishSub => "english_sub",
        // English audio without subtitles.
        English => "english",
    }
}

text_enum! {
    Role { Admin => "admin", User => "user" }
}

text_enum! {
    /// Container of a downloaded file. Streams are copied into it without re-encoding.
    OutputFormat { Mkv => "mkv", Mp4 => "mp4" }
}

text_enum! {
    JobState {
        Queued => "queued",
        Running => "running",
        Done => "done",
        Failed => "failed",
        Cancelled => "cancelled",
    }
}

/// A series as it appears in search results and lists.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SeriesSummary {
    pub site: SiteId,
    /// The site's own identifier for the series, taken from its URL.
    pub slug: String,
    pub title: String,
    pub cover_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Series {
    pub summary: SeriesSummary,
    pub description: String,
    /// Season numbers in site order. Season 0 holds films and specials.
    pub seasons: Vec<u32>,
    /// Like `tt0388629`, when the site links the series to IMDb.
    pub imdb_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Episode {
    pub season: u32,
    pub number: u32,
    pub title: String,
    pub languages: Vec<Language>,
}

/// Identifies one episode across sites.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EpisodeId {
    pub site: SiteId,
    pub slug: String,
    pub season: u32,
    pub number: u32,
}

/// A genre as a site names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Genre {
    /// The site's identifier, taken from its URL.
    pub slug: String,
    pub name: String,
}

/// One way to watch an episode: a hoster's page for one language.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HosterLink {
    pub language: Language,
    /// Lower-case hoster name as the site spells it, e.g. `voe`.
    pub hoster: String,
    pub url: String,
}

/// A direct media URL a hoster resolved to, with the headers its server insists on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stream {
    pub url: String,
    pub headers: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Job {
    pub id: i64,
    pub episode: EpisodeId,
    pub series_title: String,
    pub state: JobState,
    /// 0.0 to 1.0.
    pub progress: f64,
    pub error: Option<String>,
    /// File on disk once the job is done.
    pub path: Option<String>,
    /// Language the user picked; `None` follows the order in the settings.
    pub language: Option<Language>,
    /// Hoster the user picked, tried before the others; `None` follows the settings.
    pub hoster: Option<String>,
    /// Folder the `Series` folder of this download lives in.
    pub dir: String,
    /// Cleared from the queue page. The record stays, so the episode is not downloaded again.
    pub hidden: bool,
    /// Unix seconds of the last time this was queued.
    pub queued_at: i64,
    /// For the folder name: an IMDb ID or `none`. Absent on downloads from before folders
    /// carried the ID; those keep the old folder name.
    pub imdb_id: Option<String>,
}

text_enum! {
    RequestState { Open => "open", Accepted => "accepted", Rejected => "rejected" }
}

/// Which episodes of a series a request asks for.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Picked {
    /// The whole series; the other fields are then empty.
    pub all: bool,
    pub seasons: Vec<u32>,
    /// `(season, episode)` pairs.
    pub episodes: Vec<(u32, u32)>,
}

/// A user asking an admin to download something.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub id: i64,
    pub user_id: i64,
    pub user_name: String,
    pub site: SiteId,
    pub slug: String,
    pub series_title: String,
    pub picked: Picked,
    pub language: Option<Language>,
    pub hoster: Option<String>,
    pub imdb_id: Option<String>,
    pub state: RequestState,
    /// Unix seconds.
    pub created_at: i64,
}

/// An IMDb ID as Kura uses it in folder names: `tt` and 7 to 10 digits, or `none`.
pub fn is_imdb_id(id: &str) -> bool {
    let digits = id.strip_prefix("tt").unwrap_or_default();
    id == "none" || ((7..=10).contains(&digits.len()) && digits.bytes().all(|b| b.is_ascii_digit()))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct User {
    pub id: i64,
    pub name: String,
    pub role: Role,
}

/// Server-wide settings, changed by an admin and kept across restarts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Downloads running at the same time.
    pub parallel: u8,
    /// Tried in this order until one is available.
    pub languages: Vec<Language>,
    /// Hoster names, tried in this order within a language.
    pub hosters: Vec<String>,
    pub output_format: OutputFormat,
    /// File name of an episode, e.g. `{title} {episodenumber} {language}`. `None` uses
    /// `KURA_NAMING_TEMPLATE` or the built-in template.
    pub naming_template: Option<String>,
    /// Where downloads go. The first is the main library, used when the user picks none and
    /// no library is the default for the site. Empty until the admin sets one up.
    pub libraries: Vec<Library>,
    /// Sites left out of search, lists and Auto-Sync.
    pub disabled_sites: Vec<SiteId>,
    pub auto_sync: AutoSync,
}

/// A named folder downloads can go to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Library {
    pub name: String,
    pub path: PathBuf,
    /// Sites whose downloads go here unless the user picks another library.
    #[serde(default)]
    pub default_for: Vec<SiteId>,
}

/// Checks for new episodes of series that were downloaded before.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AutoSync {
    pub enabled: bool,
    /// On: only episodes after the newest downloaded one. Off: gaps are filled too.
    pub only_new: bool,
    pub schedule: Schedule,
    /// Titles Auto-Sync leaves alone.
    pub excluded: Vec<TitleRef>,
}

impl Default for AutoSync {
    fn default() -> Self {
        Self {
            enabled: false,
            only_new: true,
            schedule: Schedule::Every { minutes: 60 },
            excluded: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Schedule {
    Every {
        minutes: u32,
    },
    /// Once a day at `HH:MM`, server time.
    Daily {
        at: String,
    },
}

/// Names a series without loading it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TitleRef {
    pub site: SiteId,
    pub slug: String,
    pub title: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            parallel: 4,
            languages: vec![
                Language::GermanDub,
                Language::GermanSub,
                Language::EnglishSub,
            ],
            // Only hosters Kura can extract from; others would just be skipped.
            hosters: ["voe", "vidmoly"].map(String::from).to_vec(),
            output_format: OutputFormat::Mkv,
            naming_template: None,
            libraries: Vec::new(),
            disabled_sites: Vec::new(),
            auto_sync: AutoSync::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_enums_use_one_spelling_everywhere() {
        let json = serde_json::to_string(&Language::GermanSub).unwrap();
        assert_eq!(json, "\"german_sub\"");
        assert_eq!(Language::GermanSub.as_str(), "german_sub");
        assert_eq!(
            "german_sub".parse::<Language>().unwrap(),
            Language::GermanSub
        );
        assert!("klingon".parse::<Language>().is_err());
    }

    #[test]
    fn imdb_ids_are_tt_and_digits_or_none() {
        for good in ["tt0388629", "tt12345678", "none"] {
            assert!(is_imdb_id(good), "{good}");
        }
        for bad in [
            "",
            "tt",
            "tt123",
            "0388629",
            "tt0388629x",
            "../tt0388629",
            "NONE",
        ] {
            assert!(!is_imdb_id(bad), "{bad}");
        }
    }

    #[test]
    fn settings_from_an_older_version_fill_in_new_fields() {
        let settings: Settings = serde_json::from_str(r#"{"parallel": 2}"#).unwrap();
        assert_eq!(settings.parallel, 2);
        assert_eq!(settings.languages, Settings::default().languages);
    }
}
