# Updating

When a newer release exists, the top bar shows a small blue **Update** label next to the version number. It links to the release notes. Kura asks GitHub about this at start and then every hour (after a failed check, again within 5 minutes); nothing else is sent. Kura 1.1 and 1.2.0 only checked once a day: if one of them was already running when a release came out, restart it to see the label at once.

## Docker Compose

In the folder with your `docker-compose.yaml`:

```bash
docker compose pull
docker compose up -d
```

Your database and downloads live in the mapped volumes and are kept. Running downloads are interrupted and start over after the update.

To remove old images afterwards:

```bash
docker image prune
```

If you pinned a version (`ghcr.io/koxility/kura:1.1.0`), change the tag in the compose file first.

## docker run

```bash
docker pull ghcr.io/koxility/kura:latest
docker stop kura && docker rm kura
```

Then start it again with the same `docker run` command as before.

## From source

```bash
git pull
cd web && bun install && bun run build && cd ..
cargo build --release
```

Restart Kura afterwards.

## Database changes

Kura upgrades its database by itself on start. Downgrading to an older version after that is not supported; keep a copy of the data folder if you want the option.
