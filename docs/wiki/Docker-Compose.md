# Docker Compose

The [`docker-compose.yaml`](https://github.com/Koxility/kura/blob/master/docker-compose.yaml) in the repository is ready to use. This page explains it line by line.

```yaml
services:
  kura:
    image: ghcr.io/koxility/kura:latest
    container_name: kura
    restart: unless-stopped
    user: "1000:1000"
    ports:
      - "8787:8787"
    environment:
      TZ: Europe/Berlin
      # KURA_NAMING_TEMPLATE: "{title} {episodenumber} {language}"
    volumes:
      - ./data:/data
      - /path/to/media:/media
```

## image

`latest` always follows the newest release. To update only when you choose, pin a version: `ghcr.io/koxility/kura:1.1.0`, or `:1.1` to get bug-fix releases of 1.1 only. See [[Updating]].

## restart

`unless-stopped` starts Kura again after a reboot or a crash, unless you stopped it yourself.

## user

The user and group id Kura runs as inside the container. Every file Kura writes, on both volumes, belongs to them. Set them to the host user that should own your media:

```bash
id -u   # e.g. 1000
id -g   # e.g. 1000
```

Both mapped folders must be writable for this user. If they are not, Kura cannot save its database or downloads; see [[Troubleshooting]].

## ports

`"8787:8787"` means host port 8787 → container port 8787. To use another port on the host, change only the left side, e.g. `"9000:8787"`.

Kura has no HTTPS of its own. To reach it from outside your home network, put it behind a reverse proxy (Caddy, Traefik, nginx) that adds HTTPS, or use a VPN. Do not expose port 8787 directly to the internet.

## environment

| Variable | Default in the image | Meaning |
|---|---|---|
| `TZ` | UTC | Time zone, used for the daily Auto-Sync time. |
| `KURA_NAMING_TEMPLATE` | `{title} {episodenumber} {language}` | File name of each episode while Settings → Downloads sets no template. See [[Configuration]]. |
| `KURA_LISTEN` | `0.0.0.0:8787` | Address and port inside the container. Normally left alone. |
| `KURA_DATA` | `/data` | Folder for the database inside the container. Normally left alone. |
| `KURA_LIBRARY_DIR` | `/media` | Folder the first-start library setup suggests. |

## volumes

Each line maps a folder on the host (left) to a folder in the container (right).

### `./data:/data`

The database: accounts, settings, the queue, requests. Small, but important: back it up. `./data` is next to the compose file; any absolute path works too.

### `/path/to/media:/media`

Where your downloads go. Replace `/path/to/media` with the real folder, e.g. `/mnt/nas/Media`.

**Kura only sees container paths.** When you create a library in Kura, you enter the path on the right side: `/media`, or a folder below it like `/media/anime`. Kura creates missing folders.

### More libraries

Add a line per folder and create a library in Kura for each:

```yaml
    volumes:
      - ./data:/data
      - /mnt/nas/Anime:/media/anime
      - /mnt/nas/Serien:/media/serien
```

In Settings → Library: library "Anime" with path `/media/anime`, default for AniWorld; library "Serien" with path `/media/serien`, default for SerienStream.

## Health check

The image checks itself every 30 seconds (`kura healthcheck`). `docker ps` shows `healthy` once Kura answers.
