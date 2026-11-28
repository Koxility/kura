//! What Auto-Sync does with each episode a site lists, from what is on disk and on record.
//! No site and no disk is read here, so every rule can be tested on its own.
//!
//! A title follows one language: of the languages it has, on disk or downloaded before, the
//! one the settings rank highest. An episode counts as there only in that language, or in a
//! file whose name tells no language. A download on record in another language, or one that
//! failed, does not count.

use std::{cmp::Reverse, path::PathBuf};

use kura_domain::{Episode, Job, JobState, Language};

use crate::downloaded::{Found, on_disk};

/// What one title has, for deciding about its listed episodes.
pub(crate) struct Title<'a> {
    /// `None` when nothing tells a language; then any language will do.
    pub language: Option<Language>,
    /// The order of the settings, best first.
    pub order: &'a [Language],
    pub files: &'a [Found],
    /// The title's jobs; one per episode at most.
    pub jobs: &'a [&'a Job],
    /// Where the title stands in its language; `None` while nothing finished.
    pub anchor: Option<Anchor>,
    pub only_new: bool,
}

/// The newest episode a title has in its language, and the first it has of that season. With
/// `only_new`, what comes after the first counts: a gap between two downloaded episodes is
/// an episode that came out late (a dub out of order), not one somebody passed over.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Anchor {
    pub newest: (u32, u32),
    pub first: (u32, u32),
}

impl Anchor {
    /// Whether "only new" wants this episode.
    fn wants(&self, episode: (u32, u32)) -> bool {
        episode > self.newest || (episode.0 == self.newest.0 && episode > self.first)
    }
}

/// What becomes of one listed episode.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Decision {
    /// A file in the title's language, or one whose name tells no language, is there.
    OnDisk,
    /// Queued or downloading.
    InQueue,
    /// Someone cancelled its download: left alone.
    Cancelled,
    /// Downloaded in the title's language and deleted since: left alone.
    Deleted,
    /// Only in a language the settings rank lower (this one), and the title's language is
    /// not out yet.
    Held(Option<Language>),
    /// Only in a language the settings rank lower, and the title's language is out: download
    /// it, then delete these files.
    Upgrade(Vec<PathBuf>),
    /// Not wanted: with `only_new`, before the first episode the title has of its newest
    /// season or in an earlier season; or nothing of the title finished yet.
    Skipped,
    /// Wanted, but not offered in the title's language yet; these are.
    Waiting(Vec<Language>),
    /// Its download failed: queued again.
    Retry,
    New,
}

impl Decision {
    /// Whether Auto-Sync queues the episode.
    pub(crate) fn queues(&self) -> bool {
        matches!(self, Self::Upgrade(_) | Self::Retry | Self::New)
    }
}

/// Place of a language in the settings; languages they do not name come last.
fn rank(order: &[Language], language: Language) -> usize {
    order
        .iter()
        .position(|wanted| *wanted == language)
        .unwrap_or(order.len())
}

fn pair(job: &Job) -> (u32, u32) {
    (job.episode.season, job.episode.number)
}

/// Of the languages a title has, on disk or in finished downloads, the one the settings rank
/// highest; between languages they do not name, the newest episode's.
pub(crate) fn title_language(
    order: &[Language],
    files: &[Found],
    jobs: &[&Job],
) -> Option<Language> {
    let on_disk = files
        .iter()
        .filter_map(|file| Some((file.episode, file.language?)));
    let finished = jobs
        .iter()
        .filter(|job| job.state == JobState::Done)
        .filter_map(|job| Some((pair(job), job.language?)));
    on_disk
        .chain(finished)
        .min_by_key(|(episode, language)| (rank(order, *language), Reverse(*episode)))
        .map(|(_, language)| language)
}

/// Whether something in `had` counts as the title's language.
pub(crate) fn fits(language: Option<Language>, had: Option<Language>) -> bool {
    language.is_none() || had.is_none() || had == language
}

/// Where the title stands in its language, from what is on disk and from finished downloads
/// whose files are gone, so watching and deleting does not make old episodes new again.
pub(crate) fn anchor(language: Option<Language>, files: &[Found], jobs: &[&Job]) -> Option<Anchor> {
    let on_disk = files
        .iter()
        .filter(|file| fits(language, file.language))
        .map(|file| file.episode);
    let finished = jobs
        .iter()
        .filter(|job| job.state == JobState::Done && fits(language, job.language))
        .map(|job| pair(job));
    let had: Vec<(u32, u32)> = on_disk.chain(finished).collect();
    let newest = had.iter().copied().max()?;
    let first = had
        .iter()
        .copied()
        .filter(|episode| episode.0 == newest.0)
        .min()?;
    Some(Anchor { newest, first })
}

/// Episodes on disk in the title's language, or in a file whose name tells none, sorted.
pub(crate) fn present(language: Option<Language>, files: &[Found]) -> Vec<(u32, u32)> {
    let mut episodes: Vec<_> = files
        .iter()
        .filter(|file| fits(language, file.language))
        .map(|file| file.episode)
        .collect();
    episodes.sort_unstable();
    episodes.dedup();
    episodes
}

pub(crate) fn decide(episode: &Episode, title: &Title) -> Decision {
    let key = (episode.season, episode.number);
    let files: Vec<&Found> = title
        .files
        .iter()
        .filter(|file| file.episode == key)
        .collect();
    if files.iter().any(|file| fits(title.language, file.language)) {
        return Decision::OnDisk;
    }
    let job = title.jobs.iter().find(|job| pair(job) == key);
    match job {
        Some(job) if matches!(job.state, JobState::Queued | JobState::Running) => {
            return Decision::InQueue;
        }
        Some(job) if job.state == JobState::Cancelled => return Decision::Cancelled,
        Some(job)
            if job.state == JobState::Done
                && fits(title.language, job.language)
                && !on_disk(job) =>
        {
            return Decision::Deleted;
        }
        _ => {}
    }

    let offered = title
        .language
        .is_none_or(|wanted| episode.languages.contains(&wanted));
    // Every file left is in another language, so one the settings rank no higher.
    if !files.is_empty() {
        let better = title.language.is_some_and(|wanted| {
            files.iter().all(|file| {
                file.language
                    .is_some_and(|had| rank(title.order, wanted) < rank(title.order, had))
            })
        });
        return match (better, offered) {
            (true, true) => Decision::Upgrade(files.iter().map(|file| file.path.clone()).collect()),
            _ => Decision::Held(files[0].language),
        };
    }

    let failed = job.is_some_and(|job| job.state == JobState::Failed);
    let wanted = failed
        || title
            .anchor
            .is_some_and(|anchor| !title.only_new || anchor.wants(key));
    if !wanted {
        Decision::Skipped
    } else if !offered {
        Decision::Waiting(episode.languages.clone())
    } else if failed {
        Decision::Retry
    } else {
        Decision::New
    }
}

#[cfg(test)]
mod tests {
    use kura_domain::{EpisodeId, SiteId};

    use super::*;
    use Language::{EnglishSub, GermanDub as Dub, GermanSub as Sub};

    const ORDER: [Language; 2] = [Dub, Sub];

    fn job(number: u32, state: JobState, language: Option<Language>) -> Job {
        Job {
            id: i64::from(number),
            episode: EpisodeId {
                site: SiteId::AniWorld,
                slug: "s".to_owned(),
                season: 1,
                number,
            },
            series_title: "S".to_owned(),
            state,
            progress: 0.0,
            error: None,
            // Never on disk: files come from `Found`.
            path: Some("/nowhere/S S01E001.mkv".to_owned()),
            language,
            hoster: None,
            dir: String::new(),
            hidden: false,
            queued_at: 0,
            imdb_id: None,
            started_at: Some(0),
            replaces: Vec::new(),
        }
    }

    fn file(number: u32, language: Option<Language>) -> Found {
        Found {
            episode: (1, number),
            language,
            path: PathBuf::from(format!("/lib/S/Season 01/S S01E{number:03}.mkv")),
        }
    }

    fn listed(number: u32, languages: &[Language]) -> Episode {
        Episode {
            season: 1,
            number,
            title: String::new(),
            languages: languages.to_vec(),
        }
    }

    #[test]
    fn the_title_follows_the_best_language_it_has() {
        let files = [file(1, Some(Dub)), file(2, Some(Sub))];
        // The newest episode is a sub, but the settings rank the dub first.
        assert_eq!(title_language(&ORDER, &files, &[]), Some(Dub));
        let sub_first = [Sub, Dub];
        assert_eq!(title_language(&sub_first, &files, &[]), Some(Sub));
        // A finished download counts too, also once its file is gone.
        let gone = job(3, JobState::Done, Some(Dub));
        assert_eq!(
            title_language(&ORDER, &[file(2, Some(Sub))], &[&gone]),
            Some(Dub)
        );
        // Languages the settings do not name: the newest one's.
        let files = [file(1, Some(EnglishSub)), file(2, Some(Language::English))];
        assert_eq!(title_language(&ORDER, &files, &[]), Some(Language::English));
        // A failed download tells nothing.
        let failed = job(1, JobState::Failed, Some(Sub));
        assert_eq!(title_language(&ORDER, &[], &[&failed]), None);
    }

    #[test]
    fn the_anchor_is_the_newest_episode_in_the_title_language() {
        let files = [file(1, Some(Dub)), file(2, None), file(5, Some(Sub))];
        let deleted = job(3, JobState::Done, Some(Dub));
        let other = job(4, JobState::Done, Some(Sub));
        let found = anchor(Some(Dub), &files, &[&deleted, &other]);
        assert_eq!(
            found,
            Some(Anchor {
                newest: (1, 3),
                first: (1, 1)
            })
        );
        assert_eq!(anchor(Some(Dub), &[], &[]), None);
    }

    fn decide_with(episode: &Episode, files: &[Found], jobs: &[&Job]) -> Decision {
        let title = Title {
            language: Some(Dub),
            order: &ORDER,
            files,
            jobs,
            anchor: Some(Anchor {
                newest: (1, 2),
                first: (1, 2),
            }),
            only_new: true,
        };
        decide(episode, &title)
    }

    #[test]
    fn episodes_are_decided_by_disk_queue_and_language() {
        let both = [Dub, Sub];
        // On disk in the title language, or in a name without one.
        assert_eq!(
            decide_with(&listed(1, &both), &[file(1, Some(Dub))], &[]),
            Decision::OnDisk
        );
        assert_eq!(
            decide_with(&listed(1, &both), &[file(1, None)], &[]),
            Decision::OnDisk
        );
        // Up to the anchor: not wanted; after it: new, or waiting for the language.
        assert_eq!(decide_with(&listed(1, &both), &[], &[]), Decision::Skipped);
        assert_eq!(decide_with(&listed(3, &both), &[], &[]), Decision::New);
        assert_eq!(
            decide_with(&listed(3, &[Sub]), &[], &[]),
            Decision::Waiting(vec![Sub])
        );

        // The case that went wrong: a download in another language is on record, its file
        // is gone. The dub is new.
        let sub_gone = job(3, JobState::Done, Some(Sub));
        assert_eq!(
            decide_with(&listed(3, &both), &[], &[&sub_gone]),
            Decision::New
        );
        // The dub was downloaded and deleted: it stays deleted.
        let dub_gone = job(3, JobState::Done, Some(Dub));
        assert_eq!(
            decide_with(&listed(3, &both), &[], &[&dub_gone]),
            Decision::Deleted
        );

        let queued = job(3, JobState::Queued, Some(Dub));
        assert_eq!(
            decide_with(&listed(3, &both), &[], &[&queued]),
            Decision::InQueue
        );
        let cancelled = job(3, JobState::Cancelled, Some(Dub));
        assert_eq!(
            decide_with(&listed(3, &both), &[], &[&cancelled]),
            Decision::Cancelled
        );
        // Failed: tried again, also up to the anchor, once it is offered.
        let failed = job(1, JobState::Failed, Some(Sub));
        assert_eq!(
            decide_with(&listed(1, &both), &[], &[&failed]),
            Decision::Retry
        );
        assert_eq!(
            decide_with(&listed(1, &[Sub]), &[], &[&failed]),
            Decision::Waiting(vec![Sub])
        );
    }

    #[test]
    fn a_worse_language_on_disk_is_replaced_once_the_better_one_is_out() {
        let sub = [file(1, Some(Sub))];
        assert_eq!(
            decide_with(&listed(1, &[Sub]), &sub, &[]),
            Decision::Held(Some(Sub))
        );
        assert_eq!(
            decide_with(&listed(1, &[Dub, Sub]), &sub, &[]),
            Decision::Upgrade(vec![sub[0].path.clone()])
        );
        // A language the settings do not name is never worth a download over another.
        let title = Title {
            language: Some(EnglishSub),
            order: &ORDER,
            files: &[file(1, Some(Language::English))],
            jobs: &[],
            anchor: Some(Anchor {
                newest: (1, 1),
                first: (1, 1),
            }),
            only_new: true,
        };
        assert_eq!(
            decide(&listed(1, &[EnglishSub]), &title),
            Decision::Held(Some(Language::English))
        );
    }

    #[test]
    fn without_a_finished_episode_only_failed_ones_are_wanted() {
        let title = Title {
            language: None,
            order: &ORDER,
            files: &[],
            jobs: &[],
            anchor: None,
            only_new: false,
        };
        assert_eq!(decide(&listed(5, &[Dub]), &title), Decision::Skipped);
        let failed = job(5, JobState::Failed, None);
        let jobs = [&failed];
        let title = Title {
            jobs: &jobs,
            ..title
        };
        assert_eq!(decide(&listed(5, &[Dub]), &title), Decision::Retry);
    }

    /// The Vermilion Mask: episodes 1 to 4 on disk in dub and a dub download of episode 6 on
    /// record. Episode 5 sits between them: it waits for its dub, and is new once it is out.
    #[test]
    fn a_gap_between_downloaded_episodes_is_wanted_with_only_new() {
        let files: Vec<Found> = (1..=4).map(|number| file(number, Some(Dub))).collect();
        let e6 = job(6, JobState::Done, Some(Dub));
        let jobs = [&e6];
        let found = anchor(Some(Dub), &files, &jobs);
        assert_eq!(
            found,
            Some(Anchor {
                newest: (1, 6),
                first: (1, 1)
            })
        );
        let title = Title {
            language: Some(Dub),
            order: &ORDER,
            files: &files,
            jobs: &jobs,
            anchor: found,
            only_new: true,
        };
        assert_eq!(
            decide(&listed(5, &[Sub]), &title),
            Decision::Waiting(vec![Sub])
        );
        assert_eq!(decide(&listed(5, &[Dub, Sub]), &title), Decision::New);
        assert_eq!(decide(&listed(6, &[Sub]), &title), Decision::Deleted);
        assert_eq!(
            decide(&listed(7, &[Sub]), &title),
            Decision::Waiting(vec![Sub])
        );

        // Started with episode 3: the two before it stay skipped, an earlier season too.
        let title = Title {
            files: &[],
            anchor: Some(Anchor {
                newest: (1, 6),
                first: (1, 3),
            }),
            ..title
        };
        assert_eq!(decide(&listed(2, &[Dub]), &title), Decision::Skipped);
        let earlier = Episode {
            season: 0,
            ..listed(9, &[Dub])
        };
        assert_eq!(decide(&earlier, &title), Decision::Skipped);
    }
}
