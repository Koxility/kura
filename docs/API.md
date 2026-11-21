# Kura API

The web app and other programs, such as [Musubi](https://github.com/Koxility/musubi), use the same JSON API. Everything lives under `/api/v1`; a change that breaks a client gets a new version prefix, while new fields and endpoints may appear in `v1` at any time, so clients ignore what they do not know.

## Authentication

- **Web app**: the `kura_session` cookie from `POST /login`.
- **Other programs**: an API key, created under *Settings → User → API keys*, sent as

  ```
  Authorization: Bearer kura_…
  ```

A key has the rights of the account it belongs to: an admin's key can queue downloads, a user's key can send requests. Deleting the key, or its account, ends its access at once. A request with an unknown key is answered `401`.

## Errors

Failures answer with a status code and `{ "error": "text for a person" }`: `400` for input to fix, `401` not logged in, `403` not allowed, `404` unknown, `409` conflict (e.g. a taken name), `502` a site or Jellyfin did not answer.

## Endpoints a client needs

Sites are `aniworld` or `serienstream`; languages are `german_dub`, `german_sub`, `english_sub`, `english`. Times are Unix seconds unless noted.

| Method and path | Who | What |
|---|---|---|
| `GET /status` | anyone | Version, `commit`, the caller (`user`, or `null`), enabled `sites`, `defaults` |
| `GET /account` | user | Own name, role, email, theme |
| `GET /search?q=…&site=…` | user | Titles matching the text, on one site or all |
| `GET /new/{site}` · `GET /popular/{site}` | user | Title lists |
| `GET /new-episodes/{site}` | user | Newest episodes (AniWorld; empty elsewhere): `slug`, `title`, `season` (0 = films), `number`, `language`, `released` as the site shows it |
| `GET /calendar/{site}` | user | Days around today (AniWorld; empty elsewhere): `date`, `offset` (0 = today), `entries` with `time`, `title`, `season`, `number`, `language`, `online` |
| `GET /series/{site}/{slug}` | user | Title, description, `seasons`, `imdb_id` |
| `GET /series/{site}/{slug}/seasons/{n}` | user | Episodes of a season with the `languages` each is offered in |
| `GET /series/{site}/{slug}/downloaded?title=…&imdb=…` | user | `[season, episode]` pairs already in a library |
| `GET /queue` | user | Downloads with their state and progress |
| `POST /queue` | admin | Queue: `{ site, slug, all \| seasons \| episodes, language?, hoster?, library?, imdb_id? }`; answers `{ queued }` |
| `GET /requests` · `POST /requests` | user | Own requests; a user asks with the same body as `POST /queue` |
| `GET /events` | user | Server-sent events: `queue_changed`, `requests_changed`, `progress` |

To find the Kura title for a Jellyfin series, search by its name and compare `imdb_id` from the title page with the series' IMDb ID; Kura also puts `[imdbid-tt…]` into every series folder it creates.
