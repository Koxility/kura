//! What Auto-Sync does with each episode a site lists. Only the libraries count: an episode
//! is there while a file of it is, whatever the queue remembers. A deleted episode is
//! missing and comes again.
//!
//! A title may be held in several languages. Allowed are the languages it has on disk and
//! every language the settings rank higher than the best of them: a series in German Sub
//! keeps getting German Sub and takes German Dub once it is out, a series in German Dub never
//! gets German Sub. An episode on disk in a language ranked lower than the best allowed one
//! it is offered in is downloaded again in that one, and its old file goes.

use std::path::PathBuf;

use kura_domain::{Episode, Job, JobState, Language};

use crate::downloaded::Found;

/// What becomes of one listed episode.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Decision {
    /// On disk, in the best language it is offered in or a file whose name tells none.
    OnDisk,
    /// Queued or downloading.
    InQueue,
    /// Missing: download it in this language.
    New(Language),
    /// On disk only in a language ranked lower: download it in this one, then delete these.
    Upgrade(Language, Vec<PathBuf>),
    /// Missing, and not offered in an allowed language yet; these it is offered in.
    Waiting(Vec<Language>),
}

/// Place of a language in the settings; languages they do not name come last.
pub(crate) fn rank(order: &[Language], language: Language) -> usize {
    order
        .iter()
        .position(|wanted| *wanted == language)
        .unwrap_or(order.len())
}

/// The languages a title's files are in, best first. Without a file that tells one, the
/// language of its most recent download.
pub(crate) fn held(order: &[Language], files: &[Found], last: Option<Language>) -> Vec<Language> {
    let mut held: Vec<Language> = Vec::new();
    for language in files.iter().filter_map(|file| file.language) {
        if !held.contains(&language) {
            held.push(language);
        }
    }
    if held.is_empty() {
        held.extend(last);
    }
    held.sort_by_key(|language| rank(order, *language));
    held
}

/// What the title may be downloaded in: what it has and everything ranked higher. Without
/// anything known, the order of the settings.
pub(crate) fn allowed(order: &[Language], held: &[Language]) -> Vec<Language> {
    let Some(best) = held.iter().map(|language| rank(order, *language)).min() else {
        return order.to_vec();
    };
    let mut allowed: Vec<Language> = order
        .iter()
        .copied()
        .filter(|language| rank(order, *language) < best)
        .collect();
    // Every language held ranks at least as low as the best one, so none is in yet.
    allowed.extend(held.iter().copied());
    allowed
}

fn pair(job: &Job) -> (u32, u32) {
    (job.episode.season, job.episode.number)
}

pub(crate) fn decide(
    episode: &Episode,
    files: &[Found],
    jobs: &[&Job],
    order: &[Language],
    allowed: &[Language],
) -> Decision {
    let key = (episode.season, episode.number);
    let queued = jobs
        .iter()
        .any(|job| pair(job) == key && matches!(job.state, JobState::Queued | JobState::Running));
    if queued {
        return Decision::InQueue;
    }
    let best = episode
        .languages
        .iter()
        .copied()
        .filter(|language| allowed.contains(language))
        .min_by_key(|language| rank(order, *language));
    let files: Vec<&Found> = files.iter().filter(|file| file.episode == key).collect();
    if files.is_empty() {
        return match best {
            Some(language) => Decision::New(language),
            None => Decision::Waiting(episode.languages.clone()),
        };
    }
    // A file whose name tells no language is kept: nothing says it is worse.
    let have = files
        .iter()
        .map(|file| file.language.map(|language| rank(order, language)))
        .collect::<Option<Vec<usize>>>()
        .and_then(|ranks| ranks.into_iter().min());
    match (best, have) {
        (Some(language), Some(have)) if rank(order, language) < have => Decision::Upgrade(
            language,
            files.iter().map(|file| file.path.clone()).collect(),
        ),
        _ => Decision::OnDisk,
    }
}

#[cfg(test)]
mod tests {
    use kura_domain::{EpisodeId, SiteId};

    use super::*;
    use Language::{English, EnglishSub, GermanDub as Dub, GermanSub as Sub};

    const ORDER: [Language; 3] = [Dub, Sub, EnglishSub];

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

    fn job(number: u32, state: JobState) -> Job {
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
            path: None,
            language: Some(Dub),
            hoster: None,
            dir: String::new(),
            hidden: false,
            queued_at: 0,
            imdb_id: None,
            started_at: Some(0),
            replaces: Vec::new(),
        }
    }

    #[test]
    fn a_title_may_take_what_it_has_and_anything_ranked_higher() {
        let sub = held(&ORDER, &[file(1, Some(Sub))], None);
        assert_eq!(sub, [Sub]);
        assert_eq!(allowed(&ORDER, &sub), [Dub, Sub]);
        // A dub series never gets a sub.
        assert_eq!(allowed(&ORDER, &[Dub]), [Dub]);
        // Mixed: both, the best first.
        let mixed = held(&ORDER, &[file(1, Some(Sub)), file(2, Some(Dub))], None);
        assert_eq!(mixed, [Dub, Sub]);
        assert_eq!(allowed(&ORDER, &mixed), [Dub, Sub]);
        // A language the settings do not name stays allowed once it is there.
        assert_eq!(allowed(&ORDER, &[English]), [Dub, Sub, EnglishSub, English]);
        // Nothing on disk: the last download's language, else the settings.
        assert_eq!(held(&ORDER, &[file(1, None)], Some(Sub)), [Sub]);
        assert_eq!(allowed(&ORDER, &[]), ORDER);
    }

    #[test]
    fn missing_episodes_come_in_the_best_allowed_language_or_wait() {
        let both = [Dub, Sub];
        assert_eq!(
            decide(&listed(5, &[Sub, Dub]), &[], &[], &ORDER, &both),
            Decision::New(Dub)
        );
        // A mixed series takes the sub now; the dub replaces it later.
        assert_eq!(
            decide(&listed(5, &[Sub]), &[], &[], &ORDER, &both),
            Decision::New(Sub)
        );
        // A dub series waits.
        assert_eq!(
            decide(&listed(5, &[Sub]), &[], &[], &ORDER, &[Dub]),
            Decision::Waiting(vec![Sub])
        );
        // Whatever the queue remembers, failed or done and deleted: missing is missing.
        let failed = job(5, JobState::Failed);
        let done = job(5, JobState::Done);
        for old in [&failed, &done] {
            assert_eq!(
                decide(&listed(5, &[Dub]), &[], &[old], &ORDER, &[Dub]),
                Decision::New(Dub)
            );
        }
        let queued = job(5, JobState::Queued);
        assert_eq!(
            decide(&listed(5, &[Dub]), &[], &[&queued], &ORDER, &[Dub]),
            Decision::InQueue
        );
    }

    #[test]
    fn an_episode_on_disk_is_replaced_by_a_better_language() {
        let both = [Dub, Sub];
        let sub = [file(1, Some(Sub))];
        assert_eq!(
            decide(&listed(1, &[Sub]), &sub, &[], &ORDER, &both),
            Decision::OnDisk
        );
        assert_eq!(
            decide(&listed(1, &[Dub, Sub]), &sub, &[], &ORDER, &both),
            Decision::Upgrade(Dub, vec![sub[0].path.clone()])
        );
        let dub = [file(1, Some(Dub))];
        assert_eq!(
            decide(&listed(1, &[Dub, Sub]), &dub, &[], &ORDER, &both),
            Decision::OnDisk
        );
        // A name without a language is left as it is.
        assert_eq!(
            decide(&listed(1, &[Dub]), &[file(1, None)], &[], &ORDER, &both),
            Decision::OnDisk
        );
    }
}
