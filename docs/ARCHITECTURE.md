# Architecture

AniWorld and SerienStream host no video. They list series and episodes and link to external hosters, which serve the actual stream. Kura keeps these three stages apart.

```
 search / browse / series page / episode page
        │
        ▼
 ┌─────────────┐   episode → list of (language, hoster, link)
 │ kura-sites  │   AniWorld, SerienStream
 └─────────────┘
        │  hoster page URL
        ▼
 ┌──────────────┐  hoster page → direct stream URL (HLS playlist or MP4) and required headers
 │ kura-hosters │  VOE, Vidoza, Filemoon, Doodstream, ...
 └──────────────┘
        │  stream URL
        ▼
 ┌───────────────┐ stream URL → file on disk, progress events
 │ kura-download │ ffmpeg subprocess
 └───────────────┘
```

## Crates

| Crate | Job |
|---|---|
| `kura-domain` | Plain types shared by all crates. No I/O. |
| `kura-sites` | One module per site. Knows hosters only by name and link. |
| `kura-hosters` | One module per hoster. Knows nothing about sites. |
| `kura-download` | Runs ffmpeg on a stream URL. Knows nothing about sites or hosters. ffmpeg's connections go through the relay in `kura-sites` (`relay.rs`), which only connects to public addresses, so a redirect or a playlist part cannot reach the local network. |
| `kura-store` | SQLite: users, sessions, settings, download queue, requests. |
| `kura-core` | Joins the crates above: search, fallback, queue, workers, events. |
| `kura` | The binary: HTTP routes, authentication, live events, embedded frontend. |

Dependencies point downwards only. The binary talks to `kura-core` and holds no logic of its own, so another front end (a bot, a CLI) can sit on the same core or on the JSON API.

## Fallback

For each episode, `kura-core` takes the first language from the settings and walks the hoster order until one resolves and downloads. If none does, it moves to the next language. Every attempt is recorded on the job.

## Other programs

Other programs, such as [Musubi](https://github.com/Koxility/musubi), use the same JSON API as the web app, with a per-user API key instead of the session cookie. [API.md](API.md) lists what a client needs. A key has the rights of its user, no more, and cannot change the password or the keys.

## Requests and roles

Admins queue downloads directly; normal users send requests, which an admin accepts or rejects. Accepting runs the same queueing as a direct download. The rights are checked in the HTTP layer (`Admin` and `Session` extractors in `crates/kura/src/api.rs`), not only hidden in the web app.

## Folder names

`target_path` in `kura-core` builds `<library>/<Title> [imdbid-<id>]/Season XX/`, or reuses the folder an earlier episode of the series went to (also the `Series` folder of Kura 1.0 and 1.1); the file name comes from the naming template (`naming.rs`), rendered per download attempt because it carries the language actually used. Each job stores its IMDb ID, so retries and Auto-Sync keep using the same folder; jobs from before IMDb IDs have none and keep the folder without one.

## Already downloaded

`downloaded.rs` in `kura-core` combines finished downloads from the database with series folders found in the libraries (directly and in a legacy `Series` folder), matched by IMDb ID or by a normalised title, and episode files matched by `SxxEyy` in their names. Lists from the sites get a `downloaded` flag; title pages ask for the downloaded episodes of one series.

## Storage

One SQLite file. Settings are a single JSON row whose missing fields fall back to defaults, so adding a setting needs no migration. Renamed or removed settings are converted when they are read (`upgrade` in `kura-store`). A finished job keeps the path of its file, which makes the queue table the library index as well.

## Frontend

`web/` is a React app built with Vite. All colours, sizes and durations are CSS custom properties in `web/src/styles/tokens.css`; components use those and nothing else.

Colour follows 60-30-10: about 60 % background (`--bg`), 30 % surfaces, lines and text, and 10 % one accent (`--accent`, indigo). The accent only marks what acts or is chosen: the primary button, the current page, a picked option, focus and progress. Green, amber, red and blue stay state colours. Every text colour reaches WCAG AA (4.5:1) on the background, surfaces and hover. While the ASCII background runs, text stays readable in one of two looks the admin picks for everyone (`backdrop` in the settings, `data-backdrop` on the root element): frosted glass over the whole window, or a dark halo around letters with `--control-fill` under controls. Musubi mirrors these tokens.
