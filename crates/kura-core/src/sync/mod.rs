//! Auto-Sync, after the way AniWorld-Downloader does it: the sites' lists of new episodes
//! say which tracked series got something new, and only the libraries say what is there.
//!
//! On its schedule, Auto-Sync reads each site's list of new episodes ([`Core::sync_feed`]).
//! A tracked series that shows up in it gets every episode it is missing, or with "only new
//! episodes" the ones the list announced and those between them, since a release of many
//! episodes may show as its first and last one only. **Sync now** checks every tracked
//! series in full ([`Core::sync_once`]).
//!
//! An episode is there while a file of it is in the library; a deleted one comes again. What
//! is downloaded in which language, and when a better language replaces a file, is in
//! [`reconcile`]. What each check found is kept in the database for the page.

mod reconcile;

use std::{
    collections::{BTreeMap, HashMap},
    future::Future,
    sync::{Arc, MutexGuard, PoisonError},
    time::Duration,
};

use chrono::{Days, Local, NaiveTime};
use kura_domain::{
    Episode, EpisodeId, Job, Language, NewEpisode, Schedule, Settings, SiteId, TitleRef,
};
use kura_store::{NewJob, StoreError};
use reconcile::Decision;
use serde::{Deserialize, Serialize};

use crate::{Core, Result, auth::now, downloaded::Found, naming::language_name, parse_time};

/// The wait before a page that could not be read is asked for again. None in tests.
const FETCH_PAUSE: Duration = Duration::from_secs(if cfg!(test) { 0 } else { 3 });

/// Kept in memory only; the rest is in the database.
#[derive(Default)]
pub(crate) struct SyncState {
    next_run: Option<i64>,
    /// Episodes the last run queued, for a Sync Now that waited for it to end.
    last_queued: usize,
}

/// What the Auto-Sync page shows. Times are Unix seconds.
#[derive(Debug, Serialize)]
pub struct SyncStatus {
    pub enabled: bool,
    pub last_run: Option<i64>,
    pub next_run: Option<i64>,
    /// What the last run did; `None` before the first.
    pub last_report: Option<RunReport>,
    pub titles: Vec<TrackedTitle>,
}

/// A series Auto-Sync watches: a download of at least one episode of it started.
#[derive(Debug, Serialize)]
pub struct TrackedTitle {
    #[serde(flatten)]
    pub title: TitleRef,
    /// The languages its files are in, best first; without a file that tells one, the
    /// language of its most recent download.
    pub languages: Vec<Language>,
    /// Episodes in the libraries now, in any language.
    pub on_disk: usize,
    pub newest_on_disk: Option<(u32, u32)>,
    /// What the last check found; `None` before the first.
    pub check: Option<TitleCheck>,
}

/// What started a check.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    /// The series was in a site's list of new episodes.
    #[default]
    Feed,
    /// Sync now: every series, in full.
    Full,
}

/// What one check of a title found.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TitleCheck {
    pub checked_at: i64,
    pub trigger: Trigger,
    /// Why the check failed, or which seasons could not be read.
    pub error: Option<String>,
    /// The newest episode the site lists, in any language, of the seasons checked.
    pub newest_listed: Option<(u32, u32)>,
    /// Episodes queued by this check, among them those below.
    pub queued: Vec<(u32, u32)>,
    /// Episodes queued in a better language, replacing the file they have.
    pub replacing: Vec<(u32, u32)>,
    /// Missing episodes not offered in a language the series may have yet.
    pub waiting: Vec<Waiting>,
    /// One line for the page.
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Waiting {
    pub episode: (u32, u32),
    /// The languages it is offered in.
    pub offered: Vec<Language>,
}

/// What one run did.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RunReport {
    pub at: i64,
    pub trigger: Trigger,
    /// Entries in the sites' lists of new episodes; `None` for Sync now.
    pub feed_entries: Option<usize>,
    /// Series checked: those in the lists, or all for Sync now.
    pub checked: usize,
    pub queued: usize,
    /// Checks that failed.
    pub failed: usize,
    /// Lists of new episodes that could not be read.
    pub error: Option<String>,
}

/// Which episodes of a series a check looks at.
enum Scope {
    /// Every season.
    Full,
    /// Per season, the announced episodes and those between them.
    Announced(BTreeMap<u32, (u32, u32)>),
}

impl Core {
    pub fn sync_status(&self) -> Result<SyncStatus> {
        let settings = self.store.settings()?;
        let jobs = self.store.jobs()?;
        let mut checks = self.sync_checks()?;
        let mut titles = Vec::new();
        for title in tracked(&settings, &jobs) {
            let own: Vec<&Job> = of_title(&jobs, &title).collect();
            let files = self.title_files(&title, &own)?;
            let mut present: Vec<(u32, u32)> = files.iter().map(|file| file.episode).collect();
            present.sort_unstable();
            present.dedup();
            titles.push(TrackedTitle {
                languages: reconcile::held(&settings.languages, &files, last_language(&own)),
                on_disk: present.len(),
                newest_on_disk: present.last().copied(),
                check: checks.remove(&(title.site, title.slug.clone())),
                title,
            });
        }
        let report = self.store.last_sync_report()?;
        Ok(SyncStatus {
            enabled: settings.auto_sync.enabled,
            last_run: self.store.last_sync_run()?,
            next_run: self
                .sync_state()
                .next_run
                .filter(|_| settings.auto_sync.enabled),
            last_report: report.and_then(|json| serde_json::from_str(&json).ok()),
            titles,
        })
    }

    /// Sync now: checks every tracked series in full and returns how many episodes were
    /// queued.
    pub async fn sync_once(&self) -> Result<usize> {
        self.sync_run(Trigger::Full).await
    }

    /// The scheduled run: reads the sites' lists of new episodes and checks the tracked
    /// series in them. Returns how many episodes were queued.
    pub async fn sync_feed(&self) -> Result<usize> {
        self.sync_run(Trigger::Feed).await
    }

    /// One run. While a run is under way, another one waits for it and returns its result
    /// instead of checking everything a second time.
    async fn sync_run(&self, trigger: Trigger) -> Result<usize> {
        let _run = match self.sync_lock.try_lock() {
            Ok(run) => run,
            Err(_) => {
                drop(self.sync_lock.lock().await);
                return Ok(self.sync_state().last_queued);
            }
        };
        // What is in the library now, not what was there at the last scan.
        self.forget_scan();
        let settings = self.store.settings()?;
        let jobs = self.store.jobs()?;
        let mut report = RunReport {
            at: now(),
            trigger,
            ..RunReport::default()
        };
        let titles = tracked(&settings, &jobs);
        let work: Vec<(TitleRef, Scope, Vec<NewEpisode>)> = match trigger {
            Trigger::Full => titles
                .into_iter()
                .map(|title| (title, Scope::Full, Vec::new()))
                .collect(),
            Trigger::Feed => self.feed_hits(&settings, titles, &mut report).await,
        };
        report.checked = work.len();
        for (title, scope, announced) in work {
            let check = self
                .sync_title(&settings, &jobs, &title, &scope, &announced, trigger)
                .await
                .unwrap_or_else(|err| {
                    let error = err.to_string();
                    TitleCheck {
                        checked_at: now(),
                        trigger,
                        summary: format!("Check failed: {error}"),
                        error: Some(error),
                        ..TitleCheck::default()
                    }
                });
            report.queued += check.queued.len();
            report.failed += usize::from(check.error.is_some());
            let json = serde_json::to_string(&check).map_err(StoreError::from)?;
            self.store
                .save_sync_result(title.site, &title.slug, &json)?;
        }
        let json = serde_json::to_string(&report).map_err(StoreError::from)?;
        self.store.set_last_sync_run(report.at, &json)?;
        self.sync_state().last_queued = report.queued;
        tracing::info!(
            "Auto-Sync ({}): {} series checked, {} episodes queued, {} checks failed{}",
            match trigger {
                Trigger::Feed => "new episodes",
                Trigger::Full => "every series",
            },
            report.checked,
            report.queued,
            report.failed,
            report
                .error
                .as_ref()
                .map_or_else(String::new, |error| format!("; {error}")),
        );
        // A run the user started counts as the last run, so the schedule moves with it.
        self.sync_wake.notify_one();
        if report.queued > 0 {
            self.changed();
        }
        Ok(report.queued)
    }

    /// The tracked series in the sites' lists of new episodes, with what to check of them and
    /// what the lists announced. A list that cannot be read is noted in the report.
    async fn feed_hits(
        &self,
        settings: &Settings,
        titles: Vec<TitleRef>,
        report: &mut RunReport,
    ) -> Vec<(TitleRef, Scope, Vec<NewEpisode>)> {
        let mut sites: Vec<SiteId> = titles.iter().map(|title| title.site).collect();
        sites.sort_by_key(|site| site.as_str());
        sites.dedup();
        let mut announced: HashMap<(SiteId, String), Vec<NewEpisode>> = HashMap::new();
        let mut entries = 0;
        let mut errors = Vec::new();
        for id in sites {
            let feed = match self.site(id) {
                Ok(site) => twice(|| site.feed()).await.map_err(Into::into),
                Err(err) => Err(err),
            };
            match feed {
                Ok(feed) => {
                    entries += feed.len();
                    // Films and specials are not followed.
                    for entry in feed.into_iter().filter(|entry| entry.season > 0) {
                        let key = (entry.site, entry.slug.clone());
                        announced.entry(key).or_default().push(entry);
                    }
                }
                Err(err) => errors.push(format!("{}: {err}", id.as_str())),
            }
        }
        report.feed_entries = Some(entries);
        if !errors.is_empty() {
            report.error = Some(format!(
                "could not read the new episodes of {}",
                errors.join("; ")
            ));
        }
        titles
            .into_iter()
            .filter_map(|title| {
                let announced = announced.remove(&(title.site, title.slug.clone()))?;
                let scope = if settings.auto_sync.only_new {
                    let mut ranges: BTreeMap<u32, (u32, u32)> = BTreeMap::new();
                    for entry in &announced {
                        let range = ranges
                            .entry(entry.season)
                            .or_insert((entry.number, entry.number));
                        range.0 = range.0.min(entry.number);
                        range.1 = range.1.max(entry.number);
                    }
                    Scope::Announced(ranges)
                } else {
                    Scope::Full
                };
                Some((title, scope, announced))
            })
            .collect()
    }

    /// Checks one title: reads its seasons in `scope` and queues what [`reconcile::decide`]
    /// asks for, with the other choices of its most recent download. An announced episode
    /// that its season's page does not list yet is taken as the list announced it.
    async fn sync_title(
        &self,
        settings: &Settings,
        jobs: &[Job],
        title: &TitleRef,
        scope: &Scope,
        announced: &[NewEpisode],
        trigger: Trigger,
    ) -> Result<TitleCheck> {
        let order = &settings.languages;
        let own: Vec<&Job> = of_title(jobs, title).collect();
        let mut check = TitleCheck {
            checked_at: now(),
            trigger,
            ..TitleCheck::default()
        };
        let Some(last) = own.iter().max_by_key(|job| job.queued_at).copied() else {
            check.summary = summary(&check);
            return Ok(check);
        };
        // Files in the library count, also ones Kura did not download; nothing else does.
        let files = self.title_files(title, &own)?;
        let held = reconcile::held(order, &files, last_language(&own));
        let allowed = reconcile::allowed(order, &held);

        let site = self.site(title.site)?;
        let series = twice(|| site.series(&title.slug)).await?;
        // Season 0 holds films and specials, which nobody asked to follow.
        let mut seasons: Vec<u32> = match scope {
            Scope::Full => series.seasons.clone(),
            Scope::Announced(ranges) => ranges.keys().copied().collect(),
        };
        seasons.extend(announced.iter().map(|entry| entry.season));
        seasons.retain(|season| *season > 0);
        seasons.sort_unstable();
        seasons.dedup();
        let in_scope = |season: u32, number: u32| match scope {
            Scope::Full => true,
            Scope::Announced(ranges) => ranges
                .get(&season)
                .is_some_and(|(first, last)| (*first..=*last).contains(&number)),
        };

        let mut errors = Vec::new();
        let mut listed_any = false;
        for &season in &seasons {
            let mut listed = match twice(|| site.episodes(&title.slug, season)).await {
                Ok(listed) => listed,
                Err(err) => {
                    errors.push(format!("season {season}: {err}"));
                    Vec::new()
                }
            };
            listed_any |= !listed.is_empty();
            for extra in from_feed(announced, season) {
                if !listed.iter().any(|episode| episode.number == extra.number) {
                    listed.push(extra);
                }
            }
            for episode in listed
                .iter()
                .filter(|episode| in_scope(season, episode.number))
            {
                let key = (season, episode.number);
                check.newest_listed = check.newest_listed.max(Some(key));
                let (language, replaces) =
                    match reconcile::decide(episode, &files, &own, order, &allowed) {
                        Decision::New(language) => (language, Vec::new()),
                        Decision::Upgrade(language, files) => (
                            language,
                            files
                                .iter()
                                .map(|file| file.to_string_lossy().into_owned())
                                .collect(),
                        ),
                        Decision::Waiting(offered) => {
                            check.waiting.push(Waiting {
                                episode: key,
                                offered,
                            });
                            continue;
                        }
                        Decision::OnDisk | Decision::InQueue => continue,
                    };
                let id = EpisodeId {
                    site: title.site,
                    slug: title.slug.clone(),
                    season,
                    number: episode.number,
                };
                let new = NewJob {
                    episode: &id,
                    series_title: &series.summary.title,
                    user_id: None,
                    // Exactly this language: no fallback to another one in the settings.
                    language: Some(language),
                    hoster: last.hoster.as_deref(),
                    dir: &last.dir,
                    imdb_id: last.imdb_id.as_deref(),
                    now: now(),
                    // It is not on disk in this language, whatever the queue remembers.
                    replace_done: true,
                    replaces: &replaces,
                };
                if self.store.enqueue(&new)?.is_some() {
                    check.queued.push(key);
                    if !replaces.is_empty() {
                        check.replacing.push(key);
                    }
                }
            }
        }
        // A page that lists nothing for seasons with downloads is a page Kura cannot read.
        if !seasons.is_empty() && !listed_any && errors.is_empty() {
            let seasons: Vec<String> = seasons.iter().map(u32::to_string).collect();
            errors.push(format!(
                "the site listed no episodes for season {}",
                seasons.join(", ")
            ));
        }
        if !errors.is_empty() {
            check.error = Some(errors.join("; "));
        }
        check.summary = summary(&check);
        tracing::info!(
            "Auto-Sync {}: in {}, {} queued, {} replacing, {} waiting",
            title.title,
            if held.is_empty() {
                "any language".to_owned()
            } else {
                held.iter()
                    .map(|language| language_name(*language))
                    .collect::<Vec<_>>()
                    .join(", ")
            },
            check.queued.len(),
            check.replacing.len(),
            check.waiting.len(),
        );
        Ok(check)
    }

    /// Every episode file of a title in the libraries, found by the IMDb ID of its most
    /// recent download or by its name.
    fn title_files(&self, title: &TitleRef, own: &[&Job]) -> Result<Vec<Found>> {
        let last = own.iter().max_by_key(|job| job.queued_at);
        let imdb_id = last.and_then(|job| job.imdb_id.as_deref());
        self.downloaded_with_language(title.site, &title.slug, &title.title, imdb_id)
    }

    /// The last check of every title, as saved. One saved by another version that cannot be
    /// read any more counts as not checked.
    fn sync_checks(&self) -> Result<HashMap<(SiteId, String), TitleCheck>> {
        let saved = self.store.sync_results()?;
        Ok(saved
            .into_iter()
            .filter_map(|(site, slug, json)| {
                Some(((site, slug), serde_json::from_str(&json).ok()?))
            })
            .collect())
    }

    /// Runs Auto-Sync on its schedule until the store fails. Run it once, for the life of
    /// the server.
    pub async fn run_auto_sync(self: Arc<Self>) -> Result<()> {
        loop {
            let auto = self.store.settings()?.auto_sync;
            let last = self.store.last_sync_run()?;
            let next = auto.enabled.then(|| next_run(&auto.schedule, last, now()));
            self.sync_state().next_run = next;

            let due = async {
                match next {
                    Some(at) => {
                        let seconds = u64::try_from(at - now()).unwrap_or(0);
                        tokio::time::sleep(Duration::from_secs(seconds)).await;
                    }
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                () = due => {
                    if let Err(err) = self.sync_feed().await {
                        tracing::warn!("Auto-Sync run failed: {err}");
                    }
                }
                // The settings changed: work out the next run again.
                () = self.sync_wake.notified() => {}
            }
        }
    }

    fn sync_state(&self) -> MutexGuard<'_, SyncState> {
        self.sync.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn of_title<'a>(jobs: &'a [Job], title: &'a TitleRef) -> impl Iterator<Item = &'a Job> {
    let same = |job: &&Job| job.episode.site == title.site && job.episode.slug == title.slug;
    jobs.iter().filter(same)
}

/// Series with at least one download that started, minus excluded titles, disabled sites
/// and titles removed since their last start, by title. A title is followed from the moment
/// its first download starts, not only once one finished.
fn tracked(settings: &Settings, jobs: &[Job]) -> Vec<TitleRef> {
    let mut titles: Vec<TitleRef> = Vec::new();
    let started: Vec<&Job> = jobs.iter().filter(|job| job.started()).collect();
    // Downloads from before 1.2.10 know only when they were queued.
    let started_at = |job: &Job| job.started_at.unwrap_or(job.queued_at);
    for job in &started {
        let (site, slug) = (job.episode.site, &job.episode.slug);
        let is_it = |title: &TitleRef| title.site == site && title.slug == *slug;
        let removed = settings.auto_sync.removed.iter().any(|removed| {
            is_it(&removed.title)
                && !started.iter().any(|other| {
                    other.episode.site == site
                        && other.episode.slug == *slug
                        && started_at(other) > removed.at
                })
        });
        let skipped = settings.disabled_sites.contains(&site)
            || settings.auto_sync.excluded.iter().any(is_it)
            || removed
            || titles.iter().any(is_it);
        if !skipped {
            titles.push(TitleRef {
                site,
                slug: slug.clone(),
                title: job.series_title.clone(),
            });
        }
    }
    titles.sort_by_key(|title| title.title.to_lowercase());
    titles
}

/// The language the title's most recent download was queued in.
fn last_language(own: &[&Job]) -> Option<Language> {
    let last = own.iter().max_by_key(|job| job.queued_at);
    last.and_then(|job| job.language)
}

/// Episodes of a season the list of new episodes announced, with every language it named.
fn from_feed(announced: &[NewEpisode], season: u32) -> Vec<Episode> {
    let mut episodes: Vec<Episode> = Vec::new();
    for entry in announced.iter().filter(|entry| entry.season == season) {
        let index = match episodes
            .iter()
            .position(|episode| episode.number == entry.number)
        {
            Some(index) => index,
            None => {
                episodes.push(Episode {
                    season,
                    number: entry.number,
                    title: String::new(),
                    languages: Vec::new(),
                });
                episodes.len() - 1
            }
        };
        let languages = &mut episodes[index].languages;
        if let Some(language) = entry.language
            && !languages.contains(&language)
        {
            languages.push(language);
        }
    }
    episodes
}

/// Asks the site once more after a short wait when the first try fails.
async fn twice<T, F, Fut>(fetch: F) -> kura_sites::Result<T>
where
    F: Fn() -> Fut,
    Fut: Future<Output = kura_sites::Result<T>>,
{
    match fetch().await {
        Err(err) => {
            tracing::debug!("Auto-Sync: asking the site again: {err}");
            tokio::time::sleep(FETCH_PAUSE).await;
            fetch().await
        }
        fetched => fetched,
    }
}

/// `S02E005`, the same form as in file names.
fn code((season, number): (u32, u32)) -> String {
    format!("S{season:02}E{number:03}")
}

/// Up to three episode codes, and how many more.
fn codes(episodes: &[(u32, u32)]) -> String {
    let mut shown: Vec<String> = episodes
        .iter()
        .take(3)
        .map(|episode| code(*episode))
        .collect();
    if episodes.len() > 3 {
        shown.push(format!("{} more", episodes.len() - 3));
    }
    shown.join(", ")
}

/// The line the page shows for a check; the page lists waiting episodes on their own.
fn summary(check: &TitleCheck) -> String {
    let mut parts = Vec::new();
    if let Some(error) = &check.error {
        parts.push(format!("Check failed: {error}"));
    }
    match check.queued.len() {
        0 => {}
        1 => parts.push(format!("1 episode queued: {}", codes(&check.queued))),
        count => parts.push(format!("{count} episodes queued: {}", codes(&check.queued))),
    }
    if !check.replacing.is_empty() {
        parts.push(format!("replacing {}", codes(&check.replacing)));
    }
    if parts.is_empty() {
        "Nothing new".to_owned()
    } else {
        parts.join(" · ")
    }
}

/// Unix seconds of the next run. Without an earlier run, an interval starts at once. A daily
/// time is in the server's time zone.
fn next_run(schedule: &Schedule, last_run: Option<i64>, now: i64) -> i64 {
    match schedule {
        Schedule::Every { minutes } => {
            let after_last = last_run.map_or(now, |last| last + i64::from(*minutes) * 60);
            after_last.max(now)
        }
        Schedule::Daily { at } => {
            let time =
                parse_time(at).and_then(|(hour, minute)| NaiveTime::from_hms_opt(hour, minute, 0));
            let today = Local::now().date_naive();
            // Today's time if it is still ahead, else tomorrow's.
            let upcoming = [today, today + Days::new(1)].into_iter().find_map(|day| {
                let at = day.and_time(time?).and_local_timezone(Local).earliest()?;
                (at.timestamp() > now).then_some(at.timestamp())
            });
            // A time that does not exist on that day (clock change) waits a day.
            upcoming.unwrap_or(now + 24 * 60 * 60)
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Timelike};
    use kura_domain::JobState;

    use super::*;

    fn done_job(slug: &str, queued_at: i64) -> Job {
        Job {
            id: queued_at,
            episode: EpisodeId {
                site: SiteId::AniWorld,
                slug: slug.to_owned(),
                season: 1,
                number: 1,
            },
            series_title: slug.to_owned(),
            state: JobState::Done,
            progress: 1.0,
            error: None,
            path: None,
            language: None,
            hoster: None,
            dir: String::new(),
            hidden: false,
            queued_at,
            imdb_id: None,
            started_at: None,
            replaces: Vec::new(),
        }
    }

    #[test]
    fn removed_titles_come_back_with_the_next_download() {
        let mut settings = Settings::default();
        settings.auto_sync.removed.push(kura_domain::RemovedTitle {
            title: TitleRef {
                site: SiteId::AniWorld,
                slug: "a".to_owned(),
                title: "a".to_owned(),
            },
            at: 100,
        });
        let slugs = |jobs: &[Job]| {
            let titles = tracked(&settings, jobs);
            titles
                .into_iter()
                .map(|title| title.slug)
                .collect::<Vec<_>>()
        };
        let before = [done_job("a", 50), done_job("b", 60)];
        assert_eq!(slugs(&before), ["b"]);
        // Removed, not excluded: a later download tracks it again.
        assert_eq!(slugs(&[done_job("a", 50), done_job("a", 150)]), ["a"]);
        assert!(settings.auto_sync.excluded.is_empty());
    }

    #[test]
    fn titles_are_tracked_once_a_download_starts() {
        let settings = Settings::default();
        let slugs = |jobs: &[Job]| {
            let titles = tracked(&settings, jobs);
            titles.into_iter().map(|t| t.slug).collect::<Vec<_>>()
        };
        let mut waiting = done_job("a", 10);
        waiting.state = JobState::Queued;
        assert!(slugs(&[waiting.clone()]).is_empty());
        let mut running = waiting.clone();
        running.state = JobState::Running;
        running.started_at = Some(20);
        assert_eq!(slugs(&[running.clone()]), ["a"]);
        // A started download that failed keeps the title tracked.
        let mut failed = running;
        failed.state = JobState::Failed;
        assert_eq!(slugs(&[failed]), ["a"]);
    }

    #[test]
    fn interval_runs_follow_the_last_run() {
        let every = Schedule::Every { minutes: 15 };
        assert_eq!(next_run(&every, None, 1_000), 1_000);
        assert_eq!(next_run(&every, Some(1_000), 1_100), 1_900);
        // A run that is overdue happens now, not in the past.
        assert_eq!(next_run(&every, Some(1_000), 5_000), 5_000);
    }

    #[test]
    fn daily_runs_are_at_the_given_local_time_within_a_day() {
        let current = now();
        let daily = Schedule::Daily { at: "03:30".into() };
        let next = next_run(&daily, None, current);
        assert!(next > current && next <= current + 25 * 60 * 60);
        let local = DateTime::from_timestamp(next, 0)
            .unwrap()
            .with_timezone(&Local);
        assert_eq!((local.hour(), local.minute()), (3, 30));
    }
}
