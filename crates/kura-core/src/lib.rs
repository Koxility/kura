//! Joins sites, hosters, downloader and store behind one `Core`.
//!
//! Front ends (the web server, later a bot) call `Core` and nothing below it.

mod auth;
mod downloaded;
mod invites;
mod jellyfin;
mod library;
mod mail;
mod naming;
mod net;
mod requests;
mod sync;
mod theme;

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::{Duration, Instant},
};

use async_trait::async_trait;
pub use auth::{API_KEY_PREFIX, SESSION_TTL, check_email};
pub use invites::{INVITE_TTL, Registered};
use kura_domain::{
    CalendarDay, Episode, EpisodeId, Genre, HosterLink, Job, JobState, Language, Library,
    NewEpisode, OutputFormat, RemovedTitle, Schedule, Series, SeriesSummary, Settings, SiteId,
    Stream, TitleRef,
};
use kura_download::{DownloadError, Progress};
use kura_hosters::Hoster;
pub use kura_sites::SiteError;
use kura_sites::{AniWorld, SerienStream, Site};
use kura_store::{NewJob, Store, StoreError};
pub use library::{Entry, LibraryRoot};
pub use mail::RESET_TTL;
pub use naming::{DEFAULT_TEMPLATE, default_template};
pub use net::REPOSITORY;
use serde::Serialize;
pub use sync::{SyncStatus, TrackedTitle};
pub use theme::{MOST_MUSIC_BYTES, Music};
use tokio::{
    sync::{Notify, broadcast},
    task::AbortHandle,
};

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error(transparent)]
    Site(#[from] SiteError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("could not set up the HTTP client: {0}")]
    Http(#[from] reqwest::Error),
    #[error("this site is not supported")]
    UnknownSite,
    #[error("this site is switched off in the settings")]
    SiteDisabled,
    #[error("no such download")]
    UnknownJob,
    /// Input a person can fix; the text is shown to them.
    #[error("{0}")]
    Invalid(&'static str),
    #[error("could not hash the password: {0}")]
    Hashing(String),
    /// Too many wrong logins for one name; the number is the seconds to wait.
    #[error("too many wrong logins; try again in {0} seconds")]
    TooManyLogins(u64),
    /// A library folder that does not exist and cannot be created.
    #[error("the folder {0} cannot be created: {1}")]
    Folder(String, std::io::Error),
    /// Jellyfin did not answer or refused; the text is shown to the admin.
    #[error("{0}")]
    Jellyfin(String),
    /// No mail server is set up, so no mail can be sent.
    #[error("password reset by mail is not set up; ask an admin")]
    MailNotSetUp,
    /// The mail server did not take a mail; the text is shown to the admin.
    #[error("{0}")]
    Mail(String),
    /// Something with this name or address exists already; the text is shown.
    #[error("{0}")]
    Conflict(&'static str),
    /// Too many reset requests from one address; the number is the seconds to wait.
    #[error("too many reset requests; try again in {0} seconds")]
    TooManyResets(u64),
}

pub type Result<T> = std::result::Result<T, CoreError>;

/// What a front end shows live. After `QueueChanged` it reads the queue again, after
/// `RequestsChanged` the requests.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    QueueChanged,
    RequestsChanged,
    Progress { id: i64, fraction: f64, bytes: u64 },
}

/// What a download uses when nothing is picked for it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Defaults {
    /// The first of the settings' languages; later ones are tried when it has no link.
    pub language: Option<Language>,
    /// The first provider of the settings that Kura can extract from.
    pub hoster: Option<String>,
    /// The library name each enabled site downloads into.
    pub libraries: HashMap<SiteId, String>,
}

/// Which episodes of a series to queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    Series,
    /// Whole seasons and single `(season, episode)` pairs.
    Picked {
        seasons: Vec<u32>,
        episodes: Vec<(u32, u32)>,
    },
}

/// What the user picked for a download. `None` leaves the choice to the settings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Choices {
    pub language: Option<Language>,
    /// Tried first; the other hosters remain as fallback.
    pub hoster: Option<String>,
    /// Name of a library from the settings.
    pub library: Option<String>,
    /// For the folder name when the site knows no IMDb ID: an ID or `none`. Ignored when the
    /// site has one.
    pub imdb_id: Option<String>,
}

impl Selection {
    pub fn from_picked(picked: &kura_domain::Picked) -> Self {
        if picked.all {
            Self::Series
        } else {
            Self::Picked {
                seasons: picked.seasons.clone(),
                episodes: picked.episodes.clone(),
            }
        }
    }
}

/// Writes a stream to a file. Exists so tests can run without ffmpeg and a network.
#[async_trait]
pub trait Download: Send + Sync {
    async fn download(
        &self,
        stream: &Stream,
        target: &Path,
        format: OutputFormat,
        on_progress: &mut (dyn FnMut(Progress) + Send),
    ) -> std::result::Result<(), DownloadError>;
}

pub struct Ffmpeg;

#[async_trait]
impl Download for Ffmpeg {
    async fn download(
        &self,
        stream: &Stream,
        target: &Path,
        format: OutputFormat,
        on_progress: &mut (dyn FnMut(Progress) + Send),
    ) -> std::result::Result<(), DownloadError> {
        // The stream address comes from a hoster page; ffmpeg would fetch anything it names.
        if let Err(blocked) = kura_sites::guard::check_stream(&stream.url).await {
            return Err(DownloadError::BadUrl(blocked.to_string()));
        }
        // ffmpeg also follows redirects and the parts a playlist names, and asks DNS again;
        // the relay checks each of those connections too.
        let relay = kura_sites::relay::address().await.map_err(|err| {
            DownloadError::BadUrl(format!("the download relay did not start: {err}"))
        })?;
        let relay = format!("http://{relay}");
        kura_download::download_through(stream, target, format, Some(&relay), on_progress).await
    }
}

pub struct Core {
    // shortcut: SQLite calls run on the async threads; move them to a blocking pool if
    // queries ever take longer than a few milliseconds.
    store: Store,
    sites: Vec<Arc<dyn Site>>,
    hosters: Vec<Box<dyn Hoster>>,
    downloader: Box<dyn Download>,
    events: broadcast::Sender<Event>,
    /// Wakes the worker loop: something was queued, a download ended or settings changed.
    wake: Notify,
    running: Mutex<HashMap<i64, AbortHandle>>,
    /// Every series of a site, kept for the random pick.
    catalogs: Mutex<HashMap<SiteId, Catalog>>,
    sync: Mutex<sync::SyncState>,
    /// Wakes the Auto-Sync loop when its settings changed.
    sync_wake: Notify,
    /// When GitHub is asked next, and the newer release known so far.
    update: Mutex<Option<(Instant, Option<String>)>>,
    /// Series folders found in the libraries, kept for a moment.
    scan: Mutex<Option<downloaded::Scan>>,
    /// Series a title page found downloaded episodes for, also by IMDb ID, so lists mark them.
    /// Checked again with every library scan, so deleted series drop out.
    // shortcut: in memory, a series matched only by IMDb ID is marked in lists once its page
    // was opened since the start; store it if that is not enough.
    found: Mutex<downloaded::FoundTitles>,
    /// Wrong logins per address and name, so passwords cannot be guessed at speed.
    logins: Mutex<auth::LoginTries>,
    /// Tries per address and target for what a mailed link opens: reset requests per name,
    /// invite links per token. Kept apart from `logins`, so filling this list with made-up
    /// names, which is cheap, cannot clear the login brake.
    links: Mutex<auth::LoginTries>,
    /// Series folders Jellyfin is asked to scan once the queue is empty.
    pending_scan: Mutex<jellyfin::Pending>,
    /// The folder of the database, which also holds the theme song.
    data_dir: Mutex<Option<PathBuf>>,
    /// One change to the song list at a time.
    songs_lock: Mutex<()>,
}

/// When a site's full list was fetched, and the list.
type Catalog = (Instant, Arc<Vec<SeriesSummary>>);

/// How often one link is tried before the next hoster or language gets its turn. Every try
/// asks the hoster for the stream again: its addresses expire and name one server, so a
/// stream that answers 404 or stalls often works from a fresh address.
const TRIES: u32 = 3;

/// The wait before the second try; the third waits twice as long. None in tests.
const RETRY_PAUSE: Duration = Duration::from_secs(if cfg!(test) { 0 } else { 5 });

/// Most seasons one selection may name. Every season is a page fetched from the site, so a
/// selection with thousands of them would keep the site busy for hours. Long series have a few
/// dozen seasons; the whole series is picked with `all` instead.
pub const MOST_SEASONS: usize = 200;

/// Most single episodes one selection may name. More than the longest series has.
pub const MOST_EPISODES: usize = 5000;

/// Highest season and episode number a selection may name.
const HIGHEST_SEASON: u32 = 1000;
const HIGHEST_EPISODE: u32 = 100_000;

/// The full lists are megabytes of HTML, so one fetch serves an hour of random picks.
const CATALOG_TTL: Duration = Duration::from_secs(60 * 60);

impl Core {
    pub fn new(
        store: Store,
        sites: Vec<Arc<dyn Site>>,
        hosters: Vec<Box<dyn Hoster>>,
        downloader: Box<dyn Download>,
    ) -> Arc<Self> {
        Arc::new(Self {
            store,
            sites,
            hosters,
            downloader,
            events: broadcast::channel(256).0,
            wake: Notify::new(),
            running: Mutex::default(),
            catalogs: Mutex::default(),
            sync: Mutex::default(),
            sync_wake: Notify::new(),
            update: Mutex::default(),
            scan: Mutex::default(),
            found: Mutex::default(),
            logins: Mutex::default(),
            links: Mutex::default(),
            pending_scan: Mutex::default(),
            data_dir: Mutex::default(),
            songs_lock: Mutex::default(),
        })
    }

    /// A core with the real sites, hosters and ffmpeg.
    pub fn with_defaults(store: Store) -> Result<Arc<Self>> {
        let http = kura_sites::http_client()?;
        Ok(Self::new(
            store,
            vec![
                Arc::new(AniWorld::new(http.clone())),
                Arc::new(SerienStream::new(http.clone())),
            ],
            kura_hosters::all(&http),
            Box::new(Ffmpeg),
        ))
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    // Browsing

    /// Searches one site, or every site at once. With several sites, one that fails is left
    /// out as long as another answers.
    pub async fn search(&self, query: &str, only: Option<SiteId>) -> Result<Vec<SeriesSummary>> {
        if let Some(site) = only {
            self.site(site)?;
        }
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let disabled = self.store.settings()?.disabled_sites;
        let searches: Vec<_> = self
            .sites
            .iter()
            .filter(|site| !disabled.contains(&site.id()))
            .filter(|site| only.is_none_or(|wanted| site.id() == wanted))
            .map(|site| {
                let (site, query) = (site.clone(), query.to_owned());
                tokio::spawn(async move { site.search(&query).await })
            })
            .collect();

        let mut found = Vec::new();
        let mut failure = None;
        for search in searches {
            match search.await {
                Ok(Ok(series)) => found.extend(series),
                Ok(Err(err)) => {
                    tracing::warn!("search failed on one site: {err}");
                    failure = Some(err);
                }
                Err(err) => tracing::error!("search task ended early: {err}"),
            }
        }
        match failure {
            Some(err) if found.is_empty() => Err(err.into()),
            _ => self.marked(found),
        }
    }

    pub async fn popular(&self, site: SiteId) -> Result<Vec<SeriesSummary>> {
        self.marked(self.site(site)?.popular().await?)
    }

    /// Recently added titles, newest first.
    pub async fn latest(&self, site: SiteId) -> Result<Vec<SeriesSummary>> {
        self.marked(self.site(site)?.latest().await?)
    }

    /// A list with `downloaded` set on what is already in a library.
    fn marked(&self, mut list: Vec<SeriesSummary>) -> Result<Vec<SeriesSummary>> {
        self.mark_downloaded(&mut list)?;
        Ok(list)
    }

    /// Episodes the site published recently, newest first; empty for a site without a list.
    pub async fn new_episodes(&self, site: SiteId) -> Result<Vec<NewEpisode>> {
        Ok(self.site(site)?.new_episodes().await?)
    }

    /// The days around today with the episodes announced for them; empty for a site without
    /// a calendar.
    pub async fn calendar(&self, site: SiteId) -> Result<Vec<CalendarDay>> {
        Ok(self.site(site)?.calendar().await?)
    }

    pub async fn genres(&self, site: SiteId) -> Result<Vec<Genre>> {
        Ok(self.site(site)?.genres().await?)
    }

    /// One page of a genre, counted from 1. Empty past the last page.
    pub async fn by_genre(
        &self,
        site: SiteId,
        genre: &str,
        page: u32,
    ) -> Result<Vec<SeriesSummary>> {
        self.marked(self.site(site)?.by_genre(genre, page.max(1)).await?)
    }

    /// One series of the site, picked at random from its full list.
    pub async fn random(&self, site: SiteId) -> Result<SeriesSummary> {
        let source = self.site(site)?;
        let lock = || self.catalogs.lock().unwrap_or_else(PoisonError::into_inner);
        let kept = lock().get(&site).cloned();
        let catalog = match kept {
            Some((fetched, catalog)) if fetched.elapsed() < CATALOG_TTL => catalog,
            _ => {
                let catalog = Arc::new(source.all().await?);
                lock().insert(site, (Instant::now(), catalog.clone()));
                catalog
            }
        };
        if catalog.is_empty() {
            return Err(SiteError::Layout("series list").into());
        }
        let mut dice = [0u8; 8];
        getrandom::fill(&mut dice).map_err(|err| CoreError::Hashing(err.to_string()))?;
        let pick = u64::from_le_bytes(dice) % catalog.len() as u64;
        Ok(catalog[pick as usize].clone())
    }

    pub async fn series(&self, site: SiteId, slug: &str) -> Result<Series> {
        Ok(self.site(site)?.series(slug).await?)
    }

    pub async fn episodes(&self, site: SiteId, slug: &str, season: u32) -> Result<Vec<Episode>> {
        Ok(self.site(site)?.episodes(slug, season).await?)
    }

    /// The sites that are switched on, in the order Kura knows them.
    pub fn enabled_sites(&self) -> Result<Vec<SiteId>> {
        let disabled = self.store.settings()?.disabled_sites;
        let ids = self.sites.iter().map(|site| site.id());
        Ok(ids.filter(|id| !disabled.contains(id)).collect())
    }

    fn site(&self, id: SiteId) -> Result<&dyn Site> {
        if self.store.settings()?.disabled_sites.contains(&id) {
            return Err(CoreError::SiteDisabled);
        }
        let site = self.sites.iter().find(|site| site.id() == id);
        site.map(Arc::as_ref).ok_or(CoreError::UnknownSite)
    }

    // Settings

    pub fn settings(&self) -> Result<Settings> {
        Ok(self.store.settings()?)
    }

    /// What a download uses when nothing is picked: the first language and the first
    /// supported provider of the settings, and each enabled site's library.
    pub fn defaults(&self) -> Result<Defaults> {
        let settings = self.store.settings()?;
        let supported = |name: &&String| self.hosters.iter().any(|hoster| hoster.name() == *name);
        let libraries = self
            .enabled_sites()?
            .into_iter()
            .filter_map(|site| Some((site, default_library(&settings, site)?.name.clone())))
            .collect();
        Ok(Defaults {
            language: settings.languages.first().copied(),
            hoster: settings.hosters.iter().find(supported).cloned(),
            libraries,
        })
    }

    /// Saves the settings and returns them as saved. An empty Jellyfin key keeps the stored
    /// one, so the browser never needs to know it (see [`Core::keep_jellyfin_key`]).
    pub fn save_settings(&self, settings: &Settings) -> Result<Settings> {
        let mut settings = settings.clone();
        self.keep_jellyfin_key(&mut settings.jellyfin)?;
        let settings = &settings;
        check_settings(settings)?;
        // A typo in a path shows now, not at the first download.
        for library in &settings.libraries {
            std::fs::create_dir_all(&library.path).map_err(|err| {
                CoreError::Folder(library.path.to_string_lossy().into_owned(), err)
            })?;
        }
        self.store.save_settings(settings)?;
        // The number of parallel downloads may have gone up, the sync schedule may differ.
        self.wake.notify_one();
        self.sync_wake.notify_one();
        Ok(settings.clone())
    }

    /// Adds a title to the ones Auto-Sync leaves alone, or takes it off that list.
    pub fn set_excluded(&self, title: TitleRef, excluded: bool) -> Result<()> {
        let mut settings = self.store.settings()?;
        let list = &mut settings.auto_sync.excluded;
        list.retain(|other| other.site != title.site || other.slug != title.slug);
        if excluded {
            list.push(title);
        }
        Ok(self.store.save_settings(&settings)?)
    }

    /// Takes a title off Auto-Sync's tracked list without excluding it. Downloading an
    /// episode of it later tracks it again.
    pub fn remove_tracked(&self, title: TitleRef) -> Result<()> {
        let mut settings = self.store.settings()?;
        let list = &mut settings.auto_sync.removed;
        list.retain(|other| other.title.site != title.site || other.title.slug != title.slug);
        list.push(RemovedTitle {
            title,
            at: auth::now(),
        });
        Ok(self.store.save_settings(&settings)?)
    }

    // Queue

    pub fn jobs(&self) -> Result<Vec<Job>> {
        Ok(self.store.jobs()?)
    }

    /// Queues episodes and returns how many were new. Episodes that are already queued,
    /// running or downloaded are skipped; a finished download whose file is gone is queued
    /// again.
    pub async fn enqueue(
        &self,
        site: SiteId,
        slug: &str,
        selection: Selection,
        choices: &Choices,
        user_id: i64,
    ) -> Result<usize> {
        let source = self.site(site)?;
        self.check_choices(choices)?;
        let dir = library_dir(&self.store.settings()?, site, choices.library.as_deref())?;

        let series = source.series(slug).await?;
        // The site's own ID wins; without one, the user's answer, else none.
        let imdb_id = series
            .imdb_id
            .as_deref()
            .or(choices.imdb_id.as_deref())
            .unwrap_or("none");
        let (seasons, mut episodes) = match selection {
            Selection::Series => (series.seasons.clone(), Vec::new()),
            // Also a request stored before the limits existed is checked when it is accepted.
            Selection::Picked { seasons, episodes } => check_picked(seasons, episodes)?,
        };
        for season in seasons {
            let listed = source.episodes(slug, season).await?;
            episodes.extend(listed.iter().map(|episode| (season, episode.number)));
        }
        if episodes.is_empty() {
            return Err(CoreError::Invalid("nothing was selected"));
        }

        // The queue and the libraries as they are now: a finished download whose file was
        // deleted since, and that is not in a library folder either, is downloaded again
        // instead of being skipped as done.
        self.forget_scan();
        let present = self.downloaded_episodes(site, slug, &series.summary.title, Some(imdb_id))?;
        let jobs = self.store.jobs()?;
        let gone = |season: u32, number: u32| {
            jobs.iter().any(|job| {
                job.state == JobState::Done
                    && job.episode.site == site
                    && job.episode.slug == slug
                    && (job.episode.season, job.episode.number) == (season, number)
                    && !downloaded::on_disk(job)
            }) && !present.contains(&(season, number))
        };

        let mut queued = 0;
        for (season, number) in episodes {
            let episode = EpisodeId {
                site,
                slug: slug.to_owned(),
                season,
                number,
            };
            let new = NewJob {
                episode: &episode,
                series_title: &series.summary.title,
                user_id: Some(user_id),
                language: choices.language,
                hoster: choices.hoster.as_deref(),
                dir: &dir,
                imdb_id: Some(imdb_id),
                now: auth::now(),
                replace_done: gone(season, number),
            };
            // An episode picked twice, alone and with its season, is queued once.
            queued += usize::from(self.store.enqueue(&new)?.is_some());
        }
        self.changed();
        Ok(queued)
    }

    /// Queues a failed or cancelled download again, with the choices it had. Returns whether
    /// it was queued.
    pub fn retry(&self, id: i64, user_id: i64) -> Result<bool> {
        let jobs = self.store.jobs()?;
        let job = jobs.iter().find(|job| job.id == id);
        let job = job.ok_or(CoreError::UnknownJob)?;
        let queued = self.store.enqueue(&NewJob {
            episode: &job.episode,
            series_title: &job.series_title,
            user_id: Some(user_id),
            language: job.language,
            hoster: job.hoster.as_deref(),
            dir: &job.dir,
            imdb_id: job.imdb_id.as_deref(),
            now: auth::now(),
            replace_done: false,
        })?;
        self.changed();
        Ok(queued.is_some())
    }

    /// Takes finished, failed and cancelled downloads off the queue page and returns how
    /// many. They stay known, so nothing is downloaded twice.
    pub fn clear_finished(&self) -> Result<usize> {
        let cleared = self.store.hide_finished()?;
        self.changed();
        Ok(cleared)
    }

    /// Returns whether a queued or running download was cancelled.
    pub fn cancel(&self, id: i64) -> Result<bool> {
        let cancelled = self.store.cancel(id)?;
        // Aborting drops the download, which stops ffmpeg and removes the partial file.
        if let Some(download) = self.running().remove(&id) {
            download.abort();
        }
        self.changed();
        Ok(cancelled)
    }

    /// Refuses a provider Kura cannot extract from and a malformed IMDb ID.
    fn check_choices(&self, choices: &Choices) -> Result<()> {
        let supported = |name: &String| self.hosters.iter().any(|hoster| hoster.name() == name);
        if choices.hoster.as_ref().is_some_and(|name| !supported(name)) {
            return Err(CoreError::Invalid("this provider is not supported"));
        }
        if choices
            .imdb_id
            .as_deref()
            .is_some_and(|id| !kura_domain::is_imdb_id(id))
        {
            return Err(CoreError::Invalid("an IMDb ID looks like tt0388629"));
        }
        Ok(())
    }

    fn changed(&self) {
        // Nobody listening is fine.
        let _ = self.events.send(Event::QueueChanged);
        self.wake.notify_one();
    }

    fn running(&self) -> MutexGuard<'_, HashMap<i64, AbortHandle>> {
        self.running.lock().unwrap_or_else(PoisonError::into_inner)
    }

    // Workers

    /// Works through the queue until the store fails. Run it once, for the life of the server.
    pub async fn run(self: Arc<Self>) -> Result<()> {
        self.store.requeue_interrupted()?;
        loop {
            // Read every round, so a changed setting applies without a restart.
            let parallel = usize::from(self.store.settings()?.parallel.max(1));
            while self.running().len() < parallel {
                let Some(job) = self.store.claim_next()? else {
                    break;
                };
                self.start(job);
            }
            // Nothing queued and nothing running: the batch is through, also when some of it
            // failed, so Jellyfin scans what arrived, once.
            if self.running().is_empty() {
                self.queue_drained();
            }
            self.wake.notified().await;
        }
    }

    fn start(self: &Arc<Self>, job: Job) {
        let core = self.clone();
        let id = job.id;
        // Held until the entry exists, so a download that ends at once still finds it.
        let mut running = self.running();
        let download = tokio::spawn(async move {
            let saved = match core.process(&job).await {
                Ok((path, language)) => {
                    // The new file should show as downloaded at once.
                    core.forget_scan();
                    core.note_new_file(&path);
                    core.store.finish(id, &path.to_string_lossy(), language)
                }
                Err(reason) => core.store.fail(id, &reason),
            };
            if let Err(err) = saved {
                tracing::error!("could not save the result of download {id}: {err}");
            }
            core.running().remove(&id);
            core.changed();
        });
        running.insert(id, download.abort_handle());
        let _ = self.events.send(Event::QueueChanged);
    }

    /// Downloads one episode. Languages are tried in order and, within a language, hosters in
    /// order. A language picked for the job replaces the order from the settings; a picked
    /// hoster goes first. Returns the file and its language; the error lists every failed
    /// attempt.
    async fn process(&self, job: &Job) -> std::result::Result<(PathBuf, Language), String> {
        let settings = self.store.settings().map_err(|err| err.to_string())?;
        let site = self.site(job.episode.site).map_err(|err| err.to_string())?;
        let links = site.links(&job.episode).await;
        let links = links.map_err(|err| err.to_string())?;
        let format = settings.output_format;
        let template = naming::effective_template(&settings);
        // Later episodes join the folder earlier ones went to, also from older layouts.
        let jobs = self.store.jobs().map_err(|err| err.to_string())?;
        let existing = jobs
            .iter()
            .filter(|other| {
                other.state == JobState::Done
                    && other.episode.site == job.episode.site
                    && other.episode.slug == job.episode.slug
            })
            .filter_map(|other| downloaded::series_folder(Path::new(other.path.as_deref()?)))
            .find(|folder| folder.is_dir());
        // The name carries the language, which is only known once a link is picked.
        let target = |language| {
            let dir = Path::new(&job.dir);
            target_path(dir, &template, job, language, format, existing.as_deref())
        };

        let languages = job
            .language
            .map_or(settings.languages, |picked| vec![picked]);
        let mut hosters = settings.hosters;
        if let Some(picked) = &job.hoster {
            hosters.retain(|name| name != picked);
            hosters.insert(0, picked.clone());
        }

        let mut failures = Vec::new();
        for &language in &languages {
            for name in &hosters {
                let Some(hoster) = self.hosters.iter().find(|hoster| hoster.name() == name) else {
                    continue;
                };
                let wanted = |link: &&HosterLink| link.language == language && link.hoster == *name;
                for link in links.iter().filter(wanted) {
                    let target = target(language);
                    let fetched = self.fetch(job.id, hoster.as_ref(), link, &target, format);
                    match fetched.await {
                        Ok(()) => return Ok((target, language)),
                        Err(reason) => {
                            failures.push(format!("{name}, {}: {reason}", language.as_str()));
                        }
                    }
                }
            }
        }
        if failures.is_empty() {
            Err("no link in a wanted language from a supported hoster".to_owned())
        } else {
            Err(failures.join("; "))
        }
    }

    /// Downloads one link, again from a fresh stream address when the hoster or the stream
    /// failed in a way that may pass.
    async fn fetch(
        &self,
        id: i64,
        hoster: &dyn Hoster,
        link: &HosterLink,
        target: &Path,
        format: OutputFormat,
    ) -> std::result::Result<(), String> {
        let mut tried = 1;
        loop {
            let (reason, temporary) = match self.attempt(id, hoster, link, target, format).await {
                Ok(()) => return Ok(()),
                Err(failure) => failure,
            };
            // The failed try may have got halfway; the next one starts from nothing.
            self.report(id, Progress::default());
            if !temporary {
                return Err(reason);
            }
            if tried == TRIES {
                return Err(format!("{reason} (tried {TRIES} times)"));
            }
            tracing::warn!(
                "download {id}: try {tried} of {TRIES} on {} failed, trying again: {reason}",
                hoster.name()
            );
            // Shown in the queue, so a download that starts over says why.
            let note = format!("Try {tried} of {TRIES} failed, trying again: {reason}");
            let _ = self.store.note_retry(id, &note);
            let _ = self.events.send(Event::QueueChanged);
            tokio::time::sleep(RETRY_PAUSE * tried).await;
            tried += 1;
        }
    }

    /// One try: resolves the link and downloads the stream. The error says why it failed and
    /// whether another try may work.
    async fn attempt(
        &self,
        id: i64,
        hoster: &dyn Hoster,
        link: &HosterLink,
        target: &Path,
        format: OutputFormat,
    ) -> std::result::Result<(), (String, bool)> {
        let stream = hoster.resolve(&link.url).await;
        let stream = stream.map_err(|err| (err.to_string(), err.is_temporary()))?;
        let mut report = |progress: Progress| self.report(id, progress);
        let done = self
            .downloader
            .download(&stream, target, format, &mut report);
        done.await
            .map_err(|err| (err.to_string(), err.is_stream_failure()))
    }

    fn report(&self, id: i64, progress: Progress) {
        // A lost progress update is harmless; the next one follows within a second.
        let _ = self.store.set_progress(id, progress.fraction);
        let _ = self.events.send(Event::Progress {
            id,
            fraction: progress.fraction,
            bytes: progress.bytes,
        });
    }
}

/// Single `(season, episode)` pairs.
type Episodes = Vec<(u32, u32)>;

/// A selection without repeats, refused when it names more than a series can have: each
/// season is a page fetched from the site, and each episode a row in the queue.
pub(crate) fn check_picked(
    mut seasons: Vec<u32>,
    mut episodes: Vec<(u32, u32)>,
) -> Result<(Vec<u32>, Episodes)> {
    seasons.sort_unstable();
    seasons.dedup();
    episodes.sort_unstable();
    episodes.dedup();
    if seasons.len() > MOST_SEASONS {
        return Err(CoreError::Invalid(
            "too many seasons at once; pick the whole series instead",
        ));
    }
    if episodes.len() > MOST_EPISODES {
        return Err(CoreError::Invalid(
            "too many episodes at once; pick whole seasons instead",
        ));
    }
    let season_ok = |season: &u32| *season <= HIGHEST_SEASON;
    let episode_ok =
        |(season, number): &(u32, u32)| season_ok(season) && *number <= HIGHEST_EPISODE;
    if !seasons.iter().all(season_ok) || !episodes.iter().all(episode_ok) {
        return Err(CoreError::Invalid(
            "no series has a season or episode this high",
        ));
    }
    Ok((seasons, episodes))
}

/// The folder a download goes to: the picked library, else the site's default library, else
/// the main library.
fn library_dir(settings: &Settings, site: SiteId, picked: Option<&str>) -> Result<String> {
    let libraries = &settings.libraries;
    let library = match picked {
        Some(name) => {
            let library = libraries.iter().find(|library| library.name == name);
            library.ok_or(CoreError::Invalid("no such library"))?
        }
        None => default_library(settings, site).ok_or(CoreError::Invalid(
            "an admin needs to set up a library first",
        ))?,
    };
    Ok(library.path.to_string_lossy().into_owned())
}

/// The site's default library, else the main library.
fn default_library(settings: &Settings, site: SiteId) -> Option<&Library> {
    let libraries = &settings.libraries;
    let default = libraries
        .iter()
        .find(|library| library.default_for.contains(&site));
    default.or(libraries.first())
}

/// Refuses settings that would leave Kura unable to download or sync.
fn check_settings(settings: &Settings) -> Result<()> {
    let invalid = |message| Err(CoreError::Invalid(message));
    if !(1..=16).contains(&settings.parallel) {
        return invalid("parallel downloads must be between 1 and 16");
    }
    if settings.libraries.is_empty() {
        return invalid("at least one library is needed");
    }
    if settings.languages.is_empty() || settings.hosters.is_empty() {
        return invalid("at least one language and one provider are needed");
    }
    if let Some(template) = &settings.naming_template {
        naming::check_template(template)?;
    }
    if settings.jellyfin.enabled() {
        jellyfin::check(&settings.jellyfin)?;
    }

    let mut names = Vec::new();
    let mut defaults = Vec::new();
    for library in &settings.libraries {
        let name = library.name.trim().to_lowercase();
        if name.is_empty() || names.contains(&name) {
            return invalid("every library needs a name of its own");
        }
        names.push(name);
        if !library.path.is_absolute() {
            return invalid("a library path must start at the root, like /mnt/nas/Serien");
        }
        for site in &library.default_for {
            if defaults.contains(site) {
                return invalid("a site can have only one default library");
            }
            defaults.push(*site);
        }
    }

    match &settings.auto_sync.schedule {
        Schedule::Every { minutes } if !(5..=10_080).contains(minutes) => {
            invalid("the Auto-Sync interval must be between 5 minutes and one week")
        }
        Schedule::Daily { at } if parse_time(at).is_none() => {
            invalid("the Auto-Sync time must look like 03:30")
        }
        _ => Ok(()),
    }
}

/// Hour and minute of an `HH:MM` time.
pub(crate) fn parse_time(at: &str) -> Option<(u32, u32)> {
    let (hour, minute) = at.split_once(':')?;
    let (hour, minute) = (hour.parse().ok()?, minute.parse().ok()?);
    (hour < 24 && minute < 60).then_some((hour, minute))
}

/// `<dir>/<Title> [imdbid-tt0388629]/Season 01/<name from the template>.<ext>`, the layout
/// Jellyfin reads; season 0 is specials. A series that already has a folder keeps it, also
/// one in the `Series` folder older versions used, so a series is never split.
fn target_path(
    dir: &Path,
    template: &str,
    job: &Job,
    language: Language,
    format: OutputFormat,
    existing: Option<&Path>,
) -> PathBuf {
    let episode = &job.episode;
    // The slug is already limited to safe characters.
    let name = safe_name(&job.series_title).unwrap_or_else(|| episode.slug.clone());
    // A stored ID was checked on the way in; anything else would not reach the path.
    let imdb_id = job
        .imdb_id
        .as_deref()
        .filter(|id| kura_domain::is_imdb_id(id));
    let parts = naming::NameParts {
        title: &name,
        season: episode.season,
        episode: episode.number,
        language,
        imdb_id,
    };
    // A checked template always leaves a name; the fallback only guards against surprises.
    let file = naming::render(template, &parts)
        .or_else(|| naming::render(naming::DEFAULT_TEMPLATE, &parts))
        .unwrap_or_else(|| format!("S{:02}E{:03}", episode.season, episode.number));
    let folder = match (existing, imdb_id) {
        (Some(existing), _) => existing.to_owned(),
        (None, Some(id)) => dir.join(format!("{name} [imdbid-{id}]")),
        (None, None) => dir.join(name),
    };
    folder
        .join(format!("Season {:02}", episode.season))
        .join(format!("{file}.{}", format.as_str()))
}

/// A title as a folder name: nothing that could leave the folder or that a file system
/// refuses. `None` when nothing usable is left.
pub(crate) fn safe_name(title: &str) -> Option<String> {
    let cleaned: String = title
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => ' ',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let cleaned = cleaned.trim_matches(['.', ' ']);
    (!cleaned.is_empty()).then(|| cleaned.to_owned())
}

#[cfg(test)]
mod tests;
