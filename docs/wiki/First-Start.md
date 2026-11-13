# First start

## 1. Admin account

Open Kura in your browser (`http://<your server>:8787`). The first page asks for a name and a password (at least 8 characters). This account is the admin: it manages settings, libraries, users and requests.

There is no way to recover a forgotten admin password from the web app. Choose one you keep, or create a second admin later under Settings → Users.

## 2. First library

Next, Kura asks for a library: the folder downloads go to.

- **Name**: anything, e.g. `Anime` or `Serien`.
- **Folder on the server**: with Docker, a path *inside the container*, e.g. `/media` or `/media/anime` (see [[Docker Compose|Docker-Compose]]). From source, any absolute path on the machine.
- **Sites**: which sites download into this library by default.

Kura creates the folder if it does not exist. This first library is the *main library*: it is used whenever no other library applies.

## 3. First download

1. On **Home**, pick a site, search for a title or browse genres, and open it.
2. Open a season, tick episodes, or tick *Whole season* on the right of a season row.
3. Optionally choose language and provider at the bottom; *Automatic* follows your settings.
4. Press **Download Selected** or **Download All**.

If the site has no IMDb ID for the title, Kura asks for one first; see [[Folder Structure|Folder-Structure]].

Progress shows on **Queue**. Finished episodes appear under **Library** and in Jellyfin once it scans the folder.

## 4. More users

Settings → Users → name, password and role. Normal users browse and request; see [[Roles and Requests|Roles-and-Requests]].
