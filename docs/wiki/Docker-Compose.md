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

Kura has no HTTPS of its own. To reach it from outside your home network, put it behind a reverse proxy (HAProxy, Caddy, Traefik, nginx) that adds HTTPS, or use a VPN. Do not expose port 8787 directly to the internet.

### Behind a reverse proxy

1. **Only the proxy reaches Kura.** If the proxy runs on the same machine, publish the port on the loopback address only: `"127.0.0.1:8787:8787"`. Otherwise allow port 8787 only from the proxy in the firewall.
2. **The proxy tells Kura who is asking.** Every connection Kura sees comes from the proxy. To slow down wrong logins per person and not for everyone at once, the proxy sends the person's address in `X-Forwarded-For`, and `KURA_TRUSTED_PROXIES` names the proxy's address as Kura sees it. Without it, Kura ignores the header, because anyone can send one. If you don't know the address, log in once: Kura's log then names it (`a request from 172.17.0.1 names the address it was forwarded for…`). Only name addresses nobody else can connect from.
3. **HTTPS only.** Redirect plain HTTP to HTTPS. When the proxy also sends `X-Forwarded-Proto: https`, Kura marks the login cookie `Secure`, so a browser never sends it over plain HTTP.

HAProxy, with Kura on the same machine and port 8787 published as `127.0.0.1:8787:8787`:

```
frontend http
    bind :80
    http-request redirect scheme https

frontend https
    bind :443 ssl crt /etc/haproxy/certs/kura.pem
    option forwardfor
    http-request set-header X-Forwarded-Proto https
    http-response set-header Strict-Transport-Security "max-age=31536000"
    default_backend kura

backend kura
    timeout server 1h   # the live queue keeps one connection open
    server kura 127.0.0.1:8787
```

and in the compose file:

```yaml
    environment:
      KURA_TRUSTED_PROXIES: "172.17.0.1"   # the address from Kura's log, see step 2
```

## environment

| Variable | Default in the image | Meaning |
|---|---|---|
| `TZ` | UTC | Time zone, used for the daily Auto-Sync time. |
| `KURA_NAMING_TEMPLATE` | `{title} {episodenumber} {language}` | File name of each episode while Settings → Downloads sets no template. See [[Configuration]]. |
| `KURA_LISTEN` | `0.0.0.0:8787` | Address and port inside the container. Normally left alone. |
| `KURA_DATA` | `/data` | Folder for the database inside the container. Normally left alone. |
| `KURA_LIBRARY_DIR` | `/media` | Folder the first-start library setup suggests. |
| `KURA_TRUSTED_PROXIES` | none | Reverse proxies whose `X-Forwarded-For` is believed. See [Behind a reverse proxy](#behind-a-reverse-proxy). |

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
