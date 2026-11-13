<div align="center">

# Kura

**Download anime and series from AniWorld and SerienStream. Your library stays Jellyfin-ready.**

[![Latest release](https://img.shields.io/github/v/release/Koxility/kura?label=release&color=3ecf8e)](https://github.com/Koxility/kura/releases/latest)
[![License: MIT](https://img.shields.io/badge/license-MIT-f5f5f5)](LICENSE)
[![Docker image](https://img.shields.io/badge/docker-ghcr.io%2Fkoxility%2Fkura-a1a1a1)](https://github.com/Koxility/kura/pkgs/container/kura)
![Rust 1.88+](https://img.shields.io/badge/rust-1.88%2B-a1a1a1)

[Features](#features) · [Installation](#installation) · [Getting started](#getting-started) · [Configuration](#configuration) · [API](#api) · [Documentation](#documentation)

</div>

Kura (蔵, "storehouse") is a self-hosted web app. You browse AniWorld and SerienStream in one clean interface, pick what you want, and Kura downloads it into a library on your own disk, laid out the way Jellyfin reads it.

## Features

**Browse** — both sites in one place.

- A home page per site with search, genres that keep loading as you scroll, new and popular titles, and a random pick.
- A page per title with its seasons as a tree: open a season, tick episodes or whole seasons, download the selection or everything.

**Download** — reliable, several at a time.

- Language and provider per download, or Kura's fallback through your preferred order (German dub, German sub, English sub; VOE, Vidmoly).
- A queue with live progress, one entry per title ("2 of 4 episodes downloaded"), filters, search and sort.
- mkv or mp4, copied by ffmpeg without re-encoding.

**Keep** — a tidy library that stays current.

- Libraries: named folders with a default per site, with the IMDb ID in every series folder.
- Auto-Sync checks downloaded series for new episodes on a schedule, or right now with one click.

**Share** — one server for the whole household.

- Local accounts. Normal users request titles; admins accept or reject them, and accepted requests are queued.
- A JSON API for other tools, the same one the web app uses.

## Requirements

- Linux with [Docker](https://docs.docker.com/engine/install/) and Compose, on amd64 or arm64
- Or, to run from source: Rust 1.88+, [Bun](https://bun.sh) and [ffmpeg](https://ffmpeg.org)

## Installation

Download the [`docker-compose.yaml`](docker-compose.yaml), set `user:`, the media folder and `TZ`, and start Kura:

```bash
curl -LO https://raw.githubusercontent.com/Koxility/kura/master/docker-compose.yaml
```

```bash
docker compose up -d
```

```yaml
services:
  kura:
    image: ghcr.io/koxility/kura:latest
    restart: unless-stopped
    user: "1000:1000"
    ports:
      - "8787:8787"
    environment:
      TZ: Europe/Berlin
    volumes:
      - ./data:/data              # database and settings
      - /path/to/media:/media     # your library
```

To update, run `docker compose pull && docker compose up -d`. The top bar shows a blue **Update** label when a new release is out.

[Docker Compose](https://github.com/Koxility/kura/wiki/Docker-Compose) in the wiki explains every line, and [Installation](https://github.com/Koxility/kura/wiki/Installation) covers running from source.

## Getting started

1. Open `http://<your server>:8787` and create the admin account.
2. Set up the first library: a name and a folder. With Docker this is a path inside the container, like `/media`. Kura creates the folder if it does not exist.
3. Search for a title on Home, open it, pick episodes or seasons and press Download.
4. Add more users under Settings → Users. They can browse and request; you decide in Request.

## Using Kura

| Page | Admin | User |
|---|---|---|
| Home | Browse, search, download | Browse, search, request |
| Library | The libraries as a folder tree | – |
| Auto-Sync | Tracked titles, Sync now, exclusions | – |
| Queue | Watch, cancel, retry, clear | Watch |
| Request | Accept or reject | Own requests and their status |
| Settings | Downloads, libraries, sites, Auto-Sync, users, administration | – |

Inside every library Kura writes

```
Series/
  One Piece [imdbid-tt0388629]/
    Season 01/
      One Piece S01E001 German Dub.mkv
```

The file name follows a template you set under Settings → Downloads (or with `KURA_NAMING_TEMPLATE`), with `{title}`, `{episodenumber}`, `{language}`, `{season}`, `{episode}` and `{imdbid}`.

Point a Jellyfin "Shows" library at the `Series` folder. The IMDb ID comes from the site; when the site has none, Kura asks for it before downloading, and `[imdbid-none]` is used if you skip.

## Configuration

| Variable | Default (Docker) | Meaning |
|---|---|---|
| `TZ` | UTC | Time zone for the daily Auto-Sync time. |
| `KURA_NAMING_TEMPLATE` | `{title} {episodenumber} {language}` | Episode file name while Settings sets none. |
| `KURA_LISTEN` | `0.0.0.0:8787` | Address and port. |
| `KURA_DATA` | `/data` | Folder for the database. |
| `KURA_LIBRARY_DIR` | `/media` | Folder the library setup suggests. |

Everything else is set in the web app under Settings and stored in the database, so it survives restarts and updates. [Configuration](https://github.com/Koxility/kura/wiki/Configuration) in the wiki describes every setting.

## API

The web app talks to a JSON API under `/api/v1`, authenticated with the session cookie from `POST /api/v1/login`.

| Endpoint | Purpose |
|---|---|
| `GET /search?q=…&site=…` | Search one or all sites |
| `GET /series/{site}/{slug}` | A title with its seasons |
| `GET /series/{site}/{slug}/seasons/{n}` | Episodes of a season |
| `GET /queue`, `POST /queue` | The queue; queue episodes (admin) |
| `GET /requests`, `POST /requests` | Requests |
| `GET /events` | Live progress as Server-Sent Events |

[`crates/kura/src/api.rs`](crates/kura/src/api.rs) lists every route.

## Documentation

- The [wiki](https://github.com/Koxility/kura/wiki): installation, Docker Compose, configuration, updating, folder structure, roles, troubleshooting.
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md): how sites, providers, the downloader and the server fit together.
- [CHANGELOG.md](CHANGELOG.md): what changed in each release.

## Contributing

Bug reports and ideas are welcome in the [issue tracker](https://github.com/Koxility/kura/issues). For code changes, `bun run dev` in `web/` serves the web app with hot reload and passes `/api` on to a server started with `cargo run -p kura`. These three have to pass before a pull request:

```bash
cargo fmt --all --check
```

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

```bash
cargo test --workspace
```

Tests that talk to the real sites and providers are skipped by default:

```bash
cargo test -p kura-sites -p kura-hosters -- --ignored
```

## License

Kura is free software, licensed under the [MIT License](LICENSE).

## Legal disclaimer

Kura is a client-side tool. It reads publicly reachable pages of the sites you choose and passes the video links found there to ffmpeg, which saves them to storage you control. It does not host, provide, index or distribute any media, and the project runs no servers of its own.

You are responsible for how you use it, for what you download, and for following the laws and terms of service that apply where you live. In many countries, including Germany, downloading copyrighted works from sources that are not licensed to offer them is not allowed. The project is provided "as is", without warranty of any kind. Its maintainers are not responsible for third-party content, external links, or the availability, accuracy, legality or reliability of outside services.

Kura is an independent project. It is not affiliated with, endorsed by or sponsored by AniWorld, SerienStream, any video provider, IMDb or Jellyfin; their names and trademarks belong to their respective owners. Questions about content hosted by another service should be directed to that service.
