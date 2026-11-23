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
    /// Set by the core when at least one episode is in a library.
    #[serde(default)]
    pub downloaded: bool,
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

/// An episode a site published recently, as its list of new episodes shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewEpisode {
    pub site: SiteId,
    pub slug: String,
    pub title: String,
    /// 0 for films and specials.
    pub season: u32,
    pub number: u32,
    pub language: Option<Language>,
    /// As the site shows it, in its time zone, e.g. `21.11.2026 19:05`.
    pub released: String,
}

/// One day of a site's release calendar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarDay {
    /// 1 is Monday, 7 Sunday.
    pub weekday: u8,
    /// `21.11.2026`.
    pub date: String,
    pub today: bool,
    pub entries: Vec<CalendarEntry>,
}

/// An episode announced for one day.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarEntry {
    pub slug: String,
    pub title: String,
    pub cover_url: Option<String>,
    /// 0 for films and specials.
    pub season: u32,
    pub number: u32,
    /// `13:40`, approximate, in the site's time zone.
    pub time: String,
    pub language: Option<Language>,
    /// Whether the episode can be watched already.
    pub online: bool,
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
    /// Unix seconds of the last time a worker started it. `None` until it first started, and
    /// on downloads from before 1.2.10.
    #[serde(default)]
    pub started_at: Option<i64>,
}

impl Job {
    /// Whether a worker ever started this download; downloads from before 1.2.10 that are
    /// done count as started.
    pub fn started(&self) -> bool {
        self.started_at.is_some() || self.state == JobState::Done
    }
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

/// What a user sees and changes about their own account.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Account {
    pub id: i64,
    pub name: String,
    pub role: Role,
    /// Where a password reset link is sent.
    pub email: Option<String>,
    pub theme: Theme,
    /// Whether the web app asks once for an email address: none is set, and the account has
    /// not said "not now" yet.
    #[serde(default)]
    pub ask_email: bool,
}

/// A user as admins see them in Administration → Users.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserEntry {
    pub id: i64,
    pub name: String,
    pub role: Role,
    pub email: Option<String>,
}

/// An invitation an admin sent: whoever opens its link creates an account with this email and
/// role. Only the hash of the link's token is stored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Invite {
    pub id: i64,
    pub email: String,
    pub role: Role,
    /// Unix seconds.
    pub created_at: i64,
    pub expires_at: i64,
}

/// The mail server Kura sends password reset mails through, set by an admin. Kept apart from
/// [`Settings`], so the exported configuration never holds the mail password.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Mail {
    /// e.g. `smtp.protonmail.ch`. Empty while no mail server is set up.
    pub host: String,
    /// 587 for STARTTLS, 465 for TLS, 25 for a relay in the local network.
    pub port: u16,
    pub encryption: MailEncryption,
    pub username: String,
    /// The password or app token (ProtonMail: an SMTP token). Never sent back to the browser.
    pub password: String,
    /// Sender, e.g. `Kura <noreply@example.org>`.
    pub from: String,
    /// The address people open Kura under, e.g. `https://kura.example.org`; links in mails
    /// start with it.
    pub public_url: String,
}

impl Mail {
    /// Whether mails can be sent at all.
    pub fn enabled(&self) -> bool {
        !self.host.is_empty() && !self.from.is_empty() && !self.public_url.is_empty()
    }
}

/// How the connection to the mail server is protected.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MailEncryption {
    /// Plain connection upgraded with STARTTLS, which is required (port 587, "submission").
    #[default]
    Starttls,
    /// TLS from the first byte (port 465).
    Tls,
    /// No encryption: only for a relay in the local network.
    None,
}

/// A user's look and sound of the web app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Theme {
    /// Plays a song an admin uploaded, on a loop.
    pub music: bool,
    /// The id of the song; `None` plays the first one.
    pub song: Option<String>,
    /// 0 to 100.
    pub volume: u8,
    /// A faint animated ASCII scene behind the pages.
    pub ascii: bool,
    /// Which ascii.rest piece the background shows; one of `ascii_library`.
    pub ascii_piece: String,
    /// The ascii.rest pieces this account keeps to choose from.
    pub ascii_library: Vec<String>,
}

/// The scenes every account starts with.
pub const DEFAULT_ASCII_LIBRARY: [&str; 8] = [
    "misty-forest",
    "tokyo-rain",
    "night-coast",
    "kyoto-dusk",
    "lantern-lake",
    "ocean-sunset",
    "aurora-fjord",
    "snowfall",
];

impl Default for Theme {
    fn default() -> Self {
        Self {
            music: true,
            song: None,
            volume: 50,
            ascii: false,
            ascii_piece: DEFAULT_ASCII_LIBRARY[0].to_owned(),
            ascii_library: DEFAULT_ASCII_LIBRARY.map(String::from).to_vec(),
        }
    }
}

/// A song an admin uploaded for the background music.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Song {
    pub id: String,
    pub name: String,
}

/// A key another program uses instead of logging in. The key itself is shown once, when it
/// is created; Kura keeps only its hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiKey {
    pub id: i64,
    pub name: String,
    /// Unix seconds.
    pub created_at: i64,
    pub last_used_at: Option<i64>,
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
    /// Told to scan the series folders that got new episodes once the queue is empty.
    pub jellyfin: Jellyfin,
    /// How text stays readable over an ASCII background, for every account.
    pub backdrop: Backdrop,
}

/// How the web app keeps text readable while an ASCII background runs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Backdrop {
    /// Frosted glass over the whole window: the picture is blurred and dimmed behind it.
    #[default]
    Glass,
    /// The picture stays sharp; text gets a dark halo, controls a plain fill.
    Halo,
}

/// A Jellyfin server to tell about new files. Off while `url` is empty.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Jellyfin {
    /// E.g. `http://192.168.1.10:8096`.
    pub url: String,
    /// An API key from Jellyfin's dashboard (Administration, API Keys).
    pub api_key: String,
}

impl Jellyfin {
    pub fn enabled(&self) -> bool {
        !self.url.trim().is_empty()
    }
}

/// A named folder downloads can go to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Library {
    pub name: String,
    pub path: PathBuf,
    /// Sites whose downloads go here unless the user picks another library.
    #[serde(default)]
    pub default_for: Vec<SiteId>,
    /// The same folder as Jellyfin sees it, when Jellyfin mounts it elsewhere. `None` when
    /// both see the same path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jellyfin_path: Option<PathBuf>,
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
    /// Titles taken off the tracked list. One is tracked again once a download of it starts
    /// after it was removed.
    pub removed: Vec<RemovedTitle>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemovedTitle {
    #[serde(flatten)]
    pub title: TitleRef,
    /// Unix seconds.
    pub at: i64,
}

impl Default for AutoSync {
    fn default() -> Self {
        Self {
            enabled: false,
            only_new: true,
            schedule: Schedule::Every { minutes: 60 },
            excluded: Vec::new(),
            removed: Vec::new(),
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
            jellyfin: Jellyfin::default(),
            backdrop: Backdrop::default(),
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
