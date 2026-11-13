<div align="center">

# Kura

**Download anime and series from AniWorld and SerienStream. Your library stays Jellyfin-ready.**

[![Latest release](https://img.shields.io/github/v/release/Koxility/kura?label=release&color=3ecf8e)](https://github.com/Koxility/kura/releases/latest)
[![License: MIT](https://img.shields.io/badge/license-MIT-f5f5f5)](LICENSE)
![Platform: Linux](https://img.shields.io/badge/platform-Linux-a1a1a1)
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

- Linux (other systems are untested)
- [ffmpeg](https://ffmpeg.org) and `ffprobe` on the `PATH`
- To build: Rust 1.88 or newer and [Bun](https://bun.sh)

## Installation

A Docker image is planned. Until then Kura runs from source:

```bash
git clone https://github.com/Koxility/kura.git
```

```bash
cd kura/web && bun install && bun run build && cd ..
```

```bash
cargo build --release
```

```bash
./target/release/kura
```

The result is a single binary with the web app built in.

## Getting started

1. Open `http://127.0.0.1:8787` and create the admin account.
2. Set up the first library: a name and a folder on the server. Kura creates the folder if it does not exist.
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
      One Piece - S01E01.mkv
```

Point a Jellyfin "Shows" library at the `Series` folder. The IMDb ID comes from the site; when the site has none, Kura asks for it before downloading, and `[imdbid-none]` is used if you skip.

## Configuration

Two environment variables set where Kura runs:

| Variable | Default | Meaning |
|---|---|---|
| `KURA_LISTEN` | `127.0.0.1:8787` | Address and port. Use `0.0.0.0:8787` to reach Kura from other machines. |
| `KURA_DATA` | `data` | Folder for the database. |

Everything else is set in the web app under Settings and stored in the database, so it survives restarts. Settings → Administration exports and imports the configuration as one file; accounts and passwords are not part of it.

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

## Legal disclaimer

Kura is a client-side tool that saves downloads to locations you choose. It does not operate a media-hosting service for third-party sites.

You are responsible for how you use it and for following the laws and terms that apply where you live. The project is provided "as is". Its maintainers are not responsible for third-party content, external links, or the availability, accuracy, legality, or reliability of outside services.

Questions about content hosted by another service should be directed to that service.

Kura is an independent project. It is not affiliated with, endorsed by or sponsored by AniWorld, SerienStream, any video provider, IMDb or Jellyfin; their names and trademarks belong to their respective owners. Questions about content hosted by another service should be directed to that service.

## License

Kura is free software, licensed under the [MIT License](LICENSE).
