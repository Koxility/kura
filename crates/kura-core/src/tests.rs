use std::time::Duration;

use kura_domain::{AutoSync, JobState, Language, Library, Role};
use kura_hosters::HosterError;

use super::*;

/// One series with three episodes; every episode has the same links. A series whose seasons
/// are in the listing shows those instead, so a test can change what the site offers.
struct FakeSite(Vec<HosterLink>, Listing);

/// Episodes per `(slug, season)`, for series a test lists itself.
type Listing = Arc<Mutex<HashMap<(String, u32), Vec<Episode>>>>;

#[async_trait]
impl Site for FakeSite {
    fn id(&self) -> SiteId {
        SiteId::AniWorld
    }

    async fn search(&self, _: &str) -> kura_sites::Result<Vec<SeriesSummary>> {
        Ok(vec![self.series("lantern-road").await?.summary])
    }

    async fn popular(&self) -> kura_sites::Result<Vec<SeriesSummary>> {
        Ok(Vec::new())
    }

    async fn latest(&self) -> kura_sites::Result<Vec<SeriesSummary>> {
        Ok(Vec::new())
    }

    async fn genres(&self) -> kura_sites::Result<Vec<Genre>> {
        Ok(vec![Genre {
            slug: "abenteuer".to_owned(),
            name: "Abenteuer".to_owned(),
        }])
    }

    async fn by_genre(&self, _: &str, _: u32) -> kura_sites::Result<Vec<SeriesSummary>> {
        self.all().await
    }

    async fn all(&self) -> kura_sites::Result<Vec<SeriesSummary>> {
        Ok(vec![self.series("lantern-road").await?.summary])
    }

    async fn series(&self, slug: &str) -> kura_sites::Result<Series> {
        Ok(Series {
            summary: SeriesSummary {
                downloaded: false,
                site: SiteId::AniWorld,
                slug: slug.to_owned(),
                title: "Lantern: Road?".to_owned(),
                cover_url: None,
            },
            description: String::new(),
            seasons: {
                let listing = self.1.lock().unwrap();
                let mut seasons: Vec<u32> = listing
                    .keys()
                    .filter(|(listed, _)| listed == slug)
                    .map(|(_, season)| *season)
                    .collect();
                seasons.sort_unstable();
                if seasons.is_empty() { vec![1] } else { seasons }
            },
            // One fake series has an ID on the site, the other has none.
            imdb_id: (slug == "lantern-road").then(|| "tt0388629".to_owned()),
        })
    }

    async fn episodes(&self, slug: &str, season: u32) -> kura_sites::Result<Vec<Episode>> {
        use Language::{GermanDub, GermanSub};
        if let Some(listed) = self.1.lock().unwrap().get(&(slug.to_owned(), season)) {
            return Ok(listed.clone());
        }
        // `mixed` offers its episodes in different languages; every other series in dub.
        let languages = |number| match (slug, number) {
            ("mixed", 2) => vec![GermanDub, GermanSub],
            ("mixed", 3) => vec![GermanSub],
            _ => vec![GermanDub],
        };
        let count = if slug == "mixed" { 4 } else { 3 };
        Ok((1..=count)
            .map(|number| Episode {
                season,
                number,
                title: format!("Episode {number}"),
                languages: languages(number),
            })
            .collect())
    }

    async fn links(&self, _: &EpisodeId) -> kura_sites::Result<Vec<HosterLink>> {
        Ok(self.0.clone())
    }
}

struct FailingSite;

#[async_trait]
impl Site for FailingSite {
    fn id(&self) -> SiteId {
        SiteId::SerienStream
    }

    async fn search(&self, _: &str) -> kura_sites::Result<Vec<SeriesSummary>> {
        Err(SiteError::Layout("search results"))
    }

    async fn popular(&self) -> kura_sites::Result<Vec<SeriesSummary>> {
        Err(SiteError::Layout("list"))
    }

    async fn latest(&self) -> kura_sites::Result<Vec<SeriesSummary>> {
        Err(SiteError::Layout("list"))
    }

    async fn genres(&self) -> kura_sites::Result<Vec<Genre>> {
        Err(SiteError::Layout("genres"))
    }

    async fn by_genre(&self, _: &str, _: u32) -> kura_sites::Result<Vec<SeriesSummary>> {
        Err(SiteError::Layout("list"))
    }

    async fn all(&self) -> kura_sites::Result<Vec<SeriesSummary>> {
        Ok(Vec::new())
    }

    async fn series(&self, _: &str) -> kura_sites::Result<Series> {
        Err(SiteError::Layout("series title"))
    }

    async fn episodes(&self, _: &str, _: u32) -> kura_sites::Result<Vec<Episode>> {
        Err(SiteError::Layout("episodes"))
    }

    async fn links(&self, _: &EpisodeId) -> kura_sites::Result<Vec<HosterLink>> {
        Err(SiteError::Layout("links"))
    }
}

/// Resolves a link to itself, unless the link says the video is gone. A "flaky" link gets a
/// new stream address every time it is resolved: `#1`, `#2` and so on.
struct FakeHoster(&'static str, std::sync::atomic::AtomicU32);

#[async_trait]
impl Hoster for FakeHoster {
    fn name(&self) -> &'static str {
        self.0
    }

    async fn resolve(&self, url: &str) -> kura_hosters::Result<Stream> {
        if url.contains("gone") {
            return Err(HosterError::NoStream("fake"));
        }
        let mut url = url.to_owned();
        if url.contains("flaky") {
            let resolved = self.1.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            url = format!("{url}#{resolved}");
        }
        Ok(Stream {
            url,
            headers: Vec::new(),
        })
    }
}

/// Fails on "broken" streams and on the first address of a "flaky" one, never finishes
/// "slow" ones, and records the rest.
#[derive(Default)]
struct FakeDownload(Arc<Mutex<Vec<(String, PathBuf)>>>);

#[async_trait]
impl Download for FakeDownload {
    async fn download(
        &self,
        stream: &Stream,
        target: &Path,
        _: OutputFormat,
        on_progress: &mut (dyn FnMut(Progress) + Send),
    ) -> std::result::Result<(), DownloadError> {
        on_progress(Progress {
            fraction: 0.5,
            bytes: 100,
        });
        if stream.url.contains("broken") {
            return Err(DownloadError::Ffmpeg("stream ended early".to_owned()));
        }
        if stream.url.ends_with("flaky#1") {
            return Err(DownloadError::Ffmpeg(
                "Server returned 404 Not Found".to_owned(),
            ));
        }
        if stream.url.contains("slow") {
            std::future::pending::<()>().await;
        }
        // A real, empty file, so folder lookups see what was downloaded.
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, b"").unwrap();
        let done = (stream.url.clone(), target.to_owned());
        self.0.lock().unwrap().push(done);
        Ok(())
    }
}

fn link(language: Language, hoster: &str, url: &str) -> HosterLink {
    HosterLink {
        language,
        hoster: hoster.to_owned(),
        url: url.to_owned(),
    }
}

struct Setup {
    core: Arc<Core>,
    downloads: Arc<Mutex<Vec<(String, PathBuf)>>>,
    /// What the fake site lists for series a test sets up itself.
    listing: Listing,
    user: i64,
    /// Holds the library folders; removed when the test ends.
    dir: tempfile::TempDir,
}

impl Setup {
    /// A folder below the test's own temporary folder.
    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }
}

fn library(name: &str, path: PathBuf, default_for: Vec<SiteId>) -> Library {
    Library {
        name: name.to_owned(),
        path,
        default_for,
        jellyfin_path: None,
    }
}

/// A running core with one site, the hosters `voe` and `vidmoly`, and settings that prefer
/// German dub over German sub and `voe` over `vidmoly`.
fn setup(links: Vec<HosterLink>, parallel: u8) -> Setup {
    let store = Store::open_in_memory().unwrap();
    let user = store.create_user("admin", "hash", Role::Admin).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let settings = Settings {
        libraries: vec![library("Main", dir.path().join("main"), Vec::new())],
        parallel,
        languages: vec![Language::GermanDub, Language::GermanSub],
        hosters: vec!["voe".into(), "unsupported".into(), "vidmoly".into()],
        ..Settings::default()
    };
    store.save_settings(&settings).unwrap();

    let downloader = FakeDownload::default();
    let downloads = downloader.0.clone();
    let listing = Listing::default();
    let core = Core::new(
        store,
        vec![
            Arc::new(FakeSite(links, listing.clone())),
            Arc::new(FailingSite),
        ],
        vec![
            Box::new(FakeHoster("voe", Default::default())),
            Box::new(FakeHoster("vidmoly", Default::default())),
        ],
        Box::new(downloader),
    );
    tokio::spawn(core.clone().run());
    Setup {
        core,
        downloads,
        listing,
        user: user.unwrap().id,
        dir,
    }
}

/// Waits until the jobs, in queue order, are in exactly these states.
async fn states(core: &Core, expected: &[JobState]) -> Vec<Job> {
    for _ in 0..200 {
        let jobs = core.jobs().unwrap();
        if jobs
            .iter()
            .map(|job| job.state)
            .eq(expected.iter().copied())
        {
            return jobs;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("expected {expected:?}, got {:#?}", core.jobs().unwrap());
}

/// Season 1, episode 1.
fn first() -> Selection {
    Selection::Picked {
        seasons: Vec::new(),
        episodes: vec![(1, 1)],
    }
}

impl Setup {
    /// Queues from the one series of the fake site, as the test user.
    async fn queue(&self, selection: Selection, choices: Choices) -> Result<usize> {
        let queued = self.core.enqueue(
            SiteId::AniWorld,
            "lantern-road",
            selection,
            &choices,
            self.user,
        );
        queued.await
    }
}

#[tokio::test]
async fn falls_back_to_the_next_hoster_and_then_the_next_language() {
    let s = setup(
        vec![
            link(Language::EnglishSub, "voe", "https://voe/unwanted-language"),
            link(Language::GermanSub, "voe", "https://voe/sub"),
            link(Language::GermanDub, "unsupported", "https://other/dub"),
            link(Language::GermanDub, "vidmoly", "https://vidmoly/broken"),
            link(Language::GermanDub, "voe", "https://voe/gone"),
        ],
        1,
    );
    let mut events = s.core.subscribe();

    assert_eq!(s.queue(first(), Choices::default()).await.unwrap(), 1);
    let jobs = states(&s.core, &[JobState::Done]).await;

    let target = s
        .path("main/Lantern Road [imdbid-tt0388629]/Season 01/Lantern Road S01E001 German Sub.mkv");
    assert_eq!(jobs[0].path.as_deref(), target.to_str());
    assert_eq!(
        *s.downloads.lock().unwrap(),
        [("https://voe/sub".to_owned(), target)]
    );

    let mut seen = Vec::new();
    while let Ok(event) = events.try_recv() {
        seen.push(event);
    }
    assert!(seen.contains(&Event::QueueChanged));
    let progress = Event::Progress {
        id: jobs[0].id,
        fraction: 0.5,
        bytes: 100,
    };
    assert!(seen.contains(&progress));
}

#[tokio::test]
async fn a_picked_language_and_hoster_come_before_the_settings() {
    let s = setup(
        vec![
            link(Language::GermanDub, "voe", "https://voe/dub"),
            link(Language::EnglishSub, "voe", "https://voe/english"),
            link(Language::EnglishSub, "vidmoly", "https://vidmoly/english"),
        ],
        1,
    );
    let picked = Choices {
        language: Some(Language::EnglishSub),
        hoster: Some("vidmoly".to_owned()),
        ..Choices::default()
    };
    s.queue(first(), picked).await.unwrap();
    let jobs = states(&s.core, &[JobState::Done]).await;
    assert_eq!(s.downloads.lock().unwrap()[0].0, "https://vidmoly/english");
    assert_eq!(jobs[0].language, Some(Language::EnglishSub));

    let unknown = Choices {
        hoster: Some("unsupported".to_owned()),
        ..Choices::default()
    };
    let refused = s.queue(first(), unknown).await;
    assert!(matches!(refused, Err(CoreError::Invalid(_))));
}

#[tokio::test]
async fn a_failed_download_lists_every_attempt_and_can_be_retried() {
    let s = setup(
        vec![
            link(Language::GermanDub, "voe", "https://voe/gone"),
            link(Language::GermanDub, "vidmoly", "https://vidmoly/broken"),
        ],
        1,
    );
    s.queue(first(), Choices::default()).await.unwrap();
    let jobs = states(&s.core, &[JobState::Failed]).await;
    assert_eq!(
        jobs[0].error.as_deref(),
        Some(
            "voe, german_dub: no stream found on the fake page; \
             vidmoly, german_dub: ffmpeg failed: stream ended early (tried 3 times)"
        )
    );

    assert!(s.core.retry(jobs[0].id, s.user).unwrap());
    states(&s.core, &[JobState::Failed]).await;
    assert!(matches!(
        s.core.retry(999, s.user),
        Err(CoreError::UnknownJob)
    ));

    // Clearing takes it off the queue page but keeps the record.
    assert_eq!(s.core.clear_finished().unwrap(), 1);
    assert!(s.core.jobs().unwrap()[0].hidden);
}

#[tokio::test]
async fn a_stream_that_fails_is_resolved_again_before_the_next_hoster_is_tried() {
    let s = setup(
        vec![
            link(Language::GermanDub, "voe", "https://voe/flaky"),
            link(Language::GermanDub, "vidmoly", "https://vidmoly/dub"),
        ],
        1,
    );
    let mut events = s.core.subscribe();
    s.queue(first(), Choices::default()).await.unwrap();
    let jobs = states(&s.core, &[JobState::Done]).await;

    // The second address VOE handed out worked, so Vidmoly was never asked.
    assert_eq!(s.downloads.lock().unwrap()[0].0, "https://voe/flaky#2");

    // The failed try reported 50%; the bar went back to nothing before the next try, and
    // the queue was told so it can show why the download starts again.
    let mut seen = Vec::new();
    while let Ok(event) = events.try_recv() {
        match event {
            Event::Progress { id, fraction, .. } => {
                assert_eq!(id, jobs[0].id);
                seen.push(format!("{fraction}"));
            }
            Event::QueueChanged => seen.push("changed".to_owned()),
            Event::RequestsChanged => {}
        }
    }
    seen.dedup();
    assert_eq!(seen, ["changed", "0.5", "0", "changed", "0.5", "changed"]);
    // The note of the failed try is gone once the download worked.
    assert_eq!(jobs[0].error, None);
}

#[tokio::test]
async fn a_download_that_starts_over_says_why() {
    let s = setup(
        vec![link(Language::GermanDub, "voe", "https://voe/broken")],
        1,
    );
    s.queue(first(), Choices::default()).await.unwrap();
    let jobs = states(&s.core, &[JobState::Failed]).await;
    // Every try failed; the final error lists them, not the note of a retry.
    let error = jobs[0].error.as_deref().unwrap();
    assert!(error.contains("tried 3 times"), "{error}");
    assert!(!error.contains("trying again"), "{error}");
}

#[tokio::test]
async fn an_episode_without_usable_links_fails_with_a_reason() {
    let s = setup(vec![link(Language::EnglishSub, "voe", "https://voe/x")], 1);
    s.queue(first(), Choices::default()).await.unwrap();
    let jobs = states(&s.core, &[JobState::Failed]).await;
    assert_eq!(
        jobs[0].error.as_deref(),
        Some("no link in a wanted language from a supported hoster")
    );
}

#[tokio::test]
async fn runs_as_many_downloads_as_the_settings_allow() {
    use JobState::{Cancelled, Queued, Running};
    let s = setup(
        vec![link(Language::GermanDub, "voe", "https://voe/slow")],
        2,
    );
    // Episode 2 is picked twice, alone and with its season.
    let season = Selection::Picked {
        seasons: vec![1],
        episodes: vec![(1, 2)],
    };
    assert_eq!(s.queue(season, Choices::default()).await.unwrap(), 3);
    let again = s.queue(Selection::Series, Choices::default());
    assert_eq!(again.await.unwrap(), 0);
    let nothing = Selection::Picked {
        seasons: Vec::new(),
        episodes: Vec::new(),
    };
    let refused = s.queue(nothing, Choices::default()).await;
    assert!(matches!(refused, Err(CoreError::Invalid(_))));

    let jobs = states(&s.core, &[Running, Running, Queued]).await;
    assert_eq!(jobs[0].progress, 0.5);

    // Cancelling frees a slot for the waiting download.
    assert!(s.core.cancel(jobs[0].id).unwrap());
    states(&s.core, &[Cancelled, Running, Running]).await;
    assert!(!s.core.cancel(jobs[0].id).unwrap());

    // Raising the limit applies without a restart.
    assert!(s.core.retry(jobs[0].id, s.user).unwrap());
    states(&s.core, &[Queued, Running, Running]).await;
    let settings = Settings {
        parallel: 3,
        ..s.core.settings().unwrap()
    };
    s.core.save_settings(&settings).unwrap();
    states(&s.core, &[Running, Running, Running]).await;
}

#[tokio::test]
async fn downloads_go_to_the_picked_then_the_default_then_the_main_library() {
    let s = setup(vec![link(Language::GermanDub, "voe", "https://voe/x")], 1);
    let mut settings = s.core.settings().unwrap();
    settings.libraries.extend([
        library("Anime", s.path("anime"), vec![SiteId::AniWorld]),
        library("Other", s.path("other"), Vec::new()),
    ]);
    s.core.save_settings(&settings).unwrap();
    // Saving creates the folders.
    assert!(s.path("other").is_dir());

    let episode = |number| Selection::Picked {
        seasons: Vec::new(),
        episodes: vec![(1, number)],
    };
    let picked = |name: &str| Choices {
        library: Some(name.to_owned()),
        ..Choices::default()
    };
    s.queue(episode(1), picked("Other")).await.unwrap();
    s.queue(episode(2), Choices::default()).await.unwrap();
    let refused = s.queue(episode(3), picked("Gone")).await;
    assert!(matches!(refused, Err(CoreError::Invalid(_))));

    // Without a default for the site, the main library is used.
    settings.libraries[1].default_for.clear();
    s.core.save_settings(&settings).unwrap();
    s.queue(episode(3), Choices::default()).await.unwrap();

    let jobs = states(&s.core, &[JobState::Done; 3]).await;
    let dirs: Vec<_> = jobs.iter().map(|job| PathBuf::from(&job.dir)).collect();
    assert_eq!(dirs, [s.path("other"), s.path("anime"), s.path("main")]);
    let file = PathBuf::from(jobs[0].path.as_deref().unwrap());
    assert!(file.starts_with(s.path("other/Lantern Road [imdbid-tt0388629]")));
}

#[tokio::test]
async fn nothing_is_queued_before_a_library_exists() {
    let s = setup(vec![link(Language::GermanDub, "voe", "https://voe/x")], 1);
    let mut settings = s.core.settings().unwrap();
    settings.libraries.clear();
    let refused = s.core.save_settings(&settings);
    assert!(matches!(refused, Err(CoreError::Invalid(_))));

    // A store without libraries, as on a fresh installation.
    s.core.store.save_settings(&settings).unwrap();
    let queued = s.queue(first(), Choices::default()).await;
    assert!(matches!(queued, Err(CoreError::Invalid(_))));

    // A folder that cannot be created is refused when saving.
    let blocked = s.path("file");
    std::fs::write(&blocked, b"").unwrap();
    settings.libraries = vec![library("Main", blocked.join("below-a-file"), Vec::new())];
    let refused = s.core.save_settings(&settings);
    assert!(matches!(refused, Err(CoreError::Folder(..))), "{refused:?}");
}

#[tokio::test]
async fn a_disabled_site_is_left_out() {
    let s = setup(Vec::new(), 1);
    let settings = Settings {
        disabled_sites: vec![SiteId::AniWorld],
        ..s.core.settings().unwrap()
    };
    s.core.save_settings(&settings).unwrap();

    // The only site left is the failing one.
    assert!(s.core.search("lantern", None).await.is_err());
    let series = s.core.series(SiteId::AniWorld, "lantern-road").await;
    assert!(matches!(series, Err(CoreError::SiteDisabled)));
    let queued = s.queue(first(), Choices::default()).await;
    assert!(matches!(queued, Err(CoreError::SiteDisabled)));
}

#[tokio::test]
async fn random_picks_from_the_full_list() {
    let s = setup(Vec::new(), 1);
    let pick = s.core.random(SiteId::AniWorld).await.unwrap();
    assert_eq!(pick.slug, "lantern-road");
    assert_eq!(s.core.genres(SiteId::AniWorld).await.unwrap().len(), 1);
    assert_eq!(
        s.core
            .by_genre(SiteId::AniWorld, "abenteuer", 1)
            .await
            .unwrap(),
        [pick]
    );

    // A site without any series has nothing to pick from.
    let empty = s.core.random(SiteId::SerienStream).await;
    assert!(matches!(empty, Err(CoreError::Site(SiteError::Layout(_)))));
}

#[tokio::test]
async fn search_survives_one_failing_site() {
    let s = setup(Vec::new(), 1);
    let found = s.core.search(" lantern ", None).await.unwrap();
    assert_eq!(found.len(), 1);
    assert!(s.core.search("  ", None).await.unwrap().is_empty());
    assert!(s.core.popular(SiteId::SerienStream).await.is_err());

    // Asking one site only: its failure is the answer.
    let one = s.core.search("lantern", Some(SiteId::AniWorld)).await;
    assert_eq!(one.unwrap().len(), 1);
    assert!(
        s.core
            .search("lantern", Some(SiteId::SerienStream))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn titles_can_be_excluded_from_auto_sync_and_included_again() {
    let s = setup(Vec::new(), 1);
    let title = TitleRef {
        site: SiteId::AniWorld,
        slug: "lantern-road".to_owned(),
        title: "Lantern Road".to_owned(),
    };
    s.core.set_excluded(title.clone(), true).unwrap();
    s.core.set_excluded(title.clone(), true).unwrap();
    let excluded = s.core.settings().unwrap().auto_sync.excluded;
    assert_eq!(excluded, std::slice::from_ref(&title));
    s.core.set_excluded(title, false).unwrap();
    assert!(s.core.settings().unwrap().auto_sync.excluded.is_empty());
}

#[test]
fn unusable_settings_are_refused() {
    let library = |name: &str, path: &str, default_for| Library {
        name: name.to_owned(),
        path: PathBuf::from(path),
        default_for,
        jellyfin_path: None,
    };
    let schedule = |schedule| AutoSync {
        schedule,
        ..AutoSync::default()
    };
    let good = Settings {
        libraries: vec![
            library("Anime", "/mnt/anime", vec![SiteId::AniWorld]),
            library("Other", "/mnt/other", vec![SiteId::SerienStream]),
        ],
        auto_sync: schedule(Schedule::Daily { at: "03:30".into() }),
        ..Settings::default()
    };
    assert!(check_settings(&good).is_ok());

    let bad = [
        Settings {
            parallel: 0,
            ..good.clone()
        },
        Settings {
            parallel: 17,
            ..good.clone()
        },
        Settings {
            libraries: Vec::new(),
            ..good.clone()
        },
        Settings {
            languages: Vec::new(),
            ..good.clone()
        },
        Settings {
            hosters: Vec::new(),
            ..good.clone()
        },
        Settings {
            libraries: vec![library(" ", "/mnt/a", Vec::new())],
            ..good.clone()
        },
        Settings {
            libraries: vec![library("A", "relative/path", Vec::new())],
            ..good.clone()
        },
        Settings {
            libraries: vec![
                library("A", "/a", Vec::new()),
                library("a", "/b", Vec::new()),
            ],
            ..good.clone()
        },
        Settings {
            libraries: vec![
                library("A", "/a", vec![SiteId::AniWorld]),
                library("B", "/b", vec![SiteId::AniWorld]),
            ],
            ..good.clone()
        },
        Settings {
            auto_sync: schedule(Schedule::Every { minutes: 1 }),
            ..good.clone()
        },
        Settings {
            auto_sync: schedule(Schedule::Daily { at: "24:00".into() }),
            ..good.clone()
        },
        Settings {
            auto_sync: schedule(Schedule::Daily { at: "soon".into() }),
            ..good.clone()
        },
    ];
    for (index, settings) in bad.iter().enumerate() {
        let refused = check_settings(settings);
        assert!(
            matches!(refused, Err(CoreError::Invalid(_))),
            "case {index} was accepted"
        );
    }
}

#[test]
fn titles_become_safe_folder_names() {
    assert_eq!(
        safe_name("Re:Zero / Part 2?").as_deref(),
        Some("Re Zero Part 2")
    );
    assert_eq!(
        safe_name("  ...hack//Sign.  ").as_deref(),
        Some("hack Sign")
    );
    assert_eq!(safe_name("../.."), None);
    assert_eq!(safe_name("\u{0}\n"), None);

    let job = |imdb_id: Option<&str>| Job {
        id: 1,
        episode: EpisodeId {
            site: SiteId::AniWorld,
            slug: "some-slug".to_owned(),
            season: 0,
            number: 12,
        },
        series_title: "../..".to_owned(),
        state: JobState::Running,
        progress: 0.0,
        error: None,
        path: None,
        language: None,
        hoster: None,
        dir: "dl".to_owned(),
        hidden: false,
        queued_at: 0,
        imdb_id: imdb_id.map(str::to_owned),
        started_at: None,
        replaces: Vec::new(),
    };
    let path = |imdb| {
        let template = "{title} {episodenumber}";
        target_path(
            Path::new("dl"),
            template,
            &job(imdb),
            Language::GermanDub,
            OutputFormat::Mp4,
            None,
        )
    };
    let file = "Season 00/some-slug S00E012.mp4";
    let id = Path::new("dl/some-slug [imdbid-tt0388629]").join(file);
    assert_eq!(path(Some("tt0388629")), id);
    let none = Path::new("dl/some-slug [imdbid-none]").join(file);
    assert_eq!(path(Some("none")), none);
    // Without an ID the folder is the title alone.
    assert_eq!(path(None), Path::new("dl/some-slug").join(file));
    // A series that already has a folder keeps it, whatever the layout.
    let old = Path::new("dl/Series/some-slug");
    let kept = target_path(
        Path::new("dl"),
        "{title} {episodenumber}",
        &job(Some("tt0388629")),
        Language::GermanDub,
        OutputFormat::Mp4,
        Some(old),
    );
    assert_eq!(kept, old.join(file));
    // A broken stored value never reaches the path.
    assert_eq!(path(Some("../x")), Path::new("dl/some-slug").join(file));
}

#[tokio::test]
async fn without_an_id_on_the_site_the_answer_or_none_names_the_folder() {
    let s = setup(vec![link(Language::GermanDub, "voe", "https://voe/x")], 1);
    let queue = |slug: &'static str, imdb_id: Option<&str>| {
        let choices = Choices {
            imdb_id: imdb_id.map(str::to_owned),
            ..Choices::default()
        };
        let core = s.core.clone();
        let user = s.user;
        async move {
            core.enqueue(SiteId::AniWorld, slug, first(), &choices, user)
                .await
        }
    };
    queue("no-id-one", Some("tt1234567")).await.unwrap();
    queue("no-id-two", None).await.unwrap();
    // The site's own ID wins over an answer.
    queue("lantern-road", Some("tt7654321")).await.unwrap();
    let bad = queue("no-id-three", Some("123")).await;
    assert!(matches!(bad, Err(CoreError::Invalid(_))));

    let jobs = states(&s.core, &[JobState::Done; 3]).await;
    let ids: Vec<_> = jobs.iter().map(|job| job.imdb_id.as_deref()).collect();
    assert_eq!(ids, [Some("tt1234567"), Some("none"), Some("tt0388629")]);
    let folder = PathBuf::from(jobs[1].path.as_deref().unwrap());
    assert!(
        folder
            .to_string_lossy()
            .contains("[imdbid-none]/Season 01/")
    );
}

#[tokio::test]
async fn accepted_requests_are_queued_with_their_choices_and_decided_once() {
    let s = setup(vec![link(Language::GermanSub, "voe", "https://voe/x")], 1);
    let sam = s
        .core
        .create_user("sam", "battery staple", Role::User)
        .await;
    let sam = sam.unwrap().unwrap();
    let picked = kura_domain::Picked {
        all: false,
        seasons: Vec::new(),
        episodes: vec![(1, 2), (1, 3)],
    };
    let choices = Choices {
        language: Some(Language::GermanSub),
        ..Choices::default()
    };
    let ask = |picked: kura_domain::Picked| {
        let (core, choices) = (s.core.clone(), choices.clone());
        async move {
            core.request(SiteId::AniWorld, "lantern-road", &picked, &choices, sam.id)
                .await
        }
    };
    let first_request = ask(picked.clone()).await.unwrap();
    let second_request = ask(picked.clone()).await.unwrap();
    let nothing = ask(kura_domain::Picked::default()).await;
    assert!(matches!(nothing, Err(CoreError::Invalid(_))));
    // Asking queues nothing yet.
    assert!(s.core.jobs().unwrap().is_empty());
    assert_eq!(s.core.requests(Some(sam.id)).unwrap().len(), 2);
    assert!(s.core.requests(Some(s.user)).unwrap().is_empty());

    assert_eq!(
        s.core.accept_request(first_request, s.user).await.unwrap(),
        2
    );
    let jobs = states(&s.core, &[JobState::Done; 2]).await;
    assert_eq!(jobs[0].language, Some(Language::GermanSub));
    let again = s.core.accept_request(first_request, s.user).await;
    assert!(matches!(again, Err(CoreError::Invalid(_))));

    s.core.reject_request(second_request).unwrap();
    assert!(matches!(
        s.core.reject_request(second_request),
        Err(CoreError::Invalid(_))
    ));
    assert_eq!(s.core.jobs().unwrap().len(), 2);
    let states: Vec<_> = s
        .core
        .requests(None)
        .unwrap()
        .iter()
        .map(|r| r.state)
        .collect();
    assert_eq!(
        states,
        // Newest first once both are decided.
        [
            kura_domain::RequestState::Rejected,
            kura_domain::RequestState::Accepted
        ]
    );
}

#[tokio::test]
async fn auto_sync_queues_what_is_new_and_fills_gaps_only_when_asked() {
    let s = setup(vec![link(Language::GermanDub, "voe", "https://voe/x")], 1);
    let second = Selection::Picked {
        seasons: Vec::new(),
        episodes: vec![(1, 2)],
    };
    let picked = Choices {
        language: Some(Language::GermanDub),
        ..Choices::default()
    };
    s.queue(second, picked).await.unwrap();
    states(&s.core, &[JobState::Done]).await;

    // Episode 2 is downloaded: only episode 3 is new, episode 1 is a gap.
    assert_eq!(s.core.sync_once().await.unwrap(), 1);
    let jobs = states(&s.core, &[JobState::Done; 2]).await;
    assert_eq!(jobs[1].episode.number, 3);
    assert_eq!(jobs[1].language, Some(Language::GermanDub));
    assert_eq!(s.core.sync_once().await.unwrap(), 0);

    let status = s.core.sync_status().unwrap();
    assert_eq!(status.titles.len(), 1);
    assert_eq!(status.titles[0].on_disk, 2);
    assert_eq!(
        status.titles[0]
            .check
            .as_ref()
            .map(|check| check.summary.as_str()),
        Some("Nothing new")
    );

    let mut settings = s.core.settings().unwrap();
    settings.auto_sync.only_new = false;
    s.core.save_settings(&settings).unwrap();
    assert_eq!(s.core.sync_once().await.unwrap(), 1);
    let jobs = states(&s.core, &[JobState::Done; 3]).await;
    assert_eq!(jobs[2].episode.number, 1);

    // An excluded title is no longer tracked.
    let title = TitleRef {
        site: SiteId::AniWorld,
        slug: "lantern-road".to_owned(),
        title: "Lantern Road".to_owned(),
    };
    s.core.set_excluded(title, true).unwrap();
    assert!(s.core.sync_status().unwrap().titles.is_empty());
}

#[tokio::test]
async fn auto_sync_keeps_the_downloaded_language_and_skips_files_in_the_library() {
    let links = vec![
        link(Language::GermanSub, "voe", "https://voe/sub"),
        link(Language::GermanDub, "voe", "https://voe/dub"),
    ];
    let s = setup(links, 1);
    let first_episode = s
        .core
        .enqueue(
            SiteId::AniWorld,
            "mixed",
            first(),
            &Choices::default(),
            s.user,
        )
        .await;
    first_episode.unwrap();
    // Without a picked language the settings chose German dub, and the job says so.
    let jobs = states(&s.core, &[JobState::Done]).await;
    assert_eq!(jobs[0].language, Some(Language::GermanDub));
    // Episode 4 was put into the series folder by hand.
    let season = PathBuf::from(jobs[0].path.as_deref().unwrap());
    let season = season.parent().unwrap();
    std::fs::write(season.join("Mixed S01E004 German Dub.mkv"), b"").unwrap();
    s.core.forget_scan();

    // Sub first now: Auto-Sync must still stay with the dub.
    let mut settings = s.core.settings().unwrap();
    settings.languages = vec![Language::GermanSub, Language::GermanDub];
    settings.auto_sync.only_new = false;
    s.core.save_settings(&settings).unwrap();

    // Episode 2 has a dub, episode 3 only a sub, episode 4 is in the library.
    assert_eq!(s.core.sync_once().await.unwrap(), 1);
    let jobs = states(&s.core, &[JobState::Done; 2]).await;
    assert_eq!(jobs[1].episode.number, 2);
    assert_eq!(jobs[1].language, Some(Language::GermanDub));
    assert!(jobs[1].path.as_deref().unwrap().ends_with("German Dub.mkv"));
    assert_eq!(s.core.sync_once().await.unwrap(), 0);
}

#[tokio::test]
async fn the_library_view_stays_inside_its_folders() {
    let s = setup(Vec::new(), 1);
    let dir = tempfile::tempdir().unwrap();
    let season = dir.path().join("Series/Show/Season 01");
    std::fs::create_dir_all(&season).unwrap();
    std::fs::write(season.join("Show - S01E01.mkv"), b"12345").unwrap();
    std::fs::write(season.join("Show - S01E02.mkv.part"), b"1").unwrap();
    std::fs::write(season.join(".hidden"), b"1").unwrap();
    std::fs::write(season.join("Show - S01E02.MP4"), b"123").unwrap();
    for other in [
        "poster.jpg",
        "Show - S01E01.nfo",
        "Show - S01E01.de.srt",
        "notes.txt",
    ] {
        std::fs::write(season.join(other), b"1").unwrap();
    }
    std::fs::create_dir_all(season.join("extras")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("/", dir.path().join("escape")).unwrap();

    let settings = Settings {
        libraries: vec![
            library("Downloads", dir.path().to_owned(), Vec::new()),
            library("Anime", dir.path().join("anime-not-filled-yet"), Vec::new()),
        ],
        ..s.core.settings().unwrap()
    };
    s.core.save_settings(&settings).unwrap();

    let roots = s.core.library_roots().unwrap();
    let names: Vec<_> = roots.iter().map(|root| root.name.as_str()).collect();
    assert_eq!(names, ["Downloads", "Anime"]);

    let top = s.core.browse(0, "").unwrap();
    assert!(
        top.iter()
            .any(|entry| entry.name == "Series" && entry.folder)
    );
    let files = s.core.browse(0, "Series/Show/Season 01").unwrap();
    let entry = |name: &str, folder, size| Entry {
        name: name.to_owned(),
        folder,
        size,
    };
    // Folders and video files only: no posters, subtitles or metadata.
    assert_eq!(
        files,
        [
            entry("extras", true, 0),
            entry("Show - S01E01.mkv", false, 5),
            entry("Show - S01E02.MP4", false, 3),
        ]
    );

    // Not there yet is empty, not an error.
    assert!(s.core.browse(0, "Series/Other").unwrap().is_empty());
    assert!(s.core.browse(1, "").unwrap().is_empty());

    for outside in ["..", "Series/../..", "/etc"] {
        let refused = s.core.browse(0, outside);
        assert!(
            matches!(refused, Err(CoreError::Invalid(_))),
            "{outside} was opened"
        );
    }
    #[cfg(unix)]
    assert!(matches!(
        s.core.browse(0, "escape"),
        Err(CoreError::Invalid(_))
    ));
    assert!(matches!(s.core.browse(7, ""), Err(CoreError::Invalid(_))));
}

#[tokio::test]
async fn a_custom_naming_template_names_the_file() {
    let s = setup(vec![link(Language::GermanDub, "voe", "https://voe/x")], 1);
    let settings = Settings {
        naming_template: Some("{episodenumber} - {title} [{language}]".to_owned()),
        ..s.core.settings().unwrap()
    };
    s.core.save_settings(&settings).unwrap();
    let broken = Settings {
        naming_template: Some("{title}".to_owned()),
        ..settings
    };
    assert!(matches!(
        s.core.save_settings(&broken),
        Err(CoreError::Invalid(_))
    ));

    s.queue(first(), Choices::default()).await.unwrap();
    let jobs = states(&s.core, &[JobState::Done]).await;
    let file = PathBuf::from(jobs[0].path.as_deref().unwrap());
    let name = file.file_name().unwrap().to_string_lossy().into_owned();
    assert_eq!(name, "S01E001 - Lantern Road [German Dub].mkv");
}

#[tokio::test]
async fn new_episodes_join_the_folder_of_earlier_ones() {
    let s = setup(vec![link(Language::GermanDub, "voe", "https://voe/x")], 1);
    s.queue(first(), Choices::default()).await.unwrap();
    let jobs = states(&s.core, &[JobState::Done]).await;
    let folder = s.path("main/Lantern Road [imdbid-tt0388629]");
    assert!(PathBuf::from(jobs[0].path.as_deref().unwrap()).starts_with(&folder));

    // Another library becomes the default; the series still stays where it started.
    let mut settings = s.core.settings().unwrap();
    settings
        .libraries
        .push(library("Anime", s.path("anime"), vec![SiteId::AniWorld]));
    s.core.save_settings(&settings).unwrap();
    let second = Selection::Picked {
        seasons: Vec::new(),
        episodes: vec![(1, 2)],
    };
    s.queue(second, Choices::default()).await.unwrap();
    let jobs = states(&s.core, &[JobState::Done; 2]).await;
    let file = PathBuf::from(jobs[1].path.as_deref().unwrap());
    assert!(file.starts_with(&folder), "{file:?}");
}

#[tokio::test]
async fn downloaded_titles_and_episodes_are_found_by_database_id_and_name() {
    let s = setup(vec![link(Language::GermanDub, "voe", "https://voe/x")], 1);
    // A series Kura downloaded.
    s.queue(first(), Choices::default()).await.unwrap();
    states(&s.core, &[JobState::Done]).await;
    // Folders put into the library by hand: one with an IMDb ID, one by name only.
    let by_id = s.path("main/Whatever Name [imdbid-tt7654321]/Season 02");
    std::fs::create_dir_all(&by_id).unwrap();
    std::fs::write(by_id.join("x S02E005.mkv"), b"").unwrap();
    std::fs::write(by_id.join("x S02E006.mkv.part"), b"").unwrap();
    let by_name = s.path("main/Series/Glass County/Season 01");
    std::fs::create_dir_all(&by_name).unwrap();
    std::fs::write(by_name.join("Glass County - S01E03.mp4"), b"").unwrap();
    std::fs::write(by_name.join("poster.jpg"), b"").unwrap();
    s.core.forget_scan();

    let episodes = |slug, title, imdb| {
        s.core
            .downloaded_episodes(SiteId::AniWorld, slug, title, imdb)
            .unwrap()
    };
    assert_eq!(
        episodes("lantern-road", "Lantern Road", Some("tt0388629")),
        [(1, 1)]
    );
    assert_eq!(episodes("other", "Any Title", Some("tt7654321")), [(2, 5)]);
    assert_eq!(episodes("glass-county", "Glass County!", None), [(1, 3)]);
    assert!(episodes("nothing", "Nothing Here", Some("none")).is_empty());

    let summary = |slug: &str, title: &str| SeriesSummary {
        downloaded: false,
        site: SiteId::AniWorld,
        slug: slug.to_owned(),
        title: title.to_owned(),
        cover_url: None,
    };
    let mut list = vec![
        summary("lantern-road", "Renamed On The Site"),
        summary("glass-county", "glass county"),
        summary("nothing", "Nothing Here"),
    ];
    s.core.mark_downloaded(&mut list).unwrap();
    let marks: Vec<_> = list.iter().map(|series| series.downloaded).collect();
    assert_eq!(marks, [true, true, false]);
}

#[tokio::test]
async fn deleted_files_are_no_longer_downloaded() {
    let s = setup(vec![link(Language::GermanDub, "voe", "https://voe/x")], 1);
    // Only the episode the test is about, so Auto-Sync finds nothing new after it.
    s.list("lantern-road", 1, &[(1, &[DUB])]);
    s.queue(first(), Choices::default()).await.unwrap();
    let jobs = states(&s.core, &[JobState::Done]).await;
    // A series only found by its IMDb ID on its page.
    let by_id = s.path("main/Whatever Name [imdbid-tt7654321]/Season 01");
    std::fs::create_dir_all(&by_id).unwrap();
    std::fs::write(by_id.join("x S01E002.mkv"), b"").unwrap();

    let episodes = |slug, title, imdb| {
        s.core
            .downloaded_episodes(SiteId::AniWorld, slug, title, imdb)
            .unwrap()
    };
    let summary = |slug: &str, title: &str| SeriesSummary {
        downloaded: false,
        site: SiteId::AniWorld,
        slug: slug.to_owned(),
        title: title.to_owned(),
        cover_url: None,
    };
    let marks = || {
        let mut list = vec![
            summary("lantern-road", "Renamed On The Site"),
            summary("other", "Any Title"),
        ];
        s.core.mark_downloaded(&mut list).unwrap();
        list.iter()
            .map(|series| series.downloaded)
            .collect::<Vec<_>>()
    };
    s.core.forget_scan();
    assert_eq!(episodes("other", "Any Title", Some("tt7654321")), [(1, 2)]);
    assert_eq!(marks(), [true, true]);

    // Both files are deleted; the folders stay.
    std::fs::remove_file(jobs[0].path.as_deref().unwrap()).unwrap();
    std::fs::remove_file(by_id.join("x S01E002.mkv")).unwrap();
    s.core.refresh_downloaded().unwrap();
    assert_eq!(marks(), [false, false]);
    assert!(episodes("lantern-road", "Lantern Road", Some("tt0388629")).is_empty());
    assert!(episodes("other", "Any Title", Some("tt7654321")).is_empty());

    // Auto-Sync does not download a deleted episode again.
    assert_eq!(s.core.sync_once().await.unwrap(), 0);
}

/// One user's request could hold hundreds of thousands of seasons: megabytes in the
/// database and, once accepted, a page fetched from the site for every one of them.
#[tokio::test]
async fn requests_and_selections_have_limits() {
    let s = setup(vec![link(Language::GermanDub, "voe", "https://x/ok")], 1);
    let sam = s.core.store.create_user("sam", "hash", Role::User);
    let sam = sam.unwrap().unwrap();
    let picked = |seasons: Vec<u32>, episodes: Vec<(u32, u32)>| kura_domain::Picked {
        all: false,
        seasons,
        episodes,
    };
    let ask = |picked: kura_domain::Picked| {
        let core = s.core.clone();
        async move {
            let choices = Choices::default();
            core.request(SiteId::AniWorld, "lantern-road", &picked, &choices, sam.id)
                .await
        }
    };
    let too_many = |result: Result<i64>| matches!(result, Err(CoreError::Invalid(_)));

    assert!(too_many(ask(picked((1..=201).collect(), Vec::new())).await));
    let episodes = (1..=5001).map(|number| (1, number)).collect();
    assert!(too_many(ask(picked(Vec::new(), episodes)).await));
    assert!(too_many(ask(picked(vec![1001], Vec::new())).await));
    assert!(too_many(ask(picked(Vec::new(), vec![(1, 100_001)])).await));
    // Repeats count once and are stored once.
    let id = ask(picked(vec![1; 10_000], vec![(1, 2); 10_000]))
        .await
        .unwrap();
    let stored = s.core.requests(Some(sam.id)).unwrap();
    let stored = stored.iter().find(|request| request.id == id).unwrap();
    assert_eq!(
        (stored.picked.seasons.len(), stored.picked.episodes.len()),
        (1, 1)
    );

    // A user may have as many requests waiting as they like.
    for _ in 0..150 {
        ask(picked(vec![1], Vec::new())).await.unwrap();
    }
    assert_eq!(s.core.requests(Some(sam.id)).unwrap().len(), 151);

    // A request stored before the limits existed is refused when accepted, and stays open.
    let old = s.core.store.create_request(&kura_store::NewRequest {
        user_id: sam.id,
        site: SiteId::AniWorld,
        slug: "lantern-road",
        series_title: "Lantern Road",
        picked: &picked((1..=250_000).collect(), Vec::new()),
        language: None,
        hoster: None,
        imdb_id: None,
        now: 0,
    });
    let old = old.unwrap();
    assert!(too_many(
        s.core.accept_request(old, s.user).await.map(|n| n as i64)
    ));
    assert!(s.core.jobs().unwrap().is_empty());
    let open = s.core.requests(None).unwrap();
    let open = open.iter().find(|request| request.id == old).unwrap();
    assert_eq!(open.state, kura_domain::RequestState::Open);
}

/// The title page showed an episode as missing, but Download Selected answered "Already in
/// the queue or downloaded": its finished download was still on record, its file was not.
#[tokio::test]
async fn an_episode_whose_file_was_deleted_can_be_downloaded_again() {
    let s = setup(vec![link(Language::GermanDub, "voe", "https://voe/x")], 1);
    // Only the episode the test is about, so Auto-Sync finds nothing new after it.
    s.list("lantern-road", 1, &[(1, &[DUB])]);
    s.queue(first(), Choices::default()).await.unwrap();
    let jobs = states(&s.core, &[JobState::Done]).await;
    let file = PathBuf::from(jobs[0].path.as_deref().unwrap());

    // Still there: skipped, as before.
    assert_eq!(s.queue(first(), Choices::default()).await.unwrap(), 0);

    // Cleared from the queue page and deleted from the disk.
    s.core.clear_finished().unwrap();
    std::fs::remove_file(&file).unwrap();
    let shown = s
        .core
        .downloaded_episodes(
            SiteId::AniWorld,
            "lantern-road",
            "Lantern Road",
            Some("tt0388629"),
        )
        .unwrap();
    assert!(shown.is_empty());
    // Auto-Sync leaves a deleted episode alone.
    assert_eq!(s.core.sync_once().await.unwrap(), 0);
    assert_eq!(s.queue(first(), Choices::default()).await.unwrap(), 1);
    let jobs = states(&s.core, &[JobState::Done]).await;
    assert!(!jobs[0].hidden);
    assert!(file.is_file());

    // Moved by hand to another file in the series folder: still downloaded, so skipped.
    let moved = file.with_file_name("Lantern Road S01E001 moved.mkv");
    std::fs::rename(&file, &moved).unwrap();
    assert_eq!(s.queue(first(), Choices::default()).await.unwrap(), 0);
}

#[tokio::test]
async fn a_queued_or_running_episode_is_not_queued_twice() {
    let s = setup(
        vec![link(Language::GermanDub, "voe", "https://voe/slow")],
        1,
    );
    let both = Selection::Picked {
        seasons: Vec::new(),
        episodes: vec![(1, 1), (1, 2)],
    };
    assert_eq!(s.queue(both.clone(), Choices::default()).await.unwrap(), 2);
    states(&s.core, &[JobState::Running, JobState::Queued]).await;
    assert_eq!(s.queue(both, Choices::default()).await.unwrap(), 0);
}

#[tokio::test]
async fn a_title_joins_auto_sync_when_its_download_starts() {
    let s = setup(
        vec![link(Language::GermanDub, "voe", "https://voe/slow")],
        1,
    );
    let both = Selection::Picked {
        seasons: Vec::new(),
        episodes: vec![(1, 1), (1, 2)],
    };
    s.queue(both, Choices::default()).await.unwrap();
    states(&s.core, &[JobState::Running, JobState::Queued]).await;
    let titles = s.core.sync_status().unwrap().titles;
    assert_eq!(titles.len(), 1);
    assert_eq!(titles[0].title.slug, "lantern-road");
    // Nothing is on disk yet, so Auto-Sync has nothing to add.
    assert_eq!(s.core.sync_once().await.unwrap(), 0);
}

/// A fake Jellyfin that answers 204 and records each request's head and body.
async fn fake_jellyfin() -> (String, Arc<Mutex<Vec<(String, String)>>>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let record = seen.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let mut data = Vec::new();
            let mut buf = [0u8; 4096];
            let (head, body) = loop {
                let read = socket.read(&mut buf).await.unwrap();
                data.extend_from_slice(&buf[..read]);
                let text = String::from_utf8_lossy(&data).into_owned();
                if let Some((head, body)) = text.split_once("\r\n\r\n") {
                    let length = head
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().ok())?
                        })
                        .unwrap_or(0);
                    if body.len() >= length || read == 0 {
                        break (head.to_owned(), body.to_owned());
                    }
                }
                if read == 0 {
                    break (text, String::new());
                }
            };
            record.lock().unwrap().push((head, body));
            let answer =
                "HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            let _ = socket.write_all(answer.as_bytes()).await;
        }
    });
    (url, seen)
}

#[tokio::test]
async fn jellyfin_scans_the_new_series_folders_once_the_queue_is_empty() {
    let s = setup(vec![link(Language::GermanDub, "voe", "https://voe/x")], 1);
    let (url, seen) = fake_jellyfin().await;
    let mut settings = s.core.settings().unwrap();
    settings.libraries[0].jellyfin_path = Some("/jf/tv".into());
    settings.jellyfin = kura_domain::Jellyfin {
        url: format!("{url}/"),
        api_key: "secret".to_owned(),
    };
    s.core.save_settings(&settings).unwrap();

    // One episode of another site that fails, then two that arrive.
    let failing = EpisodeId {
        site: SiteId::SerienStream,
        slug: "broken".to_owned(),
        season: 1,
        number: 1,
    };
    let dir = s.path("main").to_string_lossy().into_owned();
    s.core
        .store
        .enqueue(&kura_store::NewJob {
            episode: &failing,
            series_title: "Broken",
            user_id: None,
            language: None,
            hoster: None,
            dir: &dir,
            imdb_id: Some("none"),
            now: 0,
            replace_done: false,
            replaces: &[],
        })
        .unwrap();
    let both = Selection::Picked {
        seasons: Vec::new(),
        episodes: vec![(1, 1), (1, 2)],
    };
    s.queue(both, Choices::default()).await.unwrap();
    let jobs = states(&s.core, &[JobState::Failed, JobState::Done, JobState::Done]).await;

    for _ in 0..200 {
        if !seen.lock().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    // Later wake-ups of the worker do not ask again.
    s.core.clear_finished().unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1, "{seen:#?}");
    let (head, body) = &seen[0];
    assert!(head.starts_with("POST /Library/Media/Updated "), "{head}");
    assert!(head.contains("Token=\"secret\""), "{head}");

    let folder = downloaded::series_folder(Path::new(jobs[1].path.as_deref().unwrap())).unwrap();
    let name = folder.file_name().unwrap().to_string_lossy().into_owned();
    let body: serde_json::Value = serde_json::from_str(body).unwrap();
    let updates = body["Updates"].as_array().unwrap();
    // Only the one series folder, as Jellyfin sees it; not the whole library.
    assert_eq!(updates.len(), 1, "{body}");
    assert_eq!(updates[0]["Path"], format!("/jf/tv/{name}"));
    assert_eq!(updates[0]["UpdateType"], "Modified");
}

#[tokio::test]
async fn without_jellyfin_nothing_is_asked_and_bad_settings_are_refused() {
    let s = setup(vec![link(Language::GermanDub, "voe", "https://voe/x")], 1);
    let mut settings = s.core.settings().unwrap();
    settings.jellyfin.url = "jellyfin:8096".to_owned();
    settings.jellyfin.api_key = "key".to_owned();
    assert!(matches!(
        s.core.save_settings(&settings),
        Err(CoreError::Invalid(_))
    ));
    settings.jellyfin.url = "http://127.0.0.1:9".to_owned();
    settings.jellyfin.api_key = String::new();
    assert!(s.core.save_settings(&settings).is_err());
    // Off: downloads work and nothing is kept for later.
    s.queue(first(), Choices::default()).await.unwrap();
    states(&s.core, &[JobState::Done]).await;
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(s.core.pending_scan.lock().unwrap().is_empty());
    let unreachable = kura_domain::Jellyfin {
        url: "http://127.0.0.1:9".to_owned(),
        api_key: "key".to_owned(),
    };
    assert!(matches!(
        s.core.test_jellyfin(&unreachable).await,
        Err(CoreError::Jellyfin(_))
    ));
}

#[tokio::test]
async fn songs_are_kept_in_the_data_folder() {
    let s = setup(Vec::new(), 1);
    // Without a data folder there are none and none can be added.
    assert!(s.core.songs().unwrap().is_empty());
    assert!(matches!(
        s.core.add_song("a", b"ID3x"),
        Err(CoreError::Invalid(_))
    ));

    // The one song of Kura 1.3.0 joins the list.
    let data = s.path("data");
    std::fs::create_dir_all(data.join("theme")).unwrap();
    std::fs::write(data.join("theme/music.mp3"), b"ID3 old").unwrap();
    s.core.set_data_dir(data.clone());
    let songs = s.core.songs().unwrap();
    assert_eq!(songs.len(), 1);
    assert_eq!(songs[0].name, "Wii Shop Channel Main Theme");
    assert!(!data.join("theme/music.mp3").exists());
    assert_eq!(
        s.core.song(&songs[0].id).unwrap().unwrap().bytes,
        b"ID3 old"
    );

    assert!(matches!(
        s.core.add_song("x", b"<html>"),
        Err(CoreError::Invalid(_))
    ));
    let too_big = vec![0xFF; MOST_MUSIC_BYTES + 1];
    assert!(matches!(
        s.core.add_song("x", &too_big),
        Err(CoreError::Invalid(_))
    ));
    let second = s.core.add_song("Shop_Theme.mp3", b"ID3 second").unwrap();
    assert_eq!(second.name, "Shop Theme");
    let names: Vec<_> = s
        .core
        .songs()
        .unwrap()
        .into_iter()
        .map(|song| song.name)
        .collect();
    assert_eq!(names, ["Wii Shop Channel Main Theme", "Shop Theme"]);
    // Only listed ids name a file.
    assert!(s.core.song("../kura.db").unwrap().is_none());

    assert!(s.core.remove_song(&songs[0].id).unwrap());
    assert!(!s.core.remove_song(&songs[0].id).unwrap());
    assert!(s.core.song(&songs[0].id).unwrap().is_none());
    assert_eq!(s.core.songs().unwrap(), [second]);
}

#[tokio::test]
async fn a_theme_picks_from_its_own_list_and_known_songs() {
    let s = setup(Vec::new(), 1);
    let user = s.core.store.users().unwrap().remove(0);
    let mut theme = kura_domain::Theme::default();
    theme.ascii_library.push("aurora".into());
    theme.ascii_piece = "aurora".into();
    s.core.set_theme(&user, &theme).unwrap();

    let bad = |change: fn(&mut kura_domain::Theme)| {
        let mut theme = kura_domain::Theme::default();
        change(&mut theme);
        matches!(s.core.set_theme(&user, &theme), Err(CoreError::Invalid(_)))
    };
    assert!(bad(|theme| theme.ascii_piece = "donut".into()));
    assert!(bad(|theme| theme.ascii_library.push("../x".into())));
    assert!(bad(|theme| theme.volume = 101));
    assert!(bad(|theme| theme.song = Some("missing".into())));
}

impl Setup {
    /// Makes the fake site list these episodes, each with its languages, for one season.
    fn list(&self, slug: &str, season: u32, episodes: &[(u32, &[Language])]) {
        let episodes = episodes
            .iter()
            .map(|(number, languages)| Episode {
                season,
                number: *number,
                title: format!("Episode {number}"),
                languages: languages.to_vec(),
            })
            .collect();
        let mut listing = self.listing.lock().unwrap();
        listing.insert((slug.to_owned(), season), episodes);
    }

    /// Queues single episodes of a series in one language, as the test user.
    async fn queue_in(&self, slug: &str, episodes: &[(u32, u32)], language: Language) {
        let selection = Selection::Picked {
            seasons: Vec::new(),
            episodes: episodes.to_vec(),
        };
        let choices = Choices {
            language: Some(language),
            ..Choices::default()
        };
        let queued = self
            .core
            .enqueue(SiteId::AniWorld, slug, selection, &choices, self.user);
        queued.await.unwrap();
    }

    fn tracked(&self, slug: &str) -> TrackedTitle {
        let status = self.core.sync_status().unwrap();
        let title = status
            .titles
            .into_iter()
            .find(|title| title.title.slug == slug);
        title.unwrap()
    }
}

const DUB: Language = Language::GermanDub;
const SUB: Language = Language::GermanSub;

/// The Detective Is Already Dead: S02E05 came out in German dub, but Auto-Sync never queued it.
/// An earlier download of that episode had finished in another language and its file was gone,
/// and every download on record counted as "already there", whatever its language.
#[tokio::test]
async fn a_dub_episode_is_queued_although_a_download_in_another_language_is_on_record() {
    let links = vec![
        link(DUB, "voe", "https://voe/dub"),
        link(SUB, "voe", "https://voe/sub"),
    ];
    let s = setup(links, 1);
    s.list(
        "detective",
        1,
        &[(1, &[DUB, SUB]), (2, &[DUB, SUB]), (3, &[SUB])],
    );
    s.queue_in("detective", &[(1, 1), (1, 2)], DUB).await;
    s.queue_in("detective", &[(1, 3)], SUB).await;
    let jobs = states(&s.core, &[JobState::Done; 3]).await;
    std::fs::remove_file(jobs[2].path.as_deref().unwrap()).unwrap();

    // Only the sub is out: nothing to do yet, and the page says what it waits for.
    assert_eq!(s.core.sync_once().await.unwrap(), 0);
    let title = s.tracked("detective");
    assert_eq!(title.language, Some(DUB));
    assert_eq!(title.on_disk, 2);
    assert_eq!(title.newest_on_disk, Some((1, 2)));
    let check = title.check.unwrap();
    assert_eq!(check.newest_listed, Some((1, 3)));
    assert_eq!(check.waiting.len(), 1);
    assert_eq!(check.waiting[0].episode, (1, 3));
    assert_eq!(check.waiting[0].offered, [SUB]);

    // The dub is out now.
    s.list(
        "detective",
        1,
        &[(1, &[DUB, SUB]), (2, &[DUB, SUB]), (3, &[DUB, SUB])],
    );
    assert_eq!(s.core.sync_once().await.unwrap(), 1);
    let jobs = states(&s.core, &[JobState::Done; 3]).await;
    assert_eq!(jobs[2].language, Some(DUB));
    assert!(jobs[2].path.as_deref().unwrap().ends_with("German Dub.mkv"));
    let title = s.tracked("detective");
    assert_eq!(title.on_disk, 3);
    assert_eq!(title.check.unwrap().queued, [(1, 3)]);
    assert_eq!(s.core.sync_once().await.unwrap(), 0);
}

#[tokio::test]
async fn a_failed_download_is_tried_again_on_every_run() {
    let s = setup(vec![link(DUB, "voe", "https://voe/dub")], 1);
    s.queue_in("lantern-road", &[(1, 1)], DUB).await;
    // No sub links: this one fails.
    s.queue_in("lantern-road", &[(1, 2)], SUB).await;
    states(&s.core, &[JobState::Done, JobState::Failed]).await;

    // Episode 2 is tried again, in the title's language, and episode 3 is new.
    assert_eq!(s.core.sync_once().await.unwrap(), 2);
    let jobs = states(&s.core, &[JobState::Done; 3]).await;
    assert_eq!(jobs[1].language, Some(DUB));
}

#[tokio::test]
async fn a_download_that_keeps_failing_is_queued_on_every_run() {
    let s = setup(vec![link(DUB, "voe", "https://voe/broken")], 1);
    s.list("detective", 1, &[(1, &[DUB]), (2, &[DUB])]);
    s.queue_in("detective", &[(1, 1)], DUB).await;
    states(&s.core, &[JobState::Failed]).await;
    // Nothing finished yet, so episode 2 is not new; episode 1 is tried again each time.
    for _ in 0..2 {
        assert_eq!(s.core.sync_once().await.unwrap(), 1);
        states(&s.core, &[JobState::Failed]).await;
    }
}

#[tokio::test]
async fn a_sub_episode_is_replaced_once_its_dub_is_out() {
    let links = vec![
        link(DUB, "voe", "https://voe/dub"),
        link(SUB, "voe", "https://voe/sub"),
    ];
    let s = setup(links, 1);
    s.list("detective", 1, &[(1, &[DUB, SUB]), (2, &[SUB])]);
    s.queue_in("detective", &[(1, 1)], DUB).await;
    s.queue_in("detective", &[(1, 2)], SUB).await;
    let jobs = states(&s.core, &[JobState::Done; 2]).await;
    let sub = PathBuf::from(jobs[1].path.as_deref().unwrap());
    let beside = |end: &str| {
        let stem = sub.file_stem().unwrap().to_string_lossy();
        sub.with_file_name(format!("{stem}{end}"))
    };
    for end in [".nfo", "-thumb.jpg"] {
        std::fs::write(beside(end), b"x").unwrap();
    }

    // The settings rank dub first: the title follows the dub, and the sub stays for now.
    assert_eq!(s.core.sync_once().await.unwrap(), 0);
    assert_eq!(s.tracked("detective").language, Some(DUB));
    assert!(sub.is_file());

    s.list("detective", 1, &[(1, &[DUB, SUB]), (2, &[DUB, SUB])]);
    assert_eq!(s.core.sync_once().await.unwrap(), 1);
    let jobs = states(&s.core, &[JobState::Done; 2]).await;
    assert_eq!(jobs[1].language, Some(DUB));
    let dub = PathBuf::from(jobs[1].path.as_deref().unwrap());
    assert!(dub.is_file());
    assert_ne!(dub, sub);
    // The sub, its NFO and its thumbnail are gone, so Jellyfin reads the episode afresh.
    assert!(!sub.exists());
    assert!(!beside(".nfo").exists());
    assert!(!beside("-thumb.jpg").exists());
    assert_eq!(s.core.sync_once().await.unwrap(), 0);
}

#[tokio::test]
async fn a_season_page_without_episodes_is_a_failed_check() {
    let s = setup(vec![link(DUB, "voe", "https://voe/dub")], 1);
    s.list("detective", 1, &[(1, &[DUB])]);
    s.queue_in("detective", &[(1, 1)], DUB).await;
    states(&s.core, &[JobState::Done]).await;
    s.list("detective", 1, &[]);
    assert_eq!(s.core.sync_once().await.unwrap(), 0);
    let check = s.tracked("detective").check.unwrap();
    let error = check.error.unwrap();
    assert!(error.contains("no episodes"), "{error}");
}

/// Watched and deleted: the title still follows its download history, and what was deleted
/// stays deleted.
#[tokio::test]
async fn new_episodes_come_after_the_newest_download_also_once_its_file_is_deleted() {
    let s = setup(vec![link(DUB, "voe", "https://voe/dub")], 1);
    s.queue_in("lantern-road", &[(1, 1)], DUB).await;
    let jobs = states(&s.core, &[JobState::Done]).await;
    std::fs::remove_file(jobs[0].path.as_deref().unwrap()).unwrap();
    assert_eq!(s.core.sync_once().await.unwrap(), 2);
    let jobs = states(&s.core, &[JobState::Done; 3]).await;
    assert!(!PathBuf::from(jobs[0].path.as_deref().unwrap()).exists());
    assert_eq!(s.tracked("lantern-road").on_disk, 2);
}
