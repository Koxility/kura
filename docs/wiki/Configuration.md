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

New episodes are queued in the language of the newest episode already downloaded (from Kura's records or the file name, e.g. `German Dub`) and with the provider, library and IMDb ID of the series' last download, into the folder the series already has. An episode that is not offered in that language yet is skipped and queued by a later check once it is. Episodes already in the library are never queued again, also ones Kura did not download. **Remove** next to a tracked title takes it off the list until a download of it starts again. Titles can be left out for good by searching under *Exclude titles* or with the checkbox on a title's page; *Include again* brings them back. **Sync now** on the Auto-Sync page runs a check at once.

## Settings → Jellyfin

- **Address**: e.g. `http://192.168.1.10:8096`. Empty switches this off.
- **API key**: created in Jellyfin under *Dashboard → API Keys*.
- **Test connection** checks both before you save.

When the queue is empty (nothing queued, nothing downloading), Kura asks Jellyfin once to scan the series folders that got new episodes: a finished season, a batch from Auto-Sync or a single episode. Only those folders are scanned, never the whole library, and several downloads in a row cause one scan, not one each. A failed download stays in the queue and does not hold the scan back. If Jellyfin mounts a library under another path, set **Path in Jellyfin** for that library under *Settings → Library*.

## Settings → Users / User

Every account has **Your account**: change the name, the password (the current one is needed; other devices are logged out) and an optional email address, which is stored but not used yet. **API keys** let other programs, such as Musubi, use Kura instead of logging in; a key is shown once when it is created and can do what the account can. Admins see the list of all users below that; normal users only see their own account.

## Settings → Theme

Per account:

- **Wii Shop Channel Main Theme** (on by default): plays on a loop while Kura is open. Kura ships without the song; an admin uploads an MP3 here (at most 20 MB), which stays in the data folder. Browsers start sound only after the first click or key on the page.
- **ASCII background** (off by default): a faint animated scene from [ascii.rest](https://ascii.rest) behind the pages. Pick the scene below it.

## Settings → Users (admins)

Add and remove accounts. See [[Roles and Requests|Roles-and-Requests]].

## Settings → Administration

- **Export / Import**: all settings, libraries and Auto-Sync exclusions as one JSON file. Accounts and passwords are not included.
- **IP check**: shows the public address the server reaches the internet from, e.g. to confirm a VPN. It asks api.ipify.org, only when you press the button.
