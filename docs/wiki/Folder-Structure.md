# Folder structure and naming

Inside every library Kura writes:

```
<library>/
  One Piece [imdbid-tt0388629]/
    Season 01/
      One Piece S01E001 German Dub.mkv
      One Piece S01E002 German Dub.mkv
    Season 00/
      One Piece S00E001 German Dub.mkv
```

- **`<Title> [imdbid-…]/`**: one folder per series, directly in the library. Characters not allowed in folder names are replaced by spaces.
- **`Season XX/`**: two digits. `Season 00` holds films and specials.
- **File name**: from the naming template, see [[Configuration]]. The default gives `One Piece S01E001 German Dub.mkv`.

While an episode downloads, it is written as `<name>.mkv.part` and renamed when complete, so Jellyfin never sees half a file.

### Series from older versions

Kura 1.0 and 1.1 put series into a `Series` folder inside the library (`<library>/Series/One Piece [imdbid-…]/`). Kura 1.2 leaves those where they are: new episodes of such a series, from Auto-Sync or a new download, go into the folder the series already has. Only series downloaded for the first time use the new layout.

To move everything to the new layout, stop Kura, move the series folders out of `Series` into the library folder, and start Kura again. Kura then finds them by IMDb ID or name, so they still show as downloaded; new episodes of moved series go to the new place.

## Already downloaded

Kura marks titles and episodes that are already in a library with a green check: on Home (top left of the cover), on a title's page (left of every episode, and per season), and in the Library (right of every series folder).

It recognises a series by

- its finished downloads in Kura,
- a folder in any library whose name carries the same IMDb ID (`[imdbid-tt0388629]`), or
- a folder whose name is the same title, ignoring case, punctuation and the `[imdbid-…]` part.

Inside a matching folder, every video file with `S01E001`, `S1E1` or similar in its name counts as that episode. So a library filled by hand or by another tool is recognised too. Libraries are read at most every 30 seconds.

## IMDb IDs

The `[imdbid-tt0388629]` part tells Jellyfin exactly which series this is, so it finds the right metadata even for titles with common or translated names.

- Kura reads the ID from the title's page on AniWorld or SerienStream.
- If the site has none, Kura asks before downloading. Enter the ID from the IMDb address (`imdb.com/title/tt0388629/` → `tt0388629`), or press **Skip**.
- Skipped titles get `[imdbid-none]`.
- Later episodes of a series, from Auto-Sync or a retry, use the same ID, so a series stays in one folder.

Series downloaded with Kura 1.0 before IMDb IDs existed keep their old folder without the ID.

## Jellyfin

1. Dashboard → Libraries → Add Media Library.
2. Content type: **Shows**.
3. Folder: the library folder, e.g. `/mnt/nas/Media`. With Docker, this is the *host* path (the left side of the volume line), or wherever the same disk is mounted in your Jellyfin container. If you used Kura 1.0 or 1.1, add the library's `Series` folder as a second folder of the same Jellyfin library, or move its contents as described above.
4. Save and scan.

Jellyfin reads the IMDb ID from the folder name and the season and episode from `S01E001`.
