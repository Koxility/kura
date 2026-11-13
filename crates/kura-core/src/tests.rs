use std::time::Duration;

use kura_domain::{AutoSync, JobState, Language, Library, Role};
use kura_hosters::HosterError;

use super::*;

/// One series with three episodes; every episode has the same links.
struct FakeSite(Vec<HosterLink>);

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
                site: SiteId::AniWorld,
                slug: slug.to_owned(),
                title: "Lantern: Road?".to_owned(),
                cover_url: None,
            },
            description: String::new(),
            seasons: vec![1],
            // One fake series has an ID on the site, the other has none.
            imdb_id: (slug == "lantern-road").then(|| "tt0388629".to_owned()),
        })
    }

    async fn episodes(&self, _: &str, season: u32) -> kura_sites::Result<Vec<Episode>> {
        Ok((1..=3)
            .map(|number| Episode {
                season,
                number,
                title: format!("Episode {number}"),
                languages: vec![Language::GermanDub],
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

/// Resolves a link to itself, unless the link says the video is gone.
struct FakeHoster(&'static str);

#[async_trait]
impl Hoster for FakeHoster {
    fn name(&self) -> &'static str {
        self.0
    }

    async fn resolve(&self, url: &str) -> kura_hosters::Result<Stream> {
        if url.contains("gone") {
            return Err(HosterError::NoStream("fake"));
        }
        Ok(Stream {
            url: url.to_owned(),
            headers: Vec::new(),
        })
    }
}

/// Fails on "broken" streams, never finishes "slow" ones, and records the rest.
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
        if stream.url.contains("slow") {
            std::future::pending::<()>().await;
        }
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
    let core = Core::new(
        store,
        vec![Arc::new(FakeSite(links)), Arc::new(FailingSite)],
        vec![Box::new(FakeHoster("voe")), Box::new(FakeHoster("vidmoly"))],
        Box::new(downloader),
    );
    tokio::spawn(core.clone().run());
    Setup {
        core,
        downloads,
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

    let target = s.path(
        "main/Series/Lantern Road [imdbid-tt0388629]/Season 01/Lantern Road S01E001 German Sub.mkv",
    );
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
             vidmoly, german_dub: ffmpeg failed: stream ended early"
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
    assert!(file.starts_with(s.path("other/Series")));
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
    };
    let path = |imdb| {
        let template = "{title} {episodenumber}";
        target_path(
            Path::new("dl"),
            template,
            &job(imdb),
            Language::GermanDub,
            OutputFormat::Mp4,
        )
    };
    let file = "Season 00/some-slug S00E012.mp4";
    let id = Path::new("dl/Series/some-slug [imdbid-tt0388629]").join(file);
    assert_eq!(path(Some("tt0388629")), id);
    let none = Path::new("dl/Series/some-slug [imdbid-none]").join(file);
    assert_eq!(path(Some("none")), none);
    // Downloads from before IMDb IDs keep their folder.
    assert_eq!(path(None), Path::new("dl/Series/some-slug").join(file));
    // A broken stored value never reaches the path.
    assert_eq!(
        path(Some("../x")),
        Path::new("dl/Series/some-slug").join(file)
    );
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
    assert_eq!(status.titles[0].downloaded, 2);
    assert_eq!(status.titles[0].result.as_deref(), Some("Nothing new"));

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
async fn the_library_view_stays_inside_its_folders() {
    let s = setup(Vec::new(), 1);
    let dir = tempfile::tempdir().unwrap();
    let season = dir.path().join("Series/Show/Season 01");
    std::fs::create_dir_all(&season).unwrap();
    std::fs::write(season.join("Show - S01E01.mkv"), b"12345").unwrap();
    std::fs::write(season.join("Show - S01E02.mkv.part"), b"1").unwrap();
    std::fs::write(season.join(".hidden"), b"1").unwrap();
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
    let file = Entry {
        name: "Show - S01E01.mkv".to_owned(),
        folder: false,
        size: 5,
    };
    assert_eq!(files, [file]);

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
