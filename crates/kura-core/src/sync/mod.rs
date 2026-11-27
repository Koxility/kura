//! Auto-Sync: queues new episodes of series that were downloaded before.
//!
//! Every run reads each tracked title from the site and decides about every listed episode
//! (see [`reconcile`]): what is on disk in the title's language is there, anything else that
//! is wanted and offered in that language is queued. What a check found is kept in the
//! database, so the page shows it also after a restart.

mod reconcile;

use std::{
    collections::HashMap,
    future::Future,
    sync::{Arc, MutexGuard, PoisonError},
    time::Duration,
};

use chrono::{Days, Local, NaiveTime};
use kura_domain::{EpisodeId, Job, JobState, Language, Schedule, Settings, SiteId, TitleRef};
use kura_store::{NewJob, StoreError};
use reconcile::{Decision, Title};
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
    pub titles: Vec<TrackedTitle>,
}

/// A series Auto-Sync watches: a download of at least one episode of it started.
#[derive(Debug, Serialize)]
pub struct TrackedTitle {
    #[serde(flatten)]
    pub title: TitleRef,
    /// The language Auto-Sync follows for it; `None` while nothing tells one.
    pub language: Option<Language>,
    /// Episodes in the libraries in that language, or in files whose name tells none, now.
    pub on_disk: usize,
    pub newest_on_disk: Option<(u32, u32)>,
    /// What the last check found; `None` before the first.
    pub check: Option<TitleCheck>,
}

/// What one check of a title found.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TitleCheck {
    pub checked_at: i64,
    /// Why the check failed, or which seasons could not be read.
    pub error: Option<String>,
    /// The newest episode the site lists, in any language, of the seasons checked.
    pub newest_listed: Option<(u32, u32)>,
    /// Episodes queued by this check, among them those below.
    pub queued: Vec<(u32, u32)>,
    /// Failed downloads queued again.
    pub retried: Vec<(u32, u32)>,
    /// Episodes queued in the title's language that replace a file in another one.
    pub replacing: Vec<(u32, u32)>,
    /// Wanted episodes the site does not offer in the title's language yet.
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

impl Core {
    pub fn sync_status(&self) -> Result<SyncStatus> {
        let settings = self.store.settings()?;
        let jobs = self.store.jobs()?;
        let mut checks = self.sync_checks()?;
        let mut titles = Vec::new();
        for title in tracked(&settings, &jobs) {
            let own: Vec<&Job> = of_title(&jobs, &title).collect();
            let files = self.title_files(&title, &own)?;
            let language = language_of(&settings.languages, &files, &own);
            let present = reconcile::present(language, &files);
            titles.push(TrackedTitle {
                language,
                on_disk: present.len(),
                newest_on_disk: present.last().copied(),
                check: checks.remove(&(title.site, title.slug.clone())),
                title,
            });
        }
        Ok(SyncStatus {
            enabled: settings.auto_sync.enabled,
            last_run: self.store.last_sync_run()?,
            next_run: self
                .sync_state()
                .next_run
                .filter(|_| settings.auto_sync.enabled),
            titles,
        })
    }

    /// Checks every tracked title once and returns how many episodes were queued. While a run
    /// is under way, another one waits for it and returns its result instead of checking
    /// everything a second time.
    pub async fn sync_once(&self) -> Result<usize> {
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
        let titles = tracked(&settings, &jobs);
        let (mut total, mut failed) = (0, 0);
        for title in &titles {
            let check = match self.sync_title(&settings, &jobs, title).await {
                Ok(check) => check,
                Err(err) => {
                    let error = err.to_string();
                    TitleCheck {
                        checked_at: now(),
                        summary: format!("Check failed: {error}"),
                        error: Some(error),
                        ..TitleCheck::default()
                    }
                }
            };
            total += check.queued.len();
            failed += usize::from(check.error.is_some());
            let json = serde_json::to_string(&check).map_err(StoreError::from)?;
            self.store
                .save_sync_result(title.site, &title.slug, &json)?;
        }
        self.store.set_last_sync_run(now())?;
        self.sync_state().last_queued = total;
        tracing::info!(
            "Auto-Sync checked {} titles: {total} episodes queued, {failed} checks failed",
            titles.len()
        );
        // A run the user started counts as the last run, so the schedule moves with it.
        self.sync_wake.notify_one();
        if total > 0 {
            self.changed();
        }
        Ok(total)
    }

    /// Checks one title: reads the seasons that can hold something to do and queues what
    /// [`reconcile::decide`] asks for, in the title's language and with the other choices of
    /// its most recent download.
    async fn sync_title(
        &self,
        settings: &Settings,
        jobs: &[Job],
        title: &TitleRef,
    ) -> Result<TitleCheck> {
        let only_new = settings.auto_sync.only_new;
        let own: Vec<&Job> = of_title(jobs, title).collect();
        let mut check = TitleCheck {
            checked_at: now(),
            ..TitleCheck::default()
        };
        let Some(last) = own.iter().max_by_key(|job| job.queued_at).copied() else {
            check.summary = summary(&check, None);
            return Ok(check);
        };
        // Files in the library count too, also ones Kura did not download.
        let files = self.title_files(title, &own)?;
        let language = language_of(&settings.languages, &files, &own);
        let anchor = reconcile::anchor(language, &files, &own);
        // Seasons before the newest one only matter for a failed download or a file in
        // another language.
        let pending: Vec<u32> = own
            .iter()
            .filter(|job| job.state == JobState::Failed)
            .map(|job| job.episode.season)
            .chain(
                files
                    .iter()
                    .filter(|file| !reconcile::fits(language, file.language))
                    .map(|file| file.episode.0),
            )
            .collect();
        let wanted = |season: u32| {
            pending.contains(&season)
                || anchor.is_some_and(|(newest, _)| !only_new || season >= newest)
        };
        if anchor.is_none() && pending.is_empty() {
            // Nothing finished yet: the first download decides where to go on.
            check.summary = summary(&check, language);
            return Ok(check);
        }

        let site = self.site(title.site)?;
        let series = twice(|| site.series(&title.slug)).await?;
        // Season 0 holds films and specials, which nobody asked to follow.
        let seasons: Vec<u32> = series
            .seasons
            .iter()
            .copied()
            .filter(|season| *season > 0 && wanted(*season))
            .collect();
        let context = Title {
            language,
            order: &settings.languages,
            files: &files,
            jobs: &own,
            anchor,
            only_new,
        };
        let mut errors = Vec::new();
        let mut listed_any = false;
        for &season in &seasons {
            let listed = match twice(|| site.episodes(&title.slug, season)).await {
                Ok(listed) => listed,
                Err(err) => {
                    errors.push(format!("season {season}: {err}"));
                    continue;
                }
            };
            listed_any |= !listed.is_empty();
            for episode in &listed {
                let key = (season, episode.number);
                check.newest_listed = check.newest_listed.max(Some(key));
                let decision = reconcile::decide(episode, &context);
                if let Decision::Waiting(offered) = &decision {
                    check.waiting.push(Waiting {
                        episode: key,
                        offered: offered.clone(),
                    });
                }
                if !decision.queues() {
                    continue;
                }
                let replaces: Vec<String> = match &decision {
                    Decision::Upgrade(files) => files
                        .iter()
                        .map(|file| file.to_string_lossy().into_owned())
                        .collect(),
                    _ => Vec::new(),
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
                    language,
                    hoster: last.hoster.as_deref(),
                    dir: &last.dir,
                    imdb_id: last.imdb_id.as_deref(),
                    now: now(),
                    // Only what `decide` asked for gets here: never an episode someone
                    // deleted, but one on record in another language.
                    replace_done: true,
                    replaces: &replaces,
                };
                if self.store.enqueue(&new)?.is_some() {
                    check.queued.push(key);
                    match decision {
                        Decision::Retry => check.retried.push(key),
                        Decision::Upgrade(_) => check.replacing.push(key),
                        _ => {}
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
        check.summary = summary(&check, language);
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
                    if let Err(err) = self.sync_once().await {
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

/// The language a title follows (see [`reconcile::title_language`]); with nothing that tells
/// one, the language its most recent download was queued in.
fn language_of(order: &[Language], files: &[Found], own: &[&Job]) -> Option<Language> {
    reconcile::title_language(order, files, own).or_else(|| {
        let last = own.iter().max_by_key(|job| job.queued_at);
        last.and_then(|job| job.language)
    })
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

/// `S02E05`.
fn code((season, number): (u32, u32)) -> String {
    format!("S{season:02}E{number:02}")
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

/// The line the page shows for a check.
fn summary(check: &TitleCheck, language: Option<Language>) -> String {
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
    if !check.retried.is_empty() {
        parts.push(format!("tried again {}", codes(&check.retried)));
    }
    if !check.waiting.is_empty() {
        let episodes: Vec<_> = check
            .waiting
            .iter()
            .map(|waiting| waiting.episode)
            .collect();
        let verb = if episodes.len() == 1 { "waits" } else { "wait" };
        let language = language.map_or("its language", language_name);
        parts.push(format!("{} {verb} for {language}", codes(&episodes)));
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
