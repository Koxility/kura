# Changelog

All notable changes to Kura are listed here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [1.2.2] - 2026-11-14

### Fixed

- The Requests and Queue pages show the language of every request, title and episode. A finished download records the language it was actually downloaded in, also when none was picked.
- Titles are marked as downloaded more reliably. Names match without spaces and without bracketed parts like a year (`Re:Zero` = `ReZero (2016)`), a folder with a different IMDb ID no longer matches by name, and a series found by IMDb ID on its page is marked in lists too.
- A title in the queue stays one line, however many of its episodes download at once.
- **Remove** on the Auto-Sync page takes a title off the tracked list instead of moving it to the excluded titles. It is tracked again once another episode of it is downloaded.
- Auto-Sync stays with the language of the episodes already downloaded: after a German dub it queues only episodes offered as German dub and never falls back to German sub. Episodes already in the library, also ones Kura did not download, are not queued again.

## [1.2.1] - 2026-11-13

### Fixed

- The Update label appears within an hour of a new release. Kura asked GitHub only once a day, so an instance that was already running when a release came out showed nothing until the next day; a failed check also waited a full day. Now Kura asks every hour and retries a failed check after 5 minutes.

## [1.2.0] - 2026-11-13

### Library

- Titles and episodes that are already downloaded show a green check: on Home at the top left of the cover, on a title's page left of every episode and per season, and in the Library next to every series folder.
- Kura recognises downloads by its own records and by library folders with the same IMDb ID or name, so files it did not download count too.
- New series go directly into the library folder, without the `Series` folder in between: `<library>/One Piece [imdbid-tt0388629]/Season 01/`. Series that already have a folder keep it, so nothing is split or moved.

### Requests

- Users can cancel their own requests while they are open.
- Admins can clear accepted and rejected requests with **Clear requests**.

### Auto-Sync

- **Remove** next to a tracked title stops Auto-Sync for it.

### Accounts

- Every account can change its own password by clicking its name in the top bar. Other devices with the account are logged out.

### Interface

- The login page shows Kura centred.

## [1.1.0] - 2026-11-13

### Docker

- Kura is published as a Docker image, `ghcr.io/koxility/kura`, for amd64 and arm64. Every release is built and published automatically and tagged with its version, its minor and major series, and `latest`.
- A ready-to-use `docker-compose.yaml` with volumes for the database and your media.
- The image checks its own health, stops cleanly on `docker stop`, and runs as an unprivileged user.
- `kura --version`, `kura --help` and `kura healthcheck`; unknown arguments print the help instead of starting the server.
- `KURA_LIBRARY_DIR` sets the folder the library setup suggests (`/media` in the image).

### Naming template

- New setting under Settings → Downloads: the file name of every episode, from `{title}`, `{episodenumber}`, `{language}`, `{season}`, `{episode}` and `{imdbid}`, with a live preview. `KURA_NAMING_TEMPLATE` sets it for the whole installation while the setting is empty.
- Episode numbers have three digits: `S01E001`, `S10E120`.
- The default is `{title} {episodenumber} {language}`, e.g. `One Piece S01E001 German Dub.mkv`. Files downloaded before keep their names.

### Updates

- A small blue **Update** label next to the version in the top bar when a newer release is out, linking to it. Kura checks at start and then once a day.

### Documentation

- A wiki with installation, Docker Compose, first start, configuration, updating, roles and requests, folder structure, troubleshooting and development.
- The README now starts with Docker.

## [1.0.0] - 2026-11-13

First public release.

### Browsing

- Home page per site (AniWorld, SerienStream) with search, a random title, genres, new and popular titles. Genre results load further pages while you scroll.
- Title page with the seasons as a tree: open a season to see its episodes, tick single episodes or whole seasons, then download the selection or everything.

### Downloads

- Downloads single episodes, seasons or whole series, four at a time by default.
- Language and provider can be chosen per download; otherwise Kura follows the order in the settings and falls back to the next provider and language.
- Supported providers: VOE and Vidmoly.
- Output as mkv or mp4, copied without re-encoding by ffmpeg.
- Files are stored as `<library>/Series/<Title> [imdbid-…]/Season XX/<Title> - SXXEYY.<ext>`. The IMDb ID comes from the site; when it has none, Kura asks for it or uses `[imdbid-none]`.
- Queue with live progress, one entry per title ("2 of 4 episodes downloaded"), filters, search, sort order and Clear Finished.

### Libraries

- Downloads go to libraries: named folders with a default per site. A short setup creates the first one on a fresh installation.
- Library page with the folders as a tree.

### Auto-Sync

- Checks downloaded series for new episodes every few minutes or once a day, optionally filling earlier gaps too.
- Sync now button, a list of tracked titles, and titles that are left out.

### Users and requests

- Local accounts with an admin and normal users.
- Normal users see Home, Queue and Request. They request titles instead of downloading them; admins accept or reject requests, and accepted ones are queued.

### Administration

- Settings in sections: Downloads, Library, Sites, Auto-Sync, Users and Administration.
- Configuration export and import, and an IP check.
- Shows when a newer release is available.

### Server

- One binary with the web app built in, a JSON API under `/api/v1` and live events.

[Unreleased]: https://github.com/Koxility/kura/compare/v1.2.2...HEAD
[1.2.2]: https://github.com/Koxility/kura/compare/v1.2.1...v1.2.2
[1.2.1]: https://github.com/Koxility/kura/compare/v1.2.0...v1.2.1
[1.2.0]: https://github.com/Koxility/kura/compare/v1.1.0...v1.2.0
[1.1.0]: https://github.com/Koxility/kura/compare/v1.0.0...v1.1.0
[1.0.0]: https://github.com/Koxility/kura/releases/tag/v1.0.0
