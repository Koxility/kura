# Kura

Kura (蔵, "storehouse") is a self-hosted web app that downloads anime and series from **AniWorld** and **SerienStream** into a library on your own disk, laid out the way Jellyfin reads it.

You browse both sites in one clean interface, pick episodes, seasons or whole series, and Kura downloads them in the background, several at a time. Auto-Sync keeps series up to date, and other people in your household can request titles that an admin then accepts.

## Start here

1. [[Requirements]]
2. [[Installation]] (Docker recommended)
3. [[Docker Compose|Docker-Compose]]: the configuration and its folders explained
4. [[First Start|First-Start]]: admin account and first library

## Using Kura

- [[Configuration]]: environment variables and every setting, including the naming template
- [[Folder Structure|Folder-Structure]]: where files go, IMDb IDs, Jellyfin
- [[Roles and Requests|Roles-and-Requests]]: admins, users and requests
- [[Updating]]
- [[Troubleshooting]]
- [[Development]]

## What Kura is not

Kura hosts and distributes nothing. It reads the pages of the sites you choose and saves the videos linked there to your storage. You are responsible for what you download; see the legal disclaimer in the [README](https://github.com/Koxility/kura#legal-disclaimer).
