# Troubleshooting

## Where are the logs?

```bash
docker logs kura          # Docker
docker logs -f kura       # follow live
```

From source, Kura logs to the terminal it runs in. More detail: set `RUST_LOG=debug`.

## Kura does not start: "could not open the database" or "Permission denied"

The `user:` in the compose file cannot write to the mapped data folder. Give the folder to that user:

```bash
sudo chown -R 1000:1000 ./data
```

Use the ids from your `user:` line.

## "the folder … cannot be created" when saving a library

Same cause for the media folder: the path is not writable for Kura's user, or (with Docker) the path is not inside a mapped volume. Paths in Kura must be container paths such as `/media/anime`; see [[Docker Compose|Docker-Compose]].

## The page does not load

- `docker ps` shows whether the container runs and is `healthy`.
- Is the port in use by something else? Change the left side of `ports:`.
- From another machine: is a firewall blocking the port?
- From source: Kura listens on `127.0.0.1` by default. Set `KURA_LISTEN=0.0.0.0:8787` to reach it from other machines.

## A download fails

Open the title on **Queue**: every failed attempt is listed, e.g. `voe, german_dub: no stream found on the VOE page`.

- **"no stream found"**: the video was removed from that provider, or the provider changed its page. Kura tries the next provider and language by itself; *Retry* later, or pick another provider on the title's page.
- **"no link in a wanted language from a supported hoster"**: the episode exists only in languages or on providers you have not enabled. Check the language order and provider order in Settings → Downloads.
- **"ffmpeg failed"**: the stream broke off. Usually a *Retry* works.

Providers change their pages from time to time. If a provider fails for every episode, check for a Kura update.

## Search or title pages show "the page layout changed"

The site changed its pages. Check for a Kura update; if there is none yet, open an [issue](https://github.com/Koxility/kura/issues).

## No IMDb ID dialog, or the wrong ID

The dialog only appears when the site has no ID. When the site has an ID, Kura always uses it and it cannot be overridden in Kura. If it is wrong, tell the site; renaming the folder by hand does not help, because new episodes are saved under the ID Kura has stored for the series.

## Checking the VPN

Settings → Administration → IP check shows the public address Kura uses. With Docker, Kura uses the network of its container; to route it through a VPN, run it in the network of a VPN container (`network_mode: service:vpn`).

## Forgot the admin password

Another admin can delete and recreate the account under Settings → Users. If there is no other admin, stop Kura, move the data folder away, and start fresh; downloads on disk are not affected.
