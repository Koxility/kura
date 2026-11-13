# Installation

## Docker (recommended)

1. Create a folder for Kura and download the compose file into it:

   ```bash
   mkdir kura && cd kura
   curl -LO https://raw.githubusercontent.com/Koxility/kura/master/docker-compose.yaml
   ```

2. Open `docker-compose.yaml` and set at least:
   - `user:` to the host user that should own the downloads (`id -u` and `id -g`),
   - the media volume `/path/to/media:/media` to the folder your library lives in,
   - `TZ` to your time zone.

   [[Docker Compose|Docker-Compose]] explains every line.

3. Start Kura:

   ```bash
   docker compose up -d
   ```

4. Open `http://<your server>:8787` and continue with [[First Start|First-Start]].

The image is `ghcr.io/koxility/kura`. Tags: `latest`, an exact version like `1.1.0`, or a series like `1.1` (newest 1.1.x) and `1` (newest 1.x).

### Without Compose

```bash
docker run -d --name kura --restart unless-stopped \
  --user 1000:1000 -p 8787:8787 -e TZ=Europe/Berlin \
  -v "$PWD/data:/data" -v /path/to/media:/media \
  ghcr.io/koxility/kura:latest
```

## From source

```bash
git clone https://github.com/Koxility/kura.git
cd kura/web && bun install && bun run build && cd ..
cargo build --release
./target/release/kura
```

The result is one binary with the web app built in. It listens on `127.0.0.1:8787` and keeps its database in `./data`; see [[Configuration]] to change that.

To run it as a service, a systemd unit like this works:

```ini
[Unit]
Description=Kura
After=network-online.target

[Service]
ExecStart=/opt/kura/kura
WorkingDirectory=/opt/kura
Environment=KURA_LISTEN=0.0.0.0:8787
User=kura
Restart=on-failure

[Install]
WantedBy=multi-user.target
```
