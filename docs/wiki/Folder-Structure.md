# Folder structure and naming

Inside every library Kura writes:

```
<library>/
  Series/
    One Piece [imdbid-tt0388629]/
      Season 01/
        One Piece S01E001 German Dub.mkv
        One Piece S01E002 German Dub.mkv
      Season 00/
        One Piece S00E001 German Dub.mkv
```

- **`Series/`**: always there, for anime and series alike. Point Jellyfin at this folder.
- **`<Title> [imdbid-…]/`**: one folder per series. Characters not allowed in folder names are replaced by spaces.
- **`Season XX/`**: two digits. `Season 00` holds films and specials.
- **File name**: from the naming template, see [[Configuration]]. The default gives `One Piece S01E001 German Dub.mkv`.

While an episode downloads, it is written as `<name>.mkv.part` and renamed when complete, so Jellyfin never sees half a file.

## IMDb IDs

The `[imdbid-tt0388629]` part tells Jellyfin exactly which series this is, so it finds the right metadata even for titles with common or translated names.

- Kura reads the ID from the title's page on AniWorld or SerienStream.
- If the site has none, Kura asks before downloading. Enter the ID from the IMDb address (`imdb.com/title/tt0388629/` → `tt0388629`), or press **Skip**.
- Skipped titles get `[imdbid-none]`.
- Later episodes of a series, from Auto-Sync or a retry, use the same ID, so a series stays in one folder.

Series downloaded with Kura 1.0.0 before IMDb IDs existed keep their old folder without the ID.

## Jellyfin

1. Dashboard → Libraries → Add Media Library.
2. Content type: **Shows**.
3. Folder: the library's `Series` folder, e.g. `/mnt/nas/Media/Series`. With Docker, this is the *host* path (the left side of the volume line), or wherever the same disk is mounted in your Jellyfin container.
4. Save and scan.

Jellyfin reads the IMDb ID from the folder name and the season and episode from `S01E001`.
