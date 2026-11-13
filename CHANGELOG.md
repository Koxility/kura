# Changelog

All notable changes to Kura are listed here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

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

[Unreleased]: https://github.com/Koxility/kura/compare/v1.0.0...HEAD
[1.0.0]: https://github.com/Koxility/kura/releases/tag/v1.0.0
