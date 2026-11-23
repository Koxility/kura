# Roles and requests

Kura has two roles.

| | Admin | User |
|---|---|---|
| Home, search, title pages | yes | yes |
| Download | yes | no, requests instead |
| Queue | watch, cancel, retry, clear | watch |
| Request | accept or reject all requests | own requests and their status |
| Library, Auto-Sync, server settings | yes | no |
| Calendar, own account and theme in Settings | yes | yes |

The server checks these rights itself, so they hold for the [JSON API](https://github.com/Koxility/kura#api) too, not only in the web app.

## Requests

A user opens a title, ticks episodes or seasons, optionally picks language and provider, and presses **Request Selected** or **Request All**. The request appears on the admin's **Request** page, with a count next to the menu item.

The admin presses **Accept** or **Reject**:

- **Accept** queues the episodes with the language and provider the user chose, into the site's default library (else the main library). Episodes that are already downloaded or queued are skipped.
- **Reject** queues nothing.

One request can name up to 200 seasons or 5000 single episodes; for more, request the whole series. A user can have any number of requests waiting.

Users see on their own Request page whether a request is waiting, accepted or rejected. While nobody has decided yet, they can **Cancel** it.

Admins can tidy the list with **Clear requests**: it removes accepted and rejected requests and keeps the open ones. Episodes from accepted requests stay in the queue.

## Changing your password

Every account, admin or user, changes its own name, password and email under Settings → Account; the email (where a reset link goes) also needs the current password. The current password is needed to change it; other devices logged in with the account are logged out, and the account's API keys are deleted, so programs such as Musubi need a new key.

## Managing users

Settings → Administration → Users (add an account or invite someone by email; there is no open sign-up). The last admin cannot be deleted, so nobody gets locked out. Deleting a user also deletes their requests and ends their logins.
