//! Joins sites, hosters, downloader and store behind one `Core`.
//!
//! Front ends (the web server, later a bot) call `Core` and nothing below it.

mod auth;
mod downloaded;
mod library;
mod naming;
mod net;
mod requests;
mod sync;

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::{Duration, Instant},
};

use async_trait::async_trait;
pub use auth::SESSION_TTL;
use kura_domain::{
    Episode, EpisodeId, Genre, HosterLink, Job, JobState, Language, OutputFormat, Schedule, Series,
    SeriesSummary, Settings, SiteId, Stream, TitleRef,
};
use kura_download::{DownloadError, Progress};
use kura_hosters::Hoster;
pub use kura_sites::SiteError;
use kura_sites::{AniWorld, SerienStream, Site};
use kura_store::{NewJob, Store, StoreError};
pub use library::{Entry, LibraryRoot};
pub use naming::{DEFAULT_TEMPLATE, default_template};
pub use net::REPOSITORY;
use serde::Serialize;
pub use sync::{SyncStatus, TrackedTitle};
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
    /// A library folder that does not exist and cannot be created.
    #[error("the folder {0} cannot be created: {1}")]
    Folder(String, std::io::Error),
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
        kura_download::download(stream, target, format, on_progress).await
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
    /// When GitHub was last asked for a newer release, and the answer.
    update: Mutex<Option<(Instant, Option<String>)>>,
    /// Series folders found in the libraries, kept for a moment.
    scan: Mutex<Option<downloaded::Scan>>,
}

/// When a site's full list was fetched, and the list.
type Catalog = (Instant, Arc<Vec<SeriesSummary>>);

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

    pub fn save_settings(&self, settings: &Settings) -> Result<()> {
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
        Ok(())
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

    // Queue

    pub fn jobs(&self) -> Result<Vec<Job>> {
        Ok(self.store.jobs()?)
    }

    /// Queues episodes and returns how many were new. Episodes that are already queued,
    /// running or done are skipped.
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
            Selection::Picked { seasons, episodes } => (seasons, episodes),
        };
        for season in seasons {
            let listed = source.episodes(slug, season).await?;
            episodes.extend(listed.iter().map(|episode| (season, episode.number)));
        }
        if episodes.is_empty() {
            return Err(CoreError::Invalid("nothing was selected"));
        }

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
                Ok(path) => {
                    // The new file should show as downloaded at once.
                    core.forget_scan();
                    core.store.finish(id, &path.to_string_lossy())
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
    /// hoster goes first. The error lists every failed attempt.
    async fn process(&self, job: &Job) -> std::result::Result<PathBuf, String> {
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
                        Ok(()) => return Ok(target),
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

    async fn fetch(
        &self,
        id: i64,
        hoster: &dyn Hoster,
        link: &HosterLink,
        target: &Path,
        format: OutputFormat,
    ) -> std::result::Result<(), String> {
        let stream = hoster.resolve(&link.url).await;
        let stream = stream.map_err(|err| err.to_string())?;
        let mut report = |progress: Progress| {
            // A lost progress update is harmless; the next one follows within a second.
            let _ = self.store.set_progress(id, progress.fraction);
            let _ = self.events.send(Event::Progress {
                id,
                fraction: progress.fraction,
                bytes: progress.bytes,
            });
        };
        let done = self
            .downloader
            .download(&stream, target, format, &mut report);
        done.await.map_err(|err| err.to_string())
    }
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
        None => {
            let default = libraries
                .iter()
                .find(|library| library.default_for.contains(&site));
            let main = default.or(libraries.first());
            main.ok_or(CoreError::Invalid(
                "an admin needs to set up a library first",
            ))?
        }
    };
    Ok(library.path.to_string_lossy().into_owned())
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
