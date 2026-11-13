# Roles and requests

Kura has two roles.

| | Admin | User |
|---|---|---|
| Home, search, title pages | yes | yes |
| Download | yes | no, requests instead |
| Queue | watch, cancel, retry, clear | watch |
| Request | accept or reject all requests | own requests and their status |
| Library, Auto-Sync, Settings | yes | no |

The server checks these rights itself, so they hold for the [JSON API](https://github.com/Koxility/kura#api) too, not only in the web app.

## Requests

A user opens a title, ticks episodes or seasons, optionally picks language and provider, and presses **Request Selected** or **Request All**. The request appears on the admin's **Request** page, with a count next to the menu item.

The admin presses **Accept** or **Reject**:

- **Accept** queues the episodes with the language and provider the user chose, into the site's default library (else the main library). Episodes that are already downloaded or queued are skipped.
- **Reject** queues nothing.

Users see on their own Request page whether a request is waiting, accepted or rejected.

## Managing users

Settings → Users. The last admin cannot be deleted, so nobody gets locked out. Deleting a user also deletes their requests and ends their logins.
