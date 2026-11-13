# Development

Kura is a Rust workspace with an Axum server and a React web app. How the crates fit together is in [docs/ARCHITECTURE.md](https://github.com/Koxility/kura/blob/master/docs/ARCHITECTURE.md).

## Running

```bash
cargo run -p kura
```

```bash
cd web && bun install && bun run dev
```

`bun run dev` serves the web app with hot reload and passes `/api` on to the server on port 8787.

## Checks

These run in CI on every push and pull request, and must pass before a merge:

```bash
cargo fmt --all --check
```

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

```bash
cargo test --workspace
```

Tests that talk to the real sites and providers are skipped by default. Run them after a site or provider changes:

```bash
cargo test -p kura-sites -p kura-hosters -- --ignored
```

## Releasing

1. Set the new version in `Cargo.toml` and `web/package.json` (the build refuses different versions).
2. Move the `[Unreleased]` entries in `CHANGELOG.md` under the new version.
3. Commit, then tag and push:

   ```bash
   git tag -a v1.2.0 -m "Kura 1.2.0" && git push origin v1.2.0
   ```

4. The *Docker image* workflow builds `ghcr.io/koxility/kura` for amd64 and arm64 and tags it `1.2.0`, `1.2`, `1` and `latest`.
5. Create the GitHub release from the changelog. Running Kura 1.2.1 and later show the Update label within an hour.

## Adding a site or provider

- A site implements the `Site` trait in `crates/kura-sites` (search, lists, series, episodes, links).
- A provider implements the `Hoster` trait in `crates/kura-hosters` (page → stream URL) and is added to `kura_hosters::all`.

Both get tests against saved HTML in `fixtures/`, plus an ignored live test.
