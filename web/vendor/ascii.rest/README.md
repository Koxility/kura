# ascii.rest (vendored)

The animated ASCII pieces behind Kura's optional background, from
[ascii.rest](https://ascii.rest) by [@bas3line](https://github.com/bas3line), MIT licensed (see
`LICENSE`).

Copied from <https://github.com/bas3line/ascii> at commit
`9ba5ab14a1946dc1ab875d5c456203079854bdcf` (15 November 2026): `src/library.ts`, `mount.ts`,
`types.ts`, `width.ts` and every file in `src/pieces/`, unchanged. `src/index.ts` is Kura's own
and exports only what Kura uses. The npm package `ascii.rest` (0.4.0) lags behind the website,
so pieces such as `planet-scale` were missing from it.

To update: copy the same files from a newer commit of that repository, note the commit here,
and run `bun run build`.
