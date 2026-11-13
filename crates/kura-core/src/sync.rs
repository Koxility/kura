//! Auto-Sync: queues new episodes of series that were downloaded before.

use std::{
    collections::HashMap,
    sync::{Arc, MutexGuard, PoisonError},
    time::Duration,
};

use chrono::{Days, Local, NaiveTime};
use kura_domain::{AutoSync, EpisodeId, Job, JobState, Schedule, Settings, SiteId, TitleRef};
use kura_store::NewJob;
use serde::Serialize;

use crate::{Core, Result, auth::now, parse_time};

/// Kept in memory only: after a restart the results start empty and fill with the next run.
#[derive(Default)]
pub(crate) struct SyncState {
    last_run: Option<i64>,
    next_run: Option<i64>,
    /// Per title: when it was checked and what came of it.
    results: HashMap<(SiteId, String), (i64, String)>,
}

/// What the Auto-Sync page shows. Times are Unix seconds.
#[derive(Debug, Serialize)]
pub struct SyncStatus {
    pub enabled: bool,
    pub last_run: Option<i64>,
    pub next_run: Option<i64>,
    pub titles: Vec<TrackedTitle>,
}

/// A series Auto-Sync watches: at least one episode of it was downloaded.
#[derive(Debug, Serialize)]
pub struct TrackedTitle {
    #[serde(flatten)]
    pub title: TitleRef,
    /// Episodes downloaded so far.
    pub downloaded: usize,
    pub checked_at: Option<i64>,
    pub result: Option<String>,
}

impl Core {
    pub fn sync_status(&self) -> Result<SyncStatus> {
        let settings = self.store.settings()?;
        let jobs = self.store.jobs()?;
        let state = self.sync_state();
        let titles = tracked(&settings, &jobs).into_iter().map(|title| {
            let result = state.results.get(&(title.site, title.slug.clone()));
            TrackedTitle {
                downloaded: done(&jobs, &title).count(),
                checked_at: result.map(|(at, _)| *at),
                result: result.map(|(_, text)| text.clone()),
                title,
            }
        });
        Ok(SyncStatus {
            enabled: settings.auto_sync.enabled,
            last_run: state.last_run,
            next_run: state.next_run.filter(|_| settings.auto_sync.enabled),
            titles: titles.collect(),
        })
    }

    /// Checks every tracked title once and returns how many episodes were queued.
    pub async fn sync_once(&self) -> Result<usize> {
        let settings = self.store.settings()?;
        let jobs = self.store.jobs()?;
        let mut total = 0;
        for title in tracked(&settings, &jobs) {
            let queued = self.sync_title(&settings.auto_sync, &jobs, &title).await;
            let text = match &queued {
                Ok(0) => "Nothing new".to_owned(),
                Ok(1) => "1 new episode queued".to_owned(),
                Ok(count) => format!("{count} new episodes queued"),
                Err(err) => format!("Check failed: {err}"),
            };
            total += queued.unwrap_or(0);
            let key = (title.site, title.slug);
            self.sync_state().results.insert(key, (now(), text));
        }
        self.sync_state().last_run = Some(now());
        // A run the user started counts as the last run, so the schedule moves with it.
        self.sync_wake.notify_one();
        if total > 0 {
            self.changed();
        }
        Ok(total)
    }

    /// Queues the episodes of one title that are not known yet, with the choices of its most
    /// recent download.
    async fn sync_title(&self, auto: &AutoSync, jobs: &[Job], title: &TitleRef) -> Result<usize> {
        let pair = |job: &Job| (job.episode.season, job.episode.number);
        let known: Vec<_> = of_title(jobs, title).map(pair).collect();
        let newest = done(jobs, title).map(pair).max();
        let last = of_title(jobs, title).max_by_key(|job| job.queued_at);
        let (Some(newest), Some(last)) = (newest, last) else {
            return Ok(0);
        };

        let site = self.site(title.site)?;
        let series = site.series(&title.slug).await?;
        let mut queued = 0;
        // Season 0 holds films and specials, which nobody asked to follow.
        for &season in series.seasons.iter().filter(|season| **season > 0) {
            if auto.only_new && season < newest.0 {
                continue;
            }
            let listed = site.episodes(&title.slug, season).await?;
            let listed: Vec<_> = listed
                .iter()
                .map(|episode| (season, episode.number))
                .collect();
            for (season, number) in missing(&known, &listed, auto.only_new.then_some(newest)) {
                let episode = EpisodeId {
                    site: title.site,
                    slug: title.slug.clone(),
                    season,
                    number,
                };
                let new = NewJob {
                    episode: &episode,
                    series_title: &series.summary.title,
                    user_id: None,
                    language: last.language,
                    hoster: last.hoster.as_deref(),
                    dir: &last.dir,
                    imdb_id: last.imdb_id.as_deref(),
                    now: now(),
                };
                queued += usize::from(self.store.enqueue(&new)?.is_some());
            }
        }
        Ok(queued)
    }

    /// Runs Auto-Sync on its schedule until the store fails. Run it once, for the life of
    /// the server.
    pub async fn run_auto_sync(self: Arc<Self>) -> Result<()> {
        loop {
            let auto = self.store.settings()?.auto_sync;
            let last = self.sync_state().last_run;
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

fn done<'a>(jobs: &'a [Job], title: &'a TitleRef) -> impl Iterator<Item = &'a Job> {
    of_title(jobs, title).filter(|job| job.state == JobState::Done)
}

/// Series with at least one downloaded episode, minus excluded titles and disabled sites,
/// by title.
fn tracked(settings: &Settings, jobs: &[Job]) -> Vec<TitleRef> {
    let mut titles: Vec<TitleRef> = Vec::new();
    for job in jobs.iter().filter(|job| job.state == JobState::Done) {
        let (site, slug) = (job.episode.site, &job.episode.slug);
        let is_it = |title: &TitleRef| title.site == site && title.slug == *slug;
        let skipped = settings.disabled_sites.contains(&site)
            || settings.auto_sync.excluded.iter().any(is_it)
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

/// Listed `(season, episode)` pairs that are not known yet. With `after`, only those later
/// than that pair; without, gaps are filled too.
fn missing(
    known: &[(u32, u32)],
    listed: &[(u32, u32)],
    after: Option<(u32, u32)>,
) -> Vec<(u32, u32)> {
    let wanted =
        |pair: &&(u32, u32)| !known.contains(pair) && after.is_none_or(|newest| **pair > newest);
    listed.iter().filter(wanted).copied().collect()
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

    #[test]
    fn only_new_skips_gaps_and_filling_takes_them() {
        let known = [(1, 2), (1, 3)];
        let listed = [(1, 1), (1, 2), (1, 3), (1, 4)];
        assert_eq!(missing(&known, &listed, Some((1, 3))), [(1, 4)]);
        assert_eq!(missing(&known, &listed, None), [(1, 1), (1, 4)]);
        // A new season counts as later than any episode of the one before.
        assert_eq!(missing(&known, &[(2, 1)], Some((1, 3))), [(2, 1)]);
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
