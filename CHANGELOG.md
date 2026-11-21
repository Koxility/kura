# Changelog

All notable changes to Kura are listed here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [1.3.2] - 2026-11-21

### Fixed

- Every piece on ascii.rest can be added as a background, e.g. `https://ascii.rest/planet-scale/`. Kura used the npm package of ascii.rest, which lags behind the website and missed 26 pieces; it now carries a copy of ascii.rest's source (in `web/vendor/ascii.rest`, MIT), so all of them work and still nothing is loaded from ascii.rest.

### Changed

- Latest episodes show the date and time on the right again: episode, language flag and title on the left.
- *Calendar* sits between *Library* and *Auto-Sync* in the top bar.

## [1.3.1] - 2026-11-21

### Changed

- **Calendar** shows AniWorld's whole week, read from its calendar page: a Monday-to-Sunday bar on top, and below it the days stacked from today on, each under its weekday and date. Every day is a grid of covers like the lists on Home; under each cover are the title, a green check when the episode is out, `S02E07`, the language flags and the time. A weekday in the bar scrolls to its next date. One episode coming out in several languages at the same time is one card with several flags.
- **Latest episodes** on Home read left to right: date and time, `S01 E08`, the language as a flag the way AniWorld marks it (German flag for the dub, the Japanese flag with a German or British corner for subtitles), and the title. The list no longer scrolls inside the page: it shows 12 episodes and a *Show all* button.
- **Background music** has a *Song* choice and a *Volume* slider per account, admins included. Admins add and remove songs under *Settings → Theme*; the song of 1.3.0 is kept and listed as "Wii Shop Channel Main Theme".
- **ASCII background** keeps a list of scenes per account. Paste the link of any piece from ascii.rest to add it, pick one, remove the others. Pieces with colour show their colours, washed out; every piece ships with Kura, so nothing is loaded from ascii.rest.

## [1.3.0] - 2026-11-21

### Added

- **Latest episodes on Home.** AniWorld's 50 newest episodes at the bottom of the home page: season and episode with the title, the language, and when it came out. Each row opens the title.
- **Calendar.** A new tab, for every user, with AniWorld's release calendar: the days from the day before yesterday to the day after tomorrow, each with the episodes announced, their time, language and whether they are out yet.
- **Settings for everyone.** Normal users now have Settings with two sections, *User* and *Theme*. Under *User* every account changes its name, its password and an optional email address (stored, not used yet). Admins find the same under *Users*, above the list of all users.
- **API keys** under *Settings → User*, so other programs, such as Musubi, can use Kura with `Authorization: Bearer kura_…` instead of a login. A key is shown once and can do what its account can. [docs/API.md](docs/API.md) describes the API a client needs.
- **Theme**, per account:
  - *Wii Shop Channel Main Theme*, on by default, plays on a loop while Kura is open. Kura does not ship the song: an admin uploads an MP3 under *Settings → Theme*, and it stays in the data folder. Browsers start sound only after the first click or key on the page.
  - *ASCII background*, off by default: a faint animated scene from [ascii.rest](https://ascii.rest) behind the pages, in one of eight scenes.
- A footer with the copyright, the version and the commit the build came from.

### Changed

- A title's page offers only the languages the picked episodes, or the whole title, are really available in. A title only offered with German subtitles no longer lists German dub.
- The version moved from the top bar to the footer; the *Update* label sits next to the name Kura.
- Clicking your name in the top bar no longer opens the password change; that is in *Settings → User* now.

## [1.2.10] - 2026-11-21

### Added

- Kura can tell Jellyfin about new episodes. Under *Settings → Jellyfin*, enter the server's address and an API key. When the queue is empty, Kura asks Jellyfin once to scan the series folders that got new episodes, and only those, never the whole library. A season or an Auto-Sync batch causes one scan, not one per episode. A failed download stays in the queue and does not hold back the scan of what arrived. If Jellyfin sees a library under another path, set *Path in Jellyfin* for that library.

### Fixed

- **Download Selected** no longer answers "Already in the queue or downloaded." for episodes that are neither. An episode whose file was deleted after it was downloaded still counted as done, so it could never be queued again, while its title page showed it as missing. Kura now reads the folders and the queue again when you queue, and downloads such an episode again. Episodes that are queued, downloading or still in a library folder are skipped as before, and Auto-Sync still never downloads an episode again that you deleted.

### Changed

- A title joins Auto-Sync as soon as its first download starts, not only once one has finished.
- A user can have any number of requests waiting for an admin; the limit of 100 is gone.

## [1.2.9] - 2026-11-18

### Fixed

- A download no longer shows a negative speed like `-2925931638 B/s` when it starts over. When a try failed, the byte count went back to zero and the speed was worked out from that drop.
- The speed no longer keeps falling to 0 B/s and jumping back. ffmpeg writes a stream part by part, so two reports in a row often had the same byte count; the speed is now the average over the last 10 seconds.
- A finished file is no longer thrown away and downloaded again when ffmpeg fails only at the very end, e.g. on the last part or while closing the file. Kura keeps the file when ffmpeg reached the end of the stream and the file itself has the stream's full length; without a stated length it still tries again.

### Changed

- A download that starts over shows "Retrying…" and why, e.g. "Try 1 of 3 failed, trying again: …", instead of looking like a new download at 0%.
- ffmpeg keeps the connection to the hoster and loads the next part of an HLS stream while the current one finishes (`-http_persistent 1 -http_multiple 1`), so it waits less between parts.

## [1.2.8] - 2026-11-18

### Fixed

- Switching pages always shows the current state. Pages kept what they had read for 30 seconds, so the queue, requests and settings could show an old state until the browser reloaded. Kura's own data is now read again whenever a page shows it, and again when the browser tab comes back to the front; the last known state shows meanwhile. Accepting, rejecting, cancelling and clearing requests also show their result at once.
- The queue shows at once when a download finishes and the next one starts. Behind nginx and similar reverse proxies, the live updates sat in the proxy's buffer for minutes; Kura now asks proxies not to hold them back (`X-Accel-Buffering: no`). Progress for a download the page did not know had started makes it read the queue again, and while something downloads the queue is also read every 5 seconds instead of 15, in case an update is lost on the way.
- Several open Kura tabs no longer stall each other. Each tab kept a live connection open, and over plain HTTP a browser allows only six per server, so with a few tabs pages loaded slowly or not at all until one was closed. Only the tab in front keeps the connection now; a tab coming back reconnects and reads everything again.

### Changed

- The language, provider and library choices on a title's page show the real default first instead of "Automatic" and "Default", e.g. "German dub", "VOE" and the site's library. Leaving the default still tries the next languages and providers from the settings when it has no link.
- The queue and requests show the default language and provider instead of "Language from the settings" when none was picked.

### Performance

- The web app is sent compressed (Brotli or gzip), and its scripts, styles and fonts are kept by the browser for good, since their names change with every build. The page itself is checked with an `ETag` and answered with "not modified" when it has not changed. API answers are marked `no-store`, so no browser or proxy serves an old one.

## [1.2.7] - 2026-11-18

**Running Kura behind a reverse proxy (HAProxy, Caddy, Traefik, nginx)?** Updating alone does not fix the login lockout below. Set `KURA_TRUSTED_PROXIES` to the proxy's address and have the proxy send `X-Forwarded-For`; see [Behind a reverse proxy](https://github.com/Koxility/kura/blob/master/docs/wiki/Docker-Compose.md#behind-a-reverse-proxy).

### Security

- Behind a reverse proxy like HAProxy, wrong logins from one person no longer lock everyone else out. Kura saw every login as coming from the proxy, so five wrong passwords for `admin` from anywhere on the internet held up the real admin's logins for up to 15 minutes, again and again. The new setting `KURA_TRUSTED_PROXIES` names the proxy; Kura then counts by the address in `X-Forwarded-For`. The header is ignored from anyone else, so it cannot be made up.
- Users can no longer flood the server with requests. A request could name hundreds of thousands of seasons: 1.6 MB each, stored with no limit, and accepting one fetched a page from the site for every season (250,000 fetches and 750,000 queued episodes in our test). One request now names up to 200 seasons or 5000 episodes, repeats are dropped, and a user can have up to 100 open requests. An oversized request stored before is refused when accepted, and stays open to be rejected.
- Sites and hosters can no longer send Kura into the local network. A link or redirect from a site or hoster page made Kura fetch any address, e.g. the router or a NAS. Kura now fetches only from public addresses: names are checked every time they are looked up, addresses in links and redirects before they are used, and the stream address before ffmpeg gets it. Proxies set in `HTTPS_PROXY` and similar may still be local. ffmpeg fetches the parts of a stream by itself; those addresses cannot be checked.

### Added

- The login cookie is marked `Secure` when a trusted reverse proxy says the browser came over HTTPS (`X-Forwarded-Proto: https`).

## [1.2.6] - 2026-11-15

### Security

- Passwords can no longer be guessed at speed. After five wrong logins for a name from one address, the next try has to wait 30 seconds, and every further wrong try doubles the wait, up to 15 minutes. A login that works starts the count again, and sessions that exist go on working. Before, Kura answered as many tries as it was sent, about 70 a second. Behind a reverse proxy every login comes from the proxy's address, so there the limit is per name.
- A burst of logins no longer uses up the server's memory. Checking a password takes about 19 MiB, and logging in needs no account: 64 logins at once took 2.3 GB, enough to get a small server or a container with a memory limit stopped. Kura now checks four passwords at a time and the rest wait their turn.
- Users without admin rights no longer see where the files are on the server. The queue sent every user the library folder and the full file path of each download, and errors from ffmpeg could name them too. Admins still get them.
- Kura no longer tells someone who is not logged in which folder it runs in. The suggested library path was part of the status every visitor gets; now only admins get it.

## [1.2.5] - 2026-11-15

### Fixed

- A download that fails on the stream, like VOE answering 404, is tried again by itself. The stream addresses a hoster hands out expire and name one server, and Kura asked for one only once per link. Now it asks for a fresh address and tries up to three times before it moves on to the next provider or language. The error shows what the hoster answered and that it was tried three times.
- Downloads no longer hang at some percentage. ffmpeg waited without limit on a connection that went quiet. Now it gives up on a connection after 30 seconds, and Kura stops a download that got no further for 90 seconds; both count as a failed try and start again from a fresh address.
- A download whose stream is missing a part, or ends well before its stated length, fails and is tried again. ffmpeg leaves such parts out and reports success, so the episode was saved as done with a gap or cut short.
- The progress bar goes back to the start when a download starts over, instead of staying at the last value.
- The queue page catches up by itself. It reads the queue again when the live connection comes back, e.g. after a phone woke up or Kura restarted, reconnects when the browser gave up, checks every 15 seconds while something is downloading, and shows the result of Cancel and Retry at once.
- The app is usable on phones:
  - The page links have their own line under the name, so every page can be reached; Logout stays in view.
  - Queue: an error has the full width below its episode instead of a column one letter wide.
  - A title's page: episode titles and languages no longer overlap; the languages go below the title.
  - Auto-Sync, Library and Settings: long titles and paths wrap onto their own line.
  - Text fields no longer make iPhones zoom in.

## [1.2.4] - 2026-11-15

### Added

- Kura has a logo. It shows in the browser tab, in bookmarks and favourites, and in the README.

## [1.2.3] - 2026-11-14

### Fixed

- Deleted episodes no longer show as downloaded. A finished download counts only while its file is still there, and a series folder only while it holds episode files. Kura reads the libraries again every 5 minutes, before every Auto-Sync run and whenever a title's page is opened, instead of trusting what it saw earlier. Auto-Sync still does not download a deleted episode again.
- The Library page shows only folders and `.mkv` and `.mp4` files; posters, subtitles, `.nfo` and other files are left out.
- AniWorld search results show covers. The search sends none, so Kura takes them from the series pages and remembers them.

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

[Unreleased]: https://github.com/Koxility/kura/compare/v1.3.2...HEAD
[1.3.2]: https://github.com/Koxility/kura/compare/v1.3.1...v1.3.2
[1.3.1]: https://github.com/Koxility/kura/compare/v1.3.0...v1.3.1
[1.3.0]: https://github.com/Koxility/kura/compare/v1.2.10...v1.3.0
[1.2.10]: https://github.com/Koxility/kura/compare/v1.2.9...v1.2.10
[1.2.9]: https://github.com/Koxility/kura/compare/v1.2.8...v1.2.9
[1.2.8]: https://github.com/Koxility/kura/compare/v1.2.7...v1.2.8
[1.2.7]: https://github.com/Koxility/kura/compare/v1.2.6...v1.2.7
[1.2.6]: https://github.com/Koxility/kura/compare/v1.2.5...v1.2.6
[1.2.5]: https://github.com/Koxility/kura/compare/v1.2.4...v1.2.5
[1.2.4]: https://github.com/Koxility/kura/compare/v1.2.3...v1.2.4
[1.2.3]: https://github.com/Koxility/kura/compare/v1.2.2...v1.2.3
[1.2.2]: https://github.com/Koxility/kura/compare/v1.2.1...v1.2.2
[1.2.1]: https://github.com/Koxility/kura/compare/v1.2.0...v1.2.1
[1.2.0]: https://github.com/Koxility/kura/compare/v1.1.0...v1.2.0
[1.1.0]: https://github.com/Koxility/kura/compare/v1.0.0...v1.1.0
[1.0.0]: https://github.com/Koxility/kura/releases/tag/v1.0.0
