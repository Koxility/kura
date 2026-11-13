# Requirements

## With Docker (recommended)

- A Linux machine with [Docker](https://docs.docker.com/engine/install/) and the Compose plugin (`docker compose`).
- `amd64` or `arm64` (Raspberry Pi 4/5, most ARM NAS systems).
- Enough disk space for your library. A typical episode is 200–600 MB.
- Outgoing internet access to AniWorld, SerienStream and the video providers (VOE, Vidmoly).

Everything else, including ffmpeg, is inside the image.

## From source

- Linux (other systems are untested).
- [Rust](https://rustup.rs) 1.88 or newer.
- [Bun](https://bun.sh) to build the web app.
- [ffmpeg](https://ffmpeg.org) with `ffprobe` on the `PATH`.

## Browser

Any current browser. Kura is a web app; there is nothing to install on the devices you use it from.
