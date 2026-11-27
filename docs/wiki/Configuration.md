# Configuration

Kura is configured in two places: a few environment variables for where it runs, and everything else in the web app under **Settings**, stored in the database.

## Environment variables

| Variable | Default (Docker / source) | Meaning |
|---|---|---|
| `KURA_LISTEN` | `0.0.0.0:8787` / `127.0.0.1:8787` | Address and port to listen on. |
| `KURA_DATA` | `/data` / `data` | Folder for the database. |
| `KURA_LIBRARY_DIR` | `/media` / `./downloads` | Folder the library setup suggests. |
| `KURA_NAMING_TEMPLATE` | `{title} {episodenumber} {language}` | File name template while the settings set none. |
| `KURA_TRUSTED_PROXIES` | none | Reverse proxies whose `X-Forwarded-For` Kura believes, e.g. `172.17.0.1` or `10.0.0.0/24, ::1`. See [[Docker Compose|Docker-Compose]]. |
| `TZ` | UTC | Time zone for the daily Auto-Sync time. |

## Settings → Downloads

### Naming template

Controls the file name of every new episode. Variables:

| Variable | Example | |
|---|---|---|
| `{title}` | `One Piece` | Title of the series |
| `{episodenumber}` | `S01E001` | Season with 2 digits, episode with at least 3 |
| `{language}` | `German Dub` | Language actually downloaded: `German Dub`, `German Sub`, `English Sub`, `English` |
| `{season}` | `01` | Season alone |
| `{episode}` | `001` | Episode alone |
| `{imdbid}` | `tt0388629` | IMDb ID, or `none` |

Examples, for season 1 episode 1 of One Piece in German dub:

| Template | File |
|---|---|
| `{title} {episodenumber} {language}` (default) | `One Piece S01E001 German Dub.mkv` |
| `{title} - {episodenumber}` | `One Piece - S01E001.mkv` |
| `{episodenumber} {title} [{language}]` | `S01E001 One Piece [German Dub].mkv` |
| `{title} {season}x{episode}` | `One Piece 01x001.mkv` |

Rules:

- The template needs `{episodenumber}`, or both `{season}` and `{episode}`, so no two episodes get the same name.
- Characters that are not allowed in file names (`/ \ : * ? " < > |`) become spaces.
- The extension (`.mkv`, `.mp4`) is added from *Output format*.
- Which template applies: the one in Settings, else `KURA_NAMING_TEMPLATE`, else the built-in default. Leave the field empty to use the default; the field shows it as a grey placeholder.
- Changing the template affects new downloads only. Existing files keep their names.

The folders above the file are fixed; see [[Folder Structure|Folder-Structure]].

### Output format

`mkv` or `mp4`. The stream is copied as it is, without re-encoding, so both are fast and lossless.

### Downloads at the same time

1 to 16, default 4.

### Provider fallback order and language order

When a download keeps the default language and provider (the first choice on a title's page), Kura tries the languages from top to bottom, and within a language the providers from top to bottom, until one works. Supported providers: VOE and Vidmoly. Auto-Sync always follows these orders.

## Settings → Library

Libraries are named folders downloads go to. Each can be the default for one or more sites; a site has at most one default. The first library is the *main library*, used when nothing else applies. With Docker, paths are container paths ([[Docker Compose|Docker-Compose]]). **Path in Jellyfin** is the same folder as Jellyfin sees it, e.g. `/data/tvshows` when Kura writes to `/media`; leave it empty when both see the same path.

## Settings → Sites

Switch AniWorld or SerienStream off to hide them everywhere, including Auto-Sync.

## Settings → Auto-Sync

- **Enable Auto-Sync**: checks every series for new episodes once a download of it has started.
- **Only new episodes**: on, only episodes after the newest downloaded one; off, missing earlier episodes are downloaded too.
- **Schedule**: every N minutes (5 to 10080) or every day at a fixed time (in `TZ`).

Each check reads every tracked series from the site and decides about each listed episode:

- **Language.** A series follows one language: of the languages it already has, on disk or downloaded before, the one ranked highest in the language order under *Settings → Downloads*. The language comes from Kura's records or the file name, e.g. `German Dub`. New episodes are queued in exactly that language, with the provider, library and IMDb ID of the series' last download, into the folder the series already has.
- **Already there.** An episode counts as downloaded only while a file of it is in the library in that language, or in a file whose name tells no language. Files Kura did not download count too. A download on record in another language, or one whose file is gone, does not count.
- **Waiting.** An episode that is not offered in that language yet is queued by a later check once it is. The Auto-Sync page lists it with the languages it is offered in.
- **Failed downloads** are queued again on every check. Cancelled ones are left alone, and so is an episode deleted after it was downloaded in the series' language.
- **Better language.** An episode on disk only in a language ranked lower (e.g. German Sub while the series follows German Dub) is downloaded again once the better language is out. When that download is done, the old file is deleted, with its `.nfo` and thumbnail (`-thumb.jpg` and the like), before Jellyfin is asked to scan, so Jellyfin reads the episode afresh.
- **Only new episodes** counts from the newest episode downloaded in the series' language, also after its file was deleted, so watching and deleting does not stop a series.

A season page that lists no episodes, or that cannot be read twice in a row, makes the check fail instead of passing as "Nothing new"; the other seasons are still checked. The Auto-Sync page shows, per series, its language, the episodes on disk in that language, the newest episode the site lists, what the last check queued or waits for, and when it ran. This, and the time of the last run, is kept over a restart. **Sync now** on the Auto-Sync page runs a check at once; while a check is running, it waits for that one instead of starting a second.

**Remove** next to a tracked title takes it off the list until a download of it starts again. Titles can be left out for good by searching under *Exclude titles* or with the checkbox on a title's page; *Include again* brings them back.

## Settings → Jellyfin

- **Address**: e.g. `http://192.168.1.10:8096`. Empty switches this off.
- **API key**: created in Jellyfin under *Dashboard → API Keys*. Once saved, Kura never shows it again, also not in the exported configuration; leave the field empty to keep it. It is kept only while the address stays the same, so a new address needs the key again.
- **Test connection** checks both before you save.

When the queue is empty (nothing queued, nothing downloading), Kura asks Jellyfin once to scan the series folders that got new episodes: a finished season, a batch from Auto-Sync or a single episode. Only those folders are scanned, never the whole library, and several downloads in a row cause one scan, not one each. A failed download stays in the queue and does not hold the scan back. If Jellyfin mounts a library under another path, set **Path in Jellyfin** for that library under *Settings → Library*.

## Settings → Account

Every account changes its name (five times in a row, then it has to wait a little), its password (the current one is needed; other devices are logged out and the account's API keys are deleted) and an optional email address, which is where a password reset link goes; changing it also needs the current password, except in the first 10 minutes after logging in. A new address counts once the link Kura mails to it is opened (it works once and for 24 hours); until then reset links keep going to the old one, and without a mail server no new address can be added. Every address belongs to one account, and to an open invite, whatever the case of its letters: `Sam@Example.org` and `sam@example.org` are the same. An address an admin sets under *Users* counts at once. After logging in, an account without an email is asked once for one while mail is set up (*Not now* ends it for good). **API keys** let other programs, such as Musubi, use Kura instead of logging in; a key is shown once when it is created and can do what the account can, except change the password, the email or the keys.

## Settings → Theme

Per account:

- **Background music** (on by default): plays the chosen **Song** on a loop at the chosen **Volume** while Kura is open. Kura ships without songs; admins add MP3s here (at most 20 MB each, named after the file), which stay in the data folder, and every account picks from them. Browsers start sound only after the first click or key on the page.
- **ASCII background** (off by default): a faint animated scene from [ascii.rest](https://ascii.rest) behind the pages, picked from your own list. Paste the link of any piece (e.g. `https://ascii.rest/aurora-fjord/`) to add it; pieces with colour keep it, washed out. Every piece ships with Kura, so nothing is loaded from ascii.rest.
- **Readability over the background** (admins, for all accounts): *Frosted glass* (default) softens and dims the picture behind a sheet of glass; *Text halo* keeps the picture sharp and gives letters a dark glow.

## Settings → Administration (admins)

The menu opens to four parts:

- **Users**: every account with its role and email. Click one to set its email (for password reset), **Set password** for it (it is logged out everywhere, its API keys are deleted) or **Delete user** (asks first). **Add user** opens a form for name, optional email, password and role. **Invite by email** sends a link with which the person chooses their own name and password; email and role come from the invite, the link works once and for 7 days, and open invites are listed with *Revoke*. Nobody can sign up without an admin. See [[Roles and Requests|Roles-and-Requests]].
- **Mail**: the mail server Kura sends password reset links through. Until one is set up, the login page offers no *Forgot password?* (see below).
- **Configuration**: export / import of all settings, libraries and Auto-Sync exclusions as one JSON file. Accounts, passwords and the mail server are not included.
- **Network**: shows the public address the server reaches the internet from, e.g. to confirm a VPN. It asks api.ipify.org, only when you press the button.

### Mail server

| Field | Example | Notes |
|---|---|---|
| Mail server | `smtp.example.org` | The name only, no `smtp://`. |
| Port | `587` | 587 with STARTTLS (the usual), 465 with TLS, 25 for a relay in your own network. |
| Encryption | STARTTLS | STARTTLS is required, not optional. *None* sends no password, so it only works with a relay that asks for none. Certificates are always checked. |
| User name / Password | `noreply@example.org` / app password | The password is stored but never shown again; leave the field empty to keep it. It is kept only while server, port, encryption and user name stay the same. |
| Sender | `Kura <noreply@example.org>` | The *From* of every mail. |
| Kura's address | `https://kura.example.org` | Links in mails start with it, so it must be the address people open Kura under. *Use this one* fills in the address of the open page. |

**Proton Mail** (paid plans with SMTP submission): press *Proton Mail* to fill in `smtp.protonmail.ch`, port 587, STARTTLS. Create an SMTP token in Proton under *Settings → IMAP/SMTP → SMTP tokens*; the address it belongs to is the user name, the token the password, and the sender must be that address.

**Send a test mail** (shown once the mail server is saved) sends a mail in Kura's design and shows the mail server's answer if it fails. *Switch mail off* forgets the mail server and its password.

### Password reset

With a mail server set up, the login page shows **Forgot password?**. Anyone enters a name or an email address; if an account with an email matches, a link is sent there. The page answers the same way whether or not the account exists. The link works once and for 30 minutes; a new link for the same account is sent at most every 2 minutes, and the same name or email may be asked for 5 times from one address before that address has to wait for it; other names are not held up. Setting a new password with it logs the account out everywhere, deletes its API keys and ends other links.
