# Changelog

All notable changes to Kura are listed here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Fixed

- **Auto-Sync downloads an episode whose dub comes out after a later one.** With *Only new episodes*, everything up to the newest episode on record counted as old, also a gap in between: with episodes 1 to 4 on disk in German Dub and a German Dub download of episode 6 on record, episode 5 was never listed as waiting and never queued once its dub was out. Now, in the newest season, every episode after the first one you have is wanted, and only those before it (and earlier seasons) count as passed over.
- **The Auto-Sync page says why an episode is left out.** An opened card lists the episodes of the newest season Auto-Sync leaves alone, each with its reason: deleted after a download, cancelled, on disk in a language ranked lower, or before the first episode you have of the season. Each check also writes one line per series to the log.

## [1.5.3] - 2026-11-27

Auto-Sync, reworked after it missed a newly released episode. It now decides about every listed episode by what is on disk in the series' language, and its page shows what is really there.

### Fixed

- **Auto-Sync no longer misses an episode because of an earlier download in another language.** Any download on record counted as "already there", whatever its language and whether its file was still on disk. So once an episode had been downloaded in German Sub and that file was gone, its German Dub was never queued, neither on schedule nor with *Sync now*. Now an episode counts as downloaded only while a file of it is in the library in the series' language (or in a file whose name tells none).
- **A series keeps its language.** Auto-Sync followed the language of the newest file on disk, so one episode in another language could switch a series over. It now follows the best language the series has, by the order under *Settings → Downloads*.
- **Failed downloads from Auto-Sync are tried again** on every check, instead of blocking their episode for good. Cancelled downloads stay cancelled.
- **Watching and deleting no longer stops a series.** With every file of a series deleted, Auto-Sync did nothing and said "Nothing new". It now goes on from the newest episode downloaded, and still never downloads a deleted episode again.
- **A season page Kura cannot read is a failed check**, not "Nothing new", and is asked for once more first. One season that fails no longer stops the check of the others.
- AniWorld flags that load lazily are read in season lists too, so no episode is listed without its languages.

### Changed

- **Auto-Sync replaces an episode once its better language is out**, e.g. German Dub for a series that follows the dub while that episode is only on disk as German Sub. When the new download is done, the old file is deleted with its `.nfo` and thumbnail, before Jellyfin is asked to scan.
- **The Auto-Sync page shows what is really there, with room to read it.** The number next to a title counted finished downloads, also deleted ones and ones in other languages (19 for a series with 16 episodes on disk). Each title is now a card: its name, with small chips under it for its language (with flag), the episodes on disk in that language and the newest of them, and the newest episode the site lists; its state on the right with a coloured dot (queued, waiting, check failed, nothing new). A click opens the site, the last check with its time, the episodes waiting for the language and which languages they are offered in, and *Open title* and *Remove*. Cards with a failed check or waiting episodes start open.
- The results of the last checks and the time of the last run are kept over a restart, so the next scheduled run follows the last one instead of starting at once.
- *Sync now* while a check is running waits for that check instead of starting a second one.

## [1.5.2] - 2026-11-25

A security release from an audit of mail, password reset, invites and the database. Updating is recommended for every installation.

### Security

- **An admin's API key can no longer take over Kura.** Creating or deleting accounts, sending or revoking invites, changing the mail server (and its test mail) and saving the settings now need a login with the password, like setting someone's password already did. Before, a key that got out could create a second admin, log in as it and lock the owner out, or point every reset link at another website. Programs keep everything else: queue, requests, browsing and reading.
- **The brake on reset requests and invite links counts per name, not per network.** Five reset requests for one name now hold up only requests for that name, and tries at an invite link only that link. Before, anyone could block password resets and sign-ups for everyone on the same address, which behind a reverse proxy without `KURA_TRUSTED_PROXIES` is everyone. A made-up invite link is now refused before any password is hashed. The log's hint about an untrusted proxy comes back every hour and says what it means for logins and resets.
- **The data folder is for Kura alone.** It is made `0700`, and the database `0600`, also on existing installations at the next start (a warning in the log if that is not possible). The database keeps the mail password and the Jellyfin API key as they are, since Kura has to send them.
- **The Jellyfin API key is no longer shown**, like the mail password: the settings, also the exported configuration, only say that one is saved. An empty field keeps it, but only for the same Jellyfin address.
- **The mail and Jellyfin tests no longer pass on what the server wrote.** They say in fixed words what went wrong (could not connect, not a mail server, refused with code 550, …), and Kura's log has the whole answer. Before, the test mail read out the greeting of whatever service ran at the address entered, which mapped the network Kura is in.
- **A new email address counts once its link is opened, and every address belongs to one account.** Changing the own email mails a link to the new address (it works once and for 24 hours, under *Settings → Account* the address shows as waiting); reset links go there only after it is opened. An address another account or an open invite has is refused, whatever the case of its letters (also for addresses with letters beyond A to Z), when an admin creates a user or sets an email, and when an invite is redeemed. Before, any account could enter any address, also someone else's. Addresses an admin sets count at once, and addresses saved before stay as they are.
- **Names cannot be probed at speed.** Changing the own name tells whether a name is taken, so it is now braked like a login: five changes, then a pause (30 seconds, doubling up to 15 minutes), per account.
- **First-run setup needs a code from Kura's log.** On an installation without accounts, Kura writes a one-time setup code to its log at every start (`docker compose logs kura`); creating the admin asks for it. Before, whoever reached a fresh installation first, for example while its port was open to the internet, became its admin. Wrong codes are braked like logins.

## [1.5.1] - 2026-11-23

### Changed

- The *Forgot password?* page is shorter: just the field for a name or email and *Send link*.
- The "Kura" at the top of every mail (password reset, invite, test mail) is larger.

## [1.5.0] - 2026-11-23

Replaces 1.4.1, which carried these changes under the wrong version number.

### Added

- **Invite by email** (*Settings → Administration → Users → Invite by email*): Kura mails a link; with it, the invited person chooses their own name and password. Email and role come from the invite. A link works once and for 7 days; *Revoke* ends it early, and so does deleting the admin who sent it. Needs a mail server; without one the button says so.
- **Admins set a password** for someone (*Set password* on the user): the account is logged out everywhere and its API keys are deleted. Only when logged in with a password, not by API key.

### Changed

- **Modals instead of banners and dropdowns.** The one-time email question, *Add user*, *Invite by email*, *Set password* and every "Are you sure?" open as a centred card over the page, which is dimmed and blurred behind it.
- **The reset mail is laid out like a classic account mail**: Kura wordmark, greeting, a short explanation with the link's validity in bold, one centred button, the address written out below for when the button does not work, and a footer, all in Kura's dark colours. The test mail and the new invite mail look the same.
- **Buttons follow Kura's colours**: saving and creating are indigo, deleting is red and always asks first (users, API keys, invites, switching mail off).

### Security

- **Nobody signs up on their own.** Accounts come only from the first-run setup (while no account exists), from an admin, or from an admin's invite. `/register` without a valid invite creates nothing; guessing invite links is braked per address like reset requests.

## [1.4.0] - 2026-11-23

### Added

- **Password reset by mail.** With a mail server set up, the login page offers *Forgot password?*: enter a name or email, and a link to choose a new password goes to the account's email. The answer is the same whether or not the account exists. A link works once and for 30 minutes; using it logs the account out everywhere and deletes its API keys. Only the SHA-256 of a link is stored, and the token sits after `#`, so it never reaches the server's or a proxy's logs.
- **Mail server for admins** (*Settings → Administration → Mail*): server, port, STARTTLS / TLS / none, user name, password or app token, sender (e.g. `Kura <noreply@…>`) and Kura's own address for links. One click fills in Proton Mail's SMTP submission. The password is never shown again or exported; *Send a test mail* checks it all.
- **Mails in Kura's design**: dark, with the indigo button, and a plain-text part for mail programs without HTML.
- **Asked once for an email.** After logging in, an account without an email address sees a short card above the page: add one for password reset, or *Not now*. It is asked once per account, new or existing, and never again after either answer, on any device. Right after logging in, no password is needed to save it; later the card asks for it.

### Changed

- **Administration has parts**: *Users*, *Mail*, *Configuration* (export / import) and *Network* (IP check), listed under it in the Settings menu. User management moved there from *Settings → Users*: each user opens to set their email or delete them, and *Add a user* takes an optional email.
- *Settings → Users* is now *Settings → Account* for everyone (the old link still works).
- Changing your email needs your current password (not in the first 10 minutes after logging in), and API keys cannot change it: the email decides where reset links go.
- The update notice: a pre-release install is told about its release; pre-releases are never offered as an update.

### Security

Found in the security audit of the new reset flow before release, and fixed:

- **A leaked API key could take over its account**: set the account's email to its own address, ask for a reset link and choose a new password. Keys can no longer change the email, and a session needs the current password for it (or a login from the last 10 minutes).
- **The stored mail password went along to any new mail server**, so an admin could have it sent to a server of their own. It is now kept only while server, port, encryption and user name stay the same.
- **Reset requests for existing accounts took slightly longer** (about 1.6 ms), which could tell which names exist. All work for an account now happens after the answer.

## [1.3.5] - 2026-11-23

### Fixed

- **Calendar:** the weekday buttons are solid again when no background theme runs (and under the frosted glass). Since 1.3.4 they had no fill, so the days scrolled visibly through them. Weekdays without episodes are shown in a fainter text colour instead of being see-through.

### Security

Findings of a security review, fixed. Nothing here needs an admin to act, but two changes are visible: **changing your password now deletes your API keys** (Musubi needs a new one), and the version shows in the footer only once logged in.

- **No more changes from other websites (CSRF).** The session cookie is `SameSite=Strict`, which stops other sites but not a neighbouring subdomain (e.g. another app under the same domain). A page there could make a logged-in admin's browser accept or reject requests, cancel or clear downloads, start Auto-Sync or upload a song. Kura now refuses any change that the browser marks as coming from another page (`Sec-Fetch-Site`, or `Origin` on older browsers). API keys and programs are not affected.
- **API keys end with the password.** A new password logs out other devices, as before, and now also deletes the account's API keys, so someone who got hold of the account cannot stay in through a key.
- **A key cannot make keys.** Creating or deleting API keys and changing the password need a login with the password; an API key gets `403`. Before, a leaked key could make itself a spare that survived deleting it.
- **Security headers.** Every answer now carries a Content Security Policy (only Kura's own scripts, no framing), `X-Frame-Options: DENY`, `X-Content-Type-Options: nosniff` and `Referrer-Policy: no-referrer`; over HTTPS (as a trusted proxy reports it) also `Strict-Transport-Security`.
- **Downloads cannot reach the local network.** Kura checked the stream address before handing it to ffmpeg, but ffmpeg then followed redirects and the parts of an HLS playlist on its own, so a hoster could point it at a machine in your network. ffmpeg and ffprobe now go through a small relay inside Kura that resolves every address itself and only connects to the internet; a proxy set in the environment (`HTTPS_PROXY` …) is still used behind it.
- **No rescan flood.** Checking which episodes of a title are downloaded read every library folder again on every call, so any user could slow Kura down by asking in a loop. The libraries are now read again at most every 30 seconds for this.
- **Version only for logged-in users.** `/status` no longer tells visitors who are not logged in which version and commit runs (`version`, `commit` and `latest_version` are `null` for them).

## [1.3.4] - 2026-11-21

### Changed

- **Readable over the ASCII background, without boxes.** The plain plates behind text from 1.3.3 are gone. By default a sheet of frosted, "liquid" glass lies over the whole window below the top bar: the picture behind it is softened and dimmed, with a faint light running across, and it has no edge anywhere, so text, lists and buttons sit on it freely.
- **Choice for admins:** *Settings → Theme → Readability over the background* switches every account between *Frosted glass* and *Text halo*. With the halo, the picture stays sharp, letters get a tight dark glow like subtitles over video, and buttons and chips a plain fill.
- **Less indigo:** the current page in the top bar, the picked site on Home (and the queue filters) and the current Settings section are grey again, as before 1.3.3. Buttons, checkboxes, sliders, focus, progress and today in the calendar keep the accent.
- The weekday bar in the calendar has no strip behind it while the days scroll by.
- A calendar card with many languages moves its time to a second line instead of past the card's edge.

## [1.3.3] - 2026-11-21

### Changed

- **One accent colour.** Kura now follows the 60-30-10 rule: mostly the dark background, then surfaces, lines and text, and about 10 % indigo (`#8b7cff`). Indigo marks only what acts or is chosen: primary buttons, the current page in the top bar and in Settings, the picked site, genre or weekday, checkboxes and sliders, the keyboard focus and download progress. Green, amber, red and the blue *Update* label keep meaning state. **Musubi mirrors Kura's colours and should follow.**
- **Better contrast.** The faintest text colour (dates, hints, the footer) is brighter and now reads at WCAG AA level (at least 4.5:1) on every surface, also on a hovered row.
- **Readable over the ASCII background.** While it runs, page titles, headings, notes, buttons, menus, list rows and card titles sit on plain plates, so nothing has to be read across the picture. Without the background nothing changes.
- **Calendar:** titles stay on one line; a long one fades out on the right, and its full name shows on hover.
- A clearer keyboard focus in the accent colour, one size step for small text instead of loose sizes, spacing on the 4/8-pixel grid, and more room above section headings.

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

[Unreleased]: https://github.com/Koxility/kura/compare/v1.5.3...HEAD
[1.5.3]: https://github.com/Koxility/kura/compare/v1.5.2...v1.5.3
[1.5.2]: https://github.com/Koxility/kura/compare/v1.5.1...v1.5.2
[1.5.1]: https://github.com/Koxility/kura/compare/v1.5.0...v1.5.1
[1.5.0]: https://github.com/Koxility/kura/compare/v1.4.0...v1.5.0
[1.4.0]: https://github.com/Koxility/kura/compare/v1.3.5...v1.4.0
[1.3.5]: https://github.com/Koxility/kura/compare/v1.3.4...v1.3.5
[1.3.4]: https://github.com/Koxility/kura/compare/v1.3.3...v1.3.4
[1.3.3]: https://github.com/Koxility/kura/compare/v1.3.2...v1.3.3
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
