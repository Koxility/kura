# Kura

Kura (蔵, "storehouse") is a self-hosted web app that downloads series and anime from AniWorld and SerienStream into a clean, Jellyfin-ready library on your own disk.

## Features

- **Browse** AniWorld and SerienStream from one interface: search, genres, new and popular titles, a random pick.
- **Download** single episodes, whole seasons or entire series, several at a time, with live progress.
- **Choose language and provider** per download, or let Kura fall back through your preferred order (German dub, German sub, English sub; VOE, Vidmoly).
- **Libraries**: named folders with a default per site, laid out the way Jellyfin expects, IMDb ID included.
- **Auto-Sync**: checks downloaded series for new episodes on a schedule.
- **Users and requests**: normal users request titles, admins accept or reject them.
- **JSON API** for other tools, the same one the web app uses.

## Requirements

- Linux (other systems are untested)
- [ffmpeg](https://ffmpeg.org) and `ffprobe` on the `PATH`
- To build: Rust 1.88 or newer and [Bun](https://bun.sh)

A Docker image is planned; until then Kura runs from source.

## Running Kura

```bash
git clone https://github.com/Koxility/kura.git
cd kura/web && bun install && bun run build && cd ..
cargo build --release
./target/release/kura
```

Open `http://127.0.0.1:8787`. On first start Kura asks for an admin account, then for a first library: a folder downloads go to.

| Variable | Default | Meaning |
|---|---|---|
| `KURA_LISTEN` | `127.0.0.1:8787` | Address and port. Use `0.0.0.0:8787` to reach Kura from other machines. |
| `KURA_DATA` | `data` | Folder for the database. |

Everything else is set in the web app under Settings and stored in the database.

## Folder layout

Inside every library Kura writes

```
Series/
  One Piece [imdbid-tt0388629]/
    Season 01/
      One Piece - S01E01.mkv
```

Point a Jellyfin "Shows" library at the `Series` folder. The IMDb ID comes from the site; when the site has none, Kura asks for it before downloading, and `[imdbid-none]` is used if you skip.

## Users and requests

| | Admin | User |
|---|---|---|
| Browse and search | yes | yes |
| Queue | view and manage | view |
| Download | yes | request only |
| Requests | accept or reject | own requests |
| Library, Auto-Sync, Settings | yes | no |

Accepted requests are queued into the site's default library.

## API

The web app talks to a JSON API under `/api/v1`, authenticated with the session cookie from `POST /api/v1/login`. The main endpoints:

| Endpoint | Purpose |
|---|---|
| `GET /search?q=…&site=…` | Search one or all sites |
| `GET /series/{site}/{slug}` | A title with its seasons |
| `GET /series/{site}/{slug}/seasons/{n}` | Episodes of a season |
| `GET /queue`, `POST /queue` | The queue; queue episodes (admin) |
| `GET /requests`, `POST /requests` | Requests |
| `GET /events` | Live progress as Server-Sent Events |

`crates/kura/src/api.rs` lists every route.

## Development

```bash
cargo test --workspace
```

```bash
cd web && bun run dev
```

`bun run dev` serves the web app with hot reload and passes `/api` on to a server started with `cargo run -p kura`. How the crates fit together is described in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

Tests that talk to the real sites and hosters are skipped by default:

```bash
cargo test -p kura-sites -p kura-hosters -- --ignored
```

Before every commit:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Legal

Kura hosts and distributes no content. Whether downloading from the supported sites is allowed depends on where you live and on the content; you are responsible for how you use it.

## License

[MIT](LICENSE)
