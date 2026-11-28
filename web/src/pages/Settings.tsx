import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { type ChangeEvent, type FormEvent, useRef, useState } from "react";
import { Navigate, NavLink, useParams } from "react-router";
import {
  type Account,
  accountQuery,
  type ApiKey,
  type Backdrop,
  api,
  hosterNames,
  type Invite,
  type Jellyfin as JellyfinSettings,
  type Language,
  type Library,
  languageNames,
  languages,
  type MailEncryption,
  type MailSettings,
  type Settings,
  type Site,
  settingsQuery,
  siteNames,
  sites,
  supportedHosters,
  songsQuery,
  type Role,
  type Theme,
  type User,
  type UserEntry,
} from "../api";
import { Confirm, Modal } from "../Modal";
import { pieceFromLink, pieceTitle } from "../ThemeEffects";
import ui from "../ui.module.css";
import styles from "./Settings.module.css";

type Part = { id: string; title: string };
type Section = Part & { admin?: boolean; parts?: Part[] };

/** Server settings are for admins; everyone has their own account and theme. Administration
 *  has parts of its own, listed under it in the menu while it is open. */
const sections: Section[] = [
  { id: "downloads", title: "Downloads", admin: true },
  { id: "library", title: "Library", admin: true },
  { id: "sites", title: "Sites", admin: true },
  { id: "auto-sync", title: "Auto-Sync", admin: true },
  { id: "jellyfin", title: "Jellyfin", admin: true },
  { id: "account", title: "Account" },
  { id: "theme", title: "Theme" },
  {
    id: "administration",
    title: "Administration",
    admin: true,
    parts: [
      { id: "users", title: "Users" },
      { id: "mail", title: "Mail" },
      { id: "configuration", title: "Configuration" },
      { id: "network", title: "Network" },
    ],
  },
];

export function SettingsPage({ user }: { user: User }) {
  const admin = user.role === "admin";
  const settings = useQuery({ ...settingsQuery, enabled: admin });
  const params = useParams();
  const shown = sections.filter((section) => admin || !section.admin);
  // "Users" was the account's own section before 1.4.
  if (params.section === "users") return <Navigate to="/settings/account" replace />;
  const section = shown.find((section) => section.id === params.section) ?? shown[0];
  const part = section.parts?.find((part) => part.id === params.part) ?? section.parts?.[0];

  return (
    <>
      <h1>Settings</h1>
      <div className={styles.layout}>
        <nav className={styles.sections} aria-label="Settings sections">
          {shown.map((entry) => {
            const open = entry.id === section.id;
            return entry.parts ? (
              <div key={entry.id} className={styles.group}>
                <NavLink
                  to={`/settings/${entry.id}`}
                  className={styles.section}
                  aria-expanded={open}
                >
                  {entry.title}
                  <span className={styles.chevron} aria-hidden="true">
                    {open ? "▾" : "▸"}
                  </span>
                </NavLink>
                {open && (
                  <div className={styles.parts}>
                    {entry.parts.map((item) => (
                      <NavLink
                        key={item.id}
                        to={`/settings/${entry.id}/${item.id}`}
                        className={styles.part}
                        aria-current={item.id === part?.id ? "page" : undefined}
                      >
                        {item.title}
                      </NavLink>
                    ))}
                  </div>
                )}
              </div>
            ) : (
              <NavLink key={entry.id} to={`/settings/${entry.id}`} className={styles.section}>
                {entry.title}
              </NavLink>
            );
          })}
        </nav>
        <div className={styles.content}>
          {part && <p className={styles.crumb}>{section.title}</p>}
          <h2 className={styles.heading}>{part?.title ?? section.title}</h2>
          {settings.isError && (
            <p className={ui.error} role="alert">
              {settings.error.message}
            </p>
          )}
          {section.id === "account" ? (
            <AccountSection />
          ) : section.id === "theme" ? (
            <ThemeSection admin={admin} />
          ) : part?.id === "users" ? (
            <AdminUsers me={user} />
          ) : part?.id === "mail" ? (
            <MailSection />
          ) : part?.id === "network" ? (
            <IpCheck />
          ) : (
            settings.data && (
              // Remounted per section, so leaving one drops its unsaved changes.
              <SettingsForm
                key={part?.id ?? section.id}
                initial={settings.data}
                section={part?.id ?? section.id}
                readOnly={!admin}
              />
            )
          )}
        </div>
      </div>
    </>
  );
}

function SettingsForm({
  initial,
  section,
  readOnly,
}: {
  initial: Settings;
  section: string;
  readOnly: boolean;
}) {
  const client = useQueryClient();
  const [draft, setDraft] = useState(initial);
  const change = (part: Partial<Settings>) => setDraft({ ...draft, ...part });
  const save = useMutation({
    mutationFn: (settings: Settings) => {
      // Exclusions are changed on other pages; keep the newest list, not the one from when
      // this form was opened.
      const newest = client.getQueryData<Settings>(settingsQuery.queryKey);
      const excluded = newest?.auto_sync.excluded ?? settings.auto_sync.excluded;
      const removed = newest?.auto_sync.removed ?? settings.auto_sync.removed;
      const merged = { ...settings, auto_sync: { ...settings.auto_sync, excluded, removed } };
      return api<Settings>("/settings", "PUT", merged);
    },
    onSuccess: (saved) => {
      client.setQueryData(settingsQuery.queryKey, saved);
      setDraft(saved);
      // The status names the default language, provider and libraries.
      client.invalidateQueries({ queryKey: ["status"] });
    },
  });

  function onSubmit(event: FormEvent) {
    event.preventDefault();
    save.mutate(draft);
  }

  return (
    <form className={styles.form} onSubmit={onSubmit}>
      <fieldset className={styles.fields} disabled={readOnly}>
        {section === "downloads" && <Downloads draft={draft} change={change} />}
        {section === "library" && <Libraries draft={draft} change={change} />}
        {section === "sites" && <Sites draft={draft} change={change} />}
        {section === "auto-sync" && <AutoSync draft={draft} change={change} />}
        {section === "jellyfin" && <Jellyfin draft={draft} change={change} />}
        {section === "configuration" && (
          <Configuration settings={initial} onImport={(imported) => save.mutate(imported)} />
        )}
        {!readOnly && section !== "configuration" && (
          <div className={`${ui.actions} ${styles.left}`}>
            <button className={ui.primary} disabled={save.isPending}>
              Save
            </button>
            <span className={ui.muted} role="status">
              {save.isSuccess && "Saved."}
            </span>
          </div>
        )}
        {save.isError && (
          <p className={ui.error} role="alert">
            {save.error.message}
          </p>
        )}
        {section === "configuration" && save.isSuccess && (
          <p className={ui.muted} role="status">
            Configuration imported.
          </p>
        )}
      </fieldset>
    </form>
  );
}

type SectionProps = { draft: Settings; change: (part: Partial<Settings>) => void };

/** The file name the server would give episode 1 of One Piece, for the preview. */
function previewName(template: string, format: string) {
  const name = template
    .replaceAll("{title}", "One Piece")
    .replaceAll("{episodenumber}", "S01E001")
    .replaceAll("{language}", "German Dub")
    .replaceAll("{season}", "01")
    .replaceAll("{episode}", "001")
    .replaceAll("{imdbid}", "tt0388629")
    .replace(/[/\\:*?"<>|]/g, " ")
    .replace(/\s+/g, " ")
    .trim();
  return `${name}.${format}`;
}

function Downloads({ draft, change }: SectionProps) {
  const fallback = draft.naming_template_default ?? "{title} {episodenumber} {language}";
  const template = draft.naming_template ?? fallback;

  return (
    <>
      <label className={ui.field}>
        Naming template
        <input
          className={`${ui.input} ${ui.mono}`}
          value={draft.naming_template ?? ""}
          placeholder={fallback}
          onChange={(event) => change({ naming_template: event.target.value || null })}
          spellCheck={false}
        />
      </label>
      <p className={styles.hint}>
        Variables: {"{title}"}, {"{episodenumber}"} (S01E001), {"{language}"} (German Dub),{" "}
        {"{season}"} (01), {"{episode}"} (001), {"{imdbid}"}. Needs {"{episodenumber}"}, or{" "}
        {"{season}"} and {"{episode}"}. Empty uses the default shown above.
      </p>
      <p className={styles.preview}>
        <span className={styles.faint}>Example:</span>{" "}
        <span className={ui.mono}>{previewName(template, draft.output_format)}</span>
        {draft.naming_template !== null && (
          <button type="button" className={ui.ghost} onClick={() => change({ naming_template: null })}>
            Reset to default
          </button>
        )}
      </p>
      <label className={ui.field}>
        Output format
        <select
          className={`${ui.input} ${styles.short}`}
          value={draft.output_format}
          onChange={(event) => change({ output_format: event.target.value as "mkv" | "mp4" })}
        >
          <option value="mkv">mkv</option>
          <option value="mp4">mp4</option>
        </select>
      </label>
      <label className={ui.field}>
        Downloads at the same time
        <input
          className={`${ui.input} ${styles.short}`}
          type="number"
          min={1}
          max={16}
          value={draft.parallel}
          onChange={(event) => change({ parallel: event.target.valueAsNumber || 1 })}
          required
        />
      </label>
      <Order
        label="Provider fallback order"
        hint="Tried from top to bottom within a language."
        all={[...new Set([...supportedHosters, ...draft.hosters])]}
        value={draft.hosters}
        name={(hoster) =>
          supportedHosters.includes(hoster)
            ? (hosterNames[hoster] ?? hoster)
            : `${hoster} (not supported yet)`
        }
        onChange={(hosters) => change({ hosters })}
      />
      <Order
        label="Language order"
        hint="The first is the default for downloads and requests; the others are tried when it has no link. Auto-Sync uses the same order."
        all={languages}
        value={draft.languages}
        name={(language) => languageNames[language as Language]}
        onChange={(order) => change({ languages: order as Language[] })}
      />
    </>
  );
}

function Libraries({ draft, change }: SectionProps) {
  const update = (index: number, part: Partial<Library>) =>
    change({
      libraries: draft.libraries.map((library, other) =>
        other === index ? { ...library, ...part } : library,
      ),
    });
  /** A site has one default library: choosing it here takes it away from the others. */
  const setDefault = (index: number, site: Site, on: boolean) =>
    change({
      libraries: draft.libraries.map((library, other) => {
        const rest = library.default_for.filter((entry) => entry !== site);
        return { ...library, default_for: other === index && on ? [...rest, site] : rest };
      }),
    });
  const makeMain = (index: number) =>
    change({
      libraries: [draft.libraries[index], ...draft.libraries.filter((_, other) => other !== index)],
    });

  return (
    <>
      <p className={styles.hint}>
        Downloads go to the library picked on a title's page, else to the default library of the
        site, else to the main library. Inside a library, files are stored as
        Series/Title/Season XX. Kura creates missing folders when you save.
      </p>

      {draft.libraries.map((library, index) => (
        // Libraries have no id of their own; their position is stable while editing.
        // biome-ignore lint/suspicious/noArrayIndexKey: see above
        <div key={index} className={styles.library}>
          <p className={styles.libraryHead}>
            {index === 0 ? (
              <span>Main library</span>
            ) : (
              <button type="button" className={ui.ghost} onClick={() => makeMain(index)}>
                Make main library
              </button>
            )}
            <button
              type="button"
              className={ui.ghost}
              disabled={draft.libraries.length === 1}
              title={draft.libraries.length === 1 ? "Kura needs at least one library" : undefined}
              onClick={() =>
                change({ libraries: draft.libraries.filter((_, other) => other !== index) })
              }
            >
              Remove
            </button>
          </p>
          <label className={ui.field}>
            Name
            <input
              className={ui.input}
              value={library.name}
              onChange={(event) => update(index, { name: event.target.value })}
              required
            />
          </label>
          <label className={ui.field}>
            Path
            <input
              className={ui.input}
              value={library.path}
              onChange={(event) => update(index, { path: event.target.value })}
              placeholder="/mnt/nas/Serien"
              required
            />
          </label>
          <label className={ui.field}>
            Path in Jellyfin
            <input
              className={ui.input}
              value={library.jellyfin_path ?? ""}
              onChange={(event) =>
                update(index, { jellyfin_path: event.target.value.trim() || null })
              }
              placeholder="Same as the path"
            />
          </label>
          <div className={ui.field} role="group" aria-label={`Default library for`}>
            Default for
            <div className={styles.inline}>
              {sites.map((site) => (
                <label key={site} className={ui.check}>
                  <input
                    type="checkbox"
                    checked={library.default_for.includes(site)}
                    onChange={(event) => setDefault(index, site, event.target.checked)}
                  />
                  {siteNames[site]}
                </label>
              ))}
            </div>
          </div>
        </div>
      ))}
      <div className={`${ui.actions} ${styles.left}`}>
        <button
          type="button"
          className={ui.button}
          onClick={() =>
            change({ libraries: [...draft.libraries, { name: "", path: "", default_for: [] }] })
          }
        >
          Add library
        </button>
      </div>
    </>
  );
}

function Sites({ draft, change }: SectionProps) {
  return (
    <ul className={`${ui.list} ${styles.top}`}>
      {sites.map((site) => (
        <li key={site} className={ui.row}>
          <label className={`${ui.check} ${ui.grow}`}>
            <input
              type="checkbox"
              checked={!draft.disabled_sites.includes(site)}
              onChange={(event) =>
                change({
                  disabled_sites: event.target.checked
                    ? draft.disabled_sites.filter((other) => other !== site)
                    : [...draft.disabled_sites, site],
                })
              }
            />
            {siteNames[site]}
          </label>
          <span className={ui.muted}>
            {draft.disabled_sites.includes(site) ? "Off" : "On"}
          </span>
        </li>
      ))}
    </ul>
  );
}

function AutoSync({ draft, change }: SectionProps) {
  const auto = draft.auto_sync;
  const set = (part: Partial<Settings["auto_sync"]>) => change({ auto_sync: { ...auto, ...part } });
  const every = auto.schedule.kind === "every";
  // Each kind keeps a sensible value while the other one is selected.
  const minutes = auto.schedule.kind === "every" ? auto.schedule.minutes : 60;
  const at = auto.schedule.kind === "daily" ? auto.schedule.at : "03:00";

  return (
    <>
      <label className={`${ui.check} ${styles.toggle}`}>
        <input
          type="checkbox"
          checked={auto.enabled}
          onChange={(event) => set({ enabled: event.target.checked })}
        />
        Enable Auto-Sync
      </label>
      <label className={`${ui.check} ${styles.toggle}`}>
        <input
          type="checkbox"
          checked={auto.only_new}
          onChange={(event) => set({ only_new: event.target.checked })}
        />
        Only new episodes
      </label>
      <p className={styles.hint}>
        When a series shows up in the site's list of new episodes. Off: every episode it is
        missing is downloaded. On: only the announced episodes and those between them. Sync now
        always checks every series in full.
      </p>

      <fieldset className={styles.schedule}>
        <legend>Schedule</legend>
        <label className={ui.check}>
          <input
            type="radio"
            name="schedule"
            checked={every}
            onChange={() => set({ schedule: { kind: "every", minutes } })}
          />
          Every
          <input
            className={`${ui.input} ${ui.compact} ${styles.short}`}
            type="number"
            min={5}
            max={10080}
            value={minutes}
            disabled={!every}
            aria-label="Minutes between checks"
            onChange={(event) =>
              set({ schedule: { kind: "every", minutes: event.target.valueAsNumber || 5 } })
            }
          />
          minutes
        </label>
        <label className={ui.check}>
          <input
            type="radio"
            name="schedule"
            checked={!every}
            onChange={() => set({ schedule: { kind: "daily", at } })}
          />
          Every day at
          <input
            className={`${ui.input} ${ui.compact} ${styles.short}`}
            type="time"
            value={at}
            disabled={every}
            aria-label="Time of the daily check"
            onChange={(event) => set({ schedule: { kind: "daily", at: event.target.value } })}
          />
        </label>
      </fieldset>
    </>
  );
}

function Jellyfin({ draft, change }: SectionProps) {
  const jellyfin = draft.jellyfin;
  const set = (part: Partial<JellyfinSettings>) => change({ jellyfin: { ...jellyfin, ...part } });
  const test = useMutation({
    mutationFn: () => api<{ server: string }>("/admin/jellyfin/test", "POST", jellyfin),
  });

  return (
    <>
      <p className={styles.hint}>
        When the queue is empty, Kura asks Jellyfin to scan the series folders that got new
        episodes, and only those, never the whole library. Failed downloads stay in the queue and
        do not hold the scan back. Leave the address empty to switch this off. If Jellyfin sees a
        library under another path, enter it as "Path in Jellyfin" in the Library section.
      </p>
      <label className={ui.field}>
        Address
        <input
          className={ui.input}
          type="url"
          value={jellyfin.url}
          onChange={(event) => set({ url: event.target.value })}
          placeholder="http://192.168.1.10:8096"
        />
      </label>
      <label className={ui.field}>
        API key
        <input
          className={ui.input}
          type="password"
          autoComplete="off"
          value={jellyfin.api_key}
          onChange={(event) => set({ api_key: event.target.value })}
          placeholder={
            jellyfin.api_key_set
              ? "Saved; leave empty to keep it"
              : "From Jellyfin: Dashboard, API Keys"
          }
        />
      </label>
      <div className={`${ui.actions} ${styles.left}`}>
        <button
          type="button"
          className={ui.button}
          disabled={test.isPending || !jellyfin.url.trim()}
          onClick={() => test.mutate()}
        >
          Test connection
        </button>
        <span className={ui.muted} role="status">
          {test.isPending ? "Asking…" : test.isSuccess && `Connected to ${test.data.server}.`}
        </span>
      </div>
      {test.isError && (
        <p className={ui.error} role="alert">
          {test.error.message}
        </p>
      )}
    </>
  );
}

/** Export and import of all settings. */
function Configuration({
  settings,
  onImport,
}: {
  settings: Settings;
  onImport: (settings: Settings) => void;
}) {
  const [problem, setProblem] = useState("");

  function exportFile() {
    const file = new Blob([JSON.stringify(settings, null, 2)], { type: "application/json" });
    const link = document.createElement("a");
    link.href = URL.createObjectURL(file);
    link.download = "kura-configuration.json";
    link.click();
    URL.revokeObjectURL(link.href);
  }

  async function importFile(event: ChangeEvent<HTMLInputElement>) {
    const file = event.target.files?.[0];
    event.target.value = "";
    if (!file) return;
    setProblem("");
    try {
      const imported = JSON.parse(await file.text()) as Settings;
      if (confirm("Replace the current configuration with the one from this file?")) {
        onImport(imported);
      }
    } catch {
      setProblem("This file is not a Kura configuration.");
    }
  }

  return (
    <>
      <p className={styles.hint}>
        The file holds all settings, libraries and Auto-Sync exclusions. Accounts, passwords and
        the mail server are not part of it.
      </p>
      <div className={`${ui.actions} ${styles.left}`}>
        <button type="button" className={ui.button} onClick={exportFile}>
          Export
        </button>
        <label className={ui.button}>
          Import
          <input type="file" accept="application/json,.json" hidden onChange={importFile} />
        </label>
      </div>
      {problem && (
        <p className={ui.error} role="alert">
          {problem}
        </p>
      )}
    </>
  );
}

/** The address the server reaches the internet from, on request. */
function IpCheck() {
  const ip = useMutation({ mutationFn: () => api<{ ip: string }>("/admin/ip") });
  return (
    <section className={styles.form}>
      <p className={styles.hint}>
        Shows the address the server reaches the internet from, e.g. to see whether a VPN is in
        use. This asks an outside service (api.ipify.org).
      </p>
      <div className={`${ui.actions} ${styles.left}`}>
        <button
          type="button"
          className={ui.button}
          disabled={ip.isPending}
          onClick={() => ip.mutate()}
        >
          Check now
        </button>
        <span className={ui.mono} role="status">
          {ip.isPending ? "Asking…" : ip.data?.ip}
        </span>
      </div>
      {ip.isError && (
        <p className={ui.error} role="alert">
          {ip.error.message}
        </p>
      )}
    </section>
  );
}

/** An ordered choice: move entries up and down, remove them, add unused ones back. */
function Order({
  label,
  hint,
  all,
  value,
  name,
  onChange,
}: {
  label: string;
  hint: string;
  all: string[];
  value: string[];
  name: (entry: string) => string;
  onChange: (value: string[]) => void;
}) {
  function move(index: number, by: number) {
    const next = [...value];
    [next[index], next[index + by]] = [next[index + by], next[index]];
    onChange(next);
  }
  const unused = all.filter((entry) => !value.includes(entry));

  return (
    <div className={ui.field} role="group" aria-label={label}>
      <span>
        {label} <span className={styles.faint}>· {hint}</span>
      </span>
      <ol className={ui.list}>
        {value.map((entry, index) => (
          <li key={entry} className={`${ui.row} ${styles.entry}`}>
            <span className={ui.grow}>{name(entry)}</span>
            <button
              type="button"
              className={ui.ghost}
              disabled={index === 0}
              aria-label={`Move ${name(entry)} up`}
              onClick={() => move(index, -1)}
            >
              ↑
            </button>
            <button
              type="button"
              className={ui.ghost}
              disabled={index === value.length - 1}
              aria-label={`Move ${name(entry)} down`}
              onClick={() => move(index, 1)}
            >
              ↓
            </button>
            <button
              type="button"
              className={ui.ghost}
              onClick={() => onChange(value.filter((other) => other !== entry))}
            >
              Remove
            </button>
          </li>
        ))}
        {unused.map((entry) => (
          <li key={entry} className={`${ui.row} ${styles.entry}`}>
            <span className={`${ui.grow} ${styles.faint}`}>{name(entry)}</span>
            <button type="button" className={ui.ghost} onClick={() => onChange([...value, entry])}>
              Add
            </button>
          </li>
        ))}
      </ol>
    </div>
  );
}

/** The own account: name, email, password and API keys. */
function AccountSection() {
  const client = useQueryClient();
  const account = useQuery(accountQuery);
  const saved = (next: Account) => {
    client.setQueryData(accountQuery.queryKey, next);
    // The name in the top bar and in the user list.
    client.invalidateQueries({ queryKey: ["status"] });
    client.invalidateQueries({ queryKey: ["users"] });
  };
  if (account.isError) {
    return (
      <p className={ui.error} role="alert">
        {account.error.message}
      </p>
    );
  }
  if (!account.data) return null;
  return (
    <section className={styles.form}>
      <NameForm account={account.data} onSaved={saved} />
      <EmailForm account={account.data} onSaved={saved} />
      <PasswordForm />
      <ApiKeys />
    </section>
  );
}

function NameForm({ account, onSaved }: { account: Account; onSaved: (next: Account) => void }) {
  const [name, setName] = useState(account.name);
  const rename = useMutation({
    mutationFn: () => api<Account>("/account/name", "PUT", { name }),
    onSuccess: onSaved,
  });
  return (
    <form
      className={styles.oneLine}
      onSubmit={(event) => {
        event.preventDefault();
        rename.mutate();
      }}
    >
      <label className={ui.field}>
        Name
        <input
          className={ui.input}
          value={name}
          onChange={(event) => setName(event.target.value)}
          autoComplete="username"
          maxLength={64}
          required
        />
      </label>
      <button className={ui.primary} disabled={rename.isPending || name.trim() === account.name}>
        Change name
      </button>
      <Outcome mutation={rename} done="Name changed." />
    </form>
  );
}

function EmailForm({ account, onSaved }: { account: Account; onSaved: (next: Account) => void }) {
  const [email, setEmail] = useState(account.email ?? "");
  const [password, setPassword] = useState("");
  const changed = email.trim() !== (account.email ?? "");
  const save = useMutation({
    mutationFn: () => api<Account>("/account/email", "PUT", { email, password }),
    onSuccess: (next) => {
      setPassword("");
      onSaved(next);
    },
  });
  return (
    <form
      className={styles.password}
      onSubmit={(event) => {
        event.preventDefault();
        save.mutate();
      }}
    >
      <label className={ui.field}>
        <span>
          Email <span className={styles.faint}>(optional, for password reset)</span>
        </span>
        <input
          className={ui.input}
          type="email"
          value={email}
          onChange={(event) => setEmail(event.target.value)}
          autoComplete="email"
          placeholder="you@example.org"
        />
      </label>
      {changed && (
        <label className={ui.field}>
          Current password
          <input
            className={ui.input}
            type="password"
            value={password}
            onChange={(event) => setPassword(event.target.value)}
            autoComplete="current-password"
            required
          />
        </label>
      )}
      {account.pending_email && (
        <p className={styles.hint} role="status">
          Waiting for confirmation: open the link sent to {account.pending_email}. Until then,
          reset links go {account.email ? `to ${account.email}` : "nowhere"}.
        </p>
      )}
      <div className={`${ui.actions} ${styles.left}`}>
        <button className={ui.primary} disabled={save.isPending || !changed}>
          Save email
        </button>
        <Outcome
          mutation={save}
          done={
            save.data?.pending_email
              ? `Link sent to ${save.data.pending_email}; open it to confirm.`
              : "Saved."
          }
        />
      </div>
    </form>
  );
}

function PasswordForm() {
  const [current, setCurrent] = useState("");
  const [next, setNext] = useState("");
  const [again, setAgain] = useState("");
  const client = useQueryClient();
  const change = useMutation({
    mutationFn: () => api("/account/password", "PUT", { current, new: next }),
    onSuccess: () => {
      // The account's API keys are gone with the old password.
      client.invalidateQueries({ queryKey: ["api-keys"] });
      setCurrent("");
      setNext("");
      setAgain("");
    },
  });
  const mismatch = again !== "" && again !== next;
  return (
    <form
      className={styles.password}
      onSubmit={(event) => {
        event.preventDefault();
        if (!mismatch) change.mutate();
      }}
    >
      <h4 className={styles.subheading}>Password</h4>
      <label className={ui.field}>
        Current password
        <input
          className={ui.input}
          type="password"
          value={current}
          onChange={(event) => setCurrent(event.target.value)}
          autoComplete="current-password"
          required
        />
      </label>
      <label className={ui.field}>
        New password
        <input
          className={ui.input}
          type="password"
          value={next}
          onChange={(event) => setNext(event.target.value)}
          autoComplete="new-password"
          minLength={8}
          required
        />
      </label>
      <label className={ui.field}>
        New password again
        <input
          className={ui.input}
          type="password"
          value={again}
          onChange={(event) => setAgain(event.target.value)}
          autoComplete="new-password"
          aria-invalid={mismatch}
          required
        />
      </label>
      {mismatch && (
        <p className={ui.error} role="alert">
          The new passwords differ.
        </p>
      )}
      <div className={`${ui.actions} ${styles.left}`}>
        <button className={ui.primary} disabled={change.isPending || mismatch}>
          Change password
        </button>
        <Outcome
          mutation={change}
          done="Password changed. Other devices with this account are logged out, and its API keys are deleted."
        />
      </div>
    </form>
  );
}

/** Keys for other programs, such as Musubi, instead of a login. */
function ApiKeys() {
  const client = useQueryClient();
  const keys = useQuery({ queryKey: ["api-keys"], queryFn: () => api<ApiKey[]>("/account/keys") });
  const refresh = () => client.invalidateQueries({ queryKey: ["api-keys"] });
  const [name, setName] = useState("");
  const [deleting, setDeleting] = useState<ApiKey | null>(null);
  const create = useMutation({
    mutationFn: () => api<{ key: ApiKey; secret: string }>("/account/keys", "POST", { name }),
    onSuccess: () => {
      setName("");
      refresh();
    },
  });
  const remove = useMutation({
    mutationFn: (id: number) => api(`/account/keys/${id}`, "DELETE"),
    onSuccess: refresh,
  });
  const date = (seconds: number) => new Date(seconds * 1000).toLocaleDateString();
  const failure = create.error ?? remove.error ?? keys.error;

  return (
    <div className={styles.keys}>
      <h4 className={styles.subheading}>API keys</h4>
      <p className={styles.hint}>
        Other programs, such as Musubi, use a key instead of your name and password. A key can do
        what your account can, except change the password or the keys. Changing your password
        deletes every key.
      </p>
      {deleting && (
        <Confirm
          title={`Delete the key “${deleting.name}”?`}
          text="Programs using it stop working at once."
          action="Delete key"
          danger
          pending={remove.isPending}
          error={remove.error?.message}
          onConfirm={() => remove.mutate(deleting.id, { onSuccess: () => setDeleting(null) })}
          onClose={() => setDeleting(null)}
        />
      )}
      {keys.data && keys.data.length > 0 && (
        <ul className={`${ui.list} ${styles.top}`}>
          {keys.data.map((key) => (
            <li key={key.id} className={`${ui.row} ${styles.entry}`}>
              <span className={ui.grow}>{key.name}</span>
              <span className={ui.muted}>
                {key.last_used_at ? `Used ${date(key.last_used_at)}` : `Created ${date(key.created_at)}`}
              </span>
              <button type="button" className={ui.danger} onClick={() => setDeleting(key)}>
                Delete
              </button>
            </li>
          ))}
        </ul>
      )}
      <form
        className={styles.oneLine}
        onSubmit={(event) => {
          event.preventDefault();
          create.mutate();
        }}
      >
        <label className={ui.field}>
          New key for
          <input
            className={ui.input}
            value={name}
            onChange={(event) => setName(event.target.value)}
            placeholder="Musubi"
            maxLength={64}
            required
          />
        </label>
        <button className={ui.primary} disabled={create.isPending}>
          Create key
        </button>
      </form>
      {create.data && (
        <div className={styles.secret} role="status">
          <p>Copy this key now. Kura keeps only a fingerprint of it and cannot show it again.</p>
          <code className={ui.mono}>{create.data.secret}</code>
        </div>
      )}
      {failure && (
        <p className={ui.error} role="alert">
          {failure.message}
        </p>
      )}
    </div>
  );
}

/** What became of a save: an error, or a short note once it worked. */
function Outcome({
  mutation,
  done,
}: {
  mutation: { isSuccess: boolean; isError: boolean; error: Error | null };
  done: string;
}) {
  if (mutation.isError) {
    return (
      <span className={ui.error} role="alert">
        {mutation.error?.message}
      </span>
    );
  }
  return (
    <span className={ui.muted} role="status">
      {mutation.isSuccess && done}
    </span>
  );
}

/** Music and background, per account. Admins also manage the songs here. */
function ThemeSection({ admin }: { admin: boolean }) {
  const client = useQueryClient();
  const account = useQuery(accountQuery);
  const songs = useQuery(songsQuery);
  const save = useMutation({
    mutationFn: (theme: Theme) => api<Account>("/account/theme", "PUT", theme),
    // Shown at once; the server's answer replaces it.
    onMutate: (theme) => {
      const before = client.getQueryData<Account>(accountQuery.queryKey);
      if (before) client.setQueryData(accountQuery.queryKey, { ...before, theme });
      return before;
    },
    onError: (_, __, before) => before && client.setQueryData(accountQuery.queryKey, before),
    onSuccess: (next) => client.setQueryData(accountQuery.queryKey, next),
  });
  const theme = account.data?.theme;
  if (!theme) return null;
  const set = (part: Partial<Theme>) => save.mutate({ ...theme, ...part });
  /** Applies at once, without asking the server, e.g. while the volume slider moves. */
  const preview = (part: Partial<Theme>) => {
    if (account.data)
      client.setQueryData(accountQuery.queryKey, { ...account.data, theme: { ...theme, ...part } });
  };
  const list = songs.data ?? [];
  const current = list.find((song) => song.id === theme.song) ?? list[0];

  return (
    <section className={styles.form}>
      <h3 className={`${ui.section} ${styles.left}`}>Background music</h3>
      <label className={ui.check}>
        <input
          type="checkbox"
          checked={theme.music}
          onChange={(event) => set({ music: event.target.checked })}
        />
        Play music
      </label>
      <p className={styles.hint}>
        Plays on a loop while Kura is open. Browsers start sound only after the first click or key
        on the page.
        {list.length === 0 && !admin && " An admin has not added any songs yet."}
      </p>
      <div className={styles.musicFields}>
        <label className={ui.field}>
          Song
          <select
            className={ui.input}
            value={current?.id ?? ""}
            disabled={list.length === 0}
            onChange={(event) => set({ song: event.target.value })}
          >
            {list.length === 0 && <option value="">No songs yet</option>}
            {list.map((song) => (
              <option key={song.id} value={song.id}>
                {song.name}
              </option>
            ))}
          </select>
        </label>
        <Volume value={theme.volume} onPreview={(volume) => preview({ volume })} onSave={(volume) => set({ volume })} />
      </div>
      {admin && <Songs />}

      <h3 className={`${ui.section} ${styles.left}`}>ASCII background</h3>
      <label className={ui.check}>
        <input
          type="checkbox"
          checked={theme.ascii}
          onChange={(event) => set({ ascii: event.target.checked })}
        />
        Show a background
      </label>
      <p className={styles.hint}>
        A faint animated scene behind the pages. Pieces with colour keep it, washed out.
      </p>
      <AsciiLibrary theme={theme} onChange={set} />
      {admin && <BackdropLook />}
      {save.isError && (
        <p className={ui.error} role="alert">
          {save.error.message}
        </p>
      )}
    </section>
  );
}

/** The volume slider. It keeps its own value, so it moves at once; the music follows while
 *  it moves, and the value is saved when it is let go. */
function Volume({
  value,
  onPreview,
  onSave,
}: {
  value: number;
  onPreview: (volume: number) => void;
  onSave: (volume: number) => void;
}) {
  const [shown, setShown] = useState(value);
  // The preview changes `value` too, so what was last saved is kept apart.
  const saved = useRef(value);
  const commit = () => {
    if (shown !== saved.current) {
      saved.current = shown;
      onSave(shown);
    }
  };
  return (
    <label className={ui.field}>
      <span>
        Volume <span className={styles.faint}>{shown} %</span>
      </span>
      <input
        className={styles.range}
        type="range"
        min={0}
        max={100}
        step={1}
        value={shown}
        onChange={(event) => {
          setShown(event.target.valueAsNumber);
          onPreview(event.target.valueAsNumber);
        }}
        onPointerUp={commit}
        onKeyUp={commit}
        onBlur={commit}
      />
    </label>
  );
}

/** How text stays readable over the background: one choice for every account. */
function BackdropLook() {
  const client = useQueryClient();
  const settings = useQuery(settingsQuery);
  const save = useMutation({
    mutationFn: (backdrop: Backdrop) =>
      api<Settings>("/settings", "PUT", { ...settings.data, backdrop }),
    onSuccess: (saved) => {
      client.setQueryData(settingsQuery.queryKey, saved);
      // Every page reads the look from the status.
      client.invalidateQueries({ queryKey: ["status"] });
    },
  });
  if (!settings.data) return null;
  const looks: { id: Backdrop; name: string; hint: string }[] = [
    {
      id: "glass",
      name: "Frosted glass",
      hint: "The picture is softened and dimmed behind frosted glass; text sits on it freely.",
    },
    {
      id: "halo",
      name: "Text halo",
      hint: "The picture stays sharp; letters get a dark halo and controls a plain fill.",
    },
  ];
  const current = save.isPending && save.variables ? save.variables : settings.data.backdrop;
  return (
    <fieldset className={styles.looks}>
      <legend>Readability over the background, for all accounts</legend>
      {looks.map((look) => (
        <label key={look.id} className={styles.look}>
          <span className={ui.check}>
            <input
              type="radio"
              name="backdrop"
              checked={current === look.id}
              disabled={save.isPending}
              onChange={() => save.mutate(look.id)}
            />
            {look.name}
          </span>
          <span className={styles.hint}>{look.hint}</span>
        </label>
      ))}
      {save.isError && (
        <p className={ui.error} role="alert">
          {save.error.message}
        </p>
      )}
    </fieldset>
  );
}

/** The account's own list of ascii.rest pieces: pick one, remove one, add one by link. */
function AsciiLibrary({ theme, onChange }: { theme: Theme; onChange: (part: Partial<Theme>) => void }) {
  const [link, setLink] = useState("");
  const [problem, setProblem] = useState("");

  function add(event: FormEvent) {
    event.preventDefault();
    const name = pieceFromLink(link);
    if (!name) {
      const asked = link.trim().replace(/^https?:\/\/(www\.)?ascii\.rest\/?/, "").replace(/\/$/, "");
      setProblem(
        `ascii.rest has no piece called “${asked || link.trim()}”, or it is newer than Kura's copy.`,
      );
      return;
    }
    setProblem("");
    setLink("");
    const library = theme.ascii_library.includes(name) ? theme.ascii_library : [...theme.ascii_library, name];
    onChange({ ascii_library: library, ascii_piece: name, ascii: true });
  }

  return (
    <>
      <ul className={`${ui.list} ${styles.top}`} aria-label="Your backgrounds">
        {theme.ascii_library.map((name) => (
          <li key={name} className={`${ui.row} ${styles.entry}`}>
            <label className={`${ui.check} ${ui.grow}`}>
              <input
                type="radio"
                name="ascii-piece"
                checked={theme.ascii_piece === name}
                onChange={() => onChange({ ascii_piece: name })}
              />
              {pieceTitle(name)}
            </label>
            <a
              className={`${ui.muted} ${styles.link}`}
              href={`https://ascii.rest/${name}/`}
              target="_blank"
              rel="noreferrer"
            >
              ascii.rest
            </a>
            <button
              type="button"
              className={ui.ghost}
              // The one in use stays, so there is always one to show.
              disabled={theme.ascii_piece === name}
              title={theme.ascii_piece === name ? "Pick another one first" : undefined}
              onClick={() =>
                onChange({ ascii_library: theme.ascii_library.filter((other) => other !== name) })
              }
            >
              Remove
            </button>
          </li>
        ))}
      </ul>
      <form className={styles.oneLine} onSubmit={add}>
        <label className={ui.field}>
          Add from ascii.rest
          <input
            className={ui.input}
            value={link}
            onChange={(event) => setLink(event.target.value)}
            placeholder="https://ascii.rest/aurora-fjord/"
            required
          />
        </label>
        <button className={ui.button}>Add</button>
        {problem && (
          <span className={ui.error} role="alert">
            {problem}
          </span>
        )}
      </form>
      <p className={styles.hint}>
        Paste the link of any piece from{" "}
        <a className={styles.link} href="https://ascii.rest/#pieces" target="_blank" rel="noreferrer">
          ascii.rest
        </a>
        . Kura carries its own copy of every piece, so nothing is loaded from there.
      </p>
    </>
  );
}

/** The songs every account picks from; they live on this server only. */
function Songs() {
  const client = useQueryClient();
  const songs = useQuery(songsQuery);
  const refresh = () => client.invalidateQueries({ queryKey: songsQuery.queryKey });
  const upload = useMutation({
    mutationFn: async (file: File) => {
      const name = encodeURIComponent(file.name);
      const response = await fetch(`/api/v1/admin/theme/songs?name=${name}`, {
        method: "POST",
        headers: { "Content-Type": "audio/mpeg" },
        body: file,
      });
      if (!response.ok) {
        const data = await response.json().catch(() => null);
        throw new Error(data?.error ?? `upload failed (${response.status})`);
      }
    },
    onSuccess: refresh,
  });
  const remove = useMutation({
    mutationFn: (id: string) => api(`/admin/theme/songs/${encodeURIComponent(id)}`, "DELETE"),
    onSuccess: () => {
      refresh();
      // An account that picked it falls back to the first song.
      client.invalidateQueries({ queryKey: accountQuery.queryKey });
    },
  });
  const failure = upload.error ?? remove.error;

  return (
    <div className={styles.song}>
      <h4 className={styles.subheading}>Songs</h4>
      <p className={styles.hint}>
        Kura ships without songs. Upload MP3s (at most 20 MB each); they stay on this server and
        every account picks from them.
      </p>
      {songs.data && songs.data.length > 0 && (
        <ul className={ui.list}>
          {songs.data.map((song) => (
            <li key={song.id} className={`${ui.row} ${styles.entry}`}>
              <span className={ui.grow}>{song.name}</span>
              <button
                type="button"
                className={ui.ghost}
                disabled={remove.isPending}
                onClick={() => {
                  if (confirm(`Remove “${song.name}”?`)) remove.mutate(song.id);
                }}
              >
                Remove
              </button>
            </li>
          ))}
        </ul>
      )}
      <div className={`${ui.actions} ${styles.left}`}>
        <label className={ui.button}>
          {upload.isPending ? "Uploading…" : "Add song"}
          <input
            type="file"
            accept="audio/mpeg,.mp3"
            hidden
            disabled={upload.isPending}
            onChange={(event) => {
              const file = event.target.files?.[0];
              if (file) upload.mutate(file);
              event.target.value = "";
            }}
          />
        </label>
      </div>
      {failure && (
        <p className={ui.error} role="alert">
          {failure.message}
        </p>
      )}
    </div>
  );
}

/** Every account and the invites that are out. Adding, inviting, setting a password and
 *  deleting each happen in a centred modal of their own. */
function AdminUsers({ me }: { me: User }) {
  const client = useQueryClient();
  const users = useQuery({ queryKey: ["users"], queryFn: () => api<UserEntry[]>("/users") });
  const mail = useQuery({ queryKey: ["mail"], queryFn: () => api<MailSettings>("/admin/mail") });
  const invites = useQuery({ queryKey: ["invites"], queryFn: () => api<Invite[]>("/invites") });
  const refresh = () => {
    client.invalidateQueries({ queryKey: ["users"] });
    client.invalidateQueries({ queryKey: ["invites"] });
  };
  const [open, setOpen] = useState<"add" | "invite" | null>(null);
  const mailOn = mail.data?.enabled ?? false;

  return (
    <section className={styles.form}>
      <p className={styles.hint}>
        Admins manage settings, users and requests; users browse and ask for titles. Nobody can
        sign up on their own: add an account here, or invite someone by email.
      </p>
      <div className={`${ui.actions} ${styles.left}`}>
        <button type="button" className={ui.primary} onClick={() => setOpen("add")}>
          Add user
        </button>
        <button
          type="button"
          className={ui.button}
          disabled={!mailOn}
          onClick={() => setOpen("invite")}
        >
          Invite by email
        </button>
        {!mailOn && mail.data && (
          <span className={styles.faint}>
            Inviting needs mail: <NavLink to="/settings/administration/mail">set it up</NavLink>.
          </span>
        )}
      </div>
      {users.error && (
        <p className={ui.error} role="alert">
          {users.error.message}
        </p>
      )}
      <ul className={`${ui.list} ${styles.top}`}>
        {users.data?.map((user) => (
          <li key={user.id}>
            <UserRow user={user} me={me} onChanged={refresh} />
          </li>
        ))}
      </ul>

      {invites.data && invites.data.length > 0 && (
        <>
          <h3 className={styles.subheading}>Invites that are out</h3>
          <ul className={ui.list}>
            {invites.data.map((invite) => (
              <InviteRow key={invite.id} invite={invite} onChanged={refresh} />
            ))}
          </ul>
        </>
      )}

      {open === "add" && <AddUser onClose={() => setOpen(null)} onAdded={refresh} />}
      {open === "invite" && <InviteUser onClose={() => setOpen(null)} onSent={refresh} />}
    </section>
  );
}

function UserRow({ user, me, onChanged }: { user: UserEntry; me: User; onChanged: () => void }) {
  const [email, setEmail] = useState(user.email ?? "");
  const [open, setOpen] = useState<"password" | "delete" | null>(null);
  const save = useMutation({
    mutationFn: () => api(`/users/${user.id}/email`, "PUT", { email }),
    onSuccess: onChanged,
  });
  const remove = useMutation({
    mutationFn: () => api(`/users/${user.id}`, "DELETE"),
    onSuccess: () => {
      setOpen(null);
      onChanged();
    },
  });
  const self = user.id === me.id;

  return (
    <details className={styles.userRow}>
      <summary className={styles.userSummary}>
        <span className={ui.grow}>
          {user.name}
          {self && <span className={styles.faint}> (you)</span>}
        </span>
        <span className={styles.userEmail}>{user.email ?? "no email"}</span>
        <span className={ui.muted}>{user.role === "admin" ? "Admin" : "User"}</span>
      </summary>
      <div className={styles.expanded}>
        <form
          className={styles.oneLine}
          onSubmit={(event) => {
            event.preventDefault();
            save.mutate();
          }}
        >
          <label className={ui.field}>
            Email for password reset
            <input
              className={ui.input}
              type="email"
              value={email}
              onChange={(event) => setEmail(event.target.value)}
              autoComplete="off"
              placeholder="name@example.org"
            />
          </label>
          <button
            className={ui.primary}
            disabled={save.isPending || email.trim() === (user.email ?? "")}
          >
            Save email
          </button>
          <Outcome mutation={save} done="Saved." />
        </form>
        <div className={`${ui.actions} ${styles.left}`}>
          {!self && (
            <button type="button" className={ui.button} onClick={() => setOpen("password")}>
              Set password
            </button>
          )}
          <button
            type="button"
            className={ui.danger}
            disabled={self}
            title={self ? "You cannot delete your own account" : undefined}
            onClick={() => setOpen("delete")}
          >
            Delete user
          </button>
        </div>
      </div>
      {open === "password" && <SetPassword user={user} onClose={() => setOpen(null)} />}
      {open === "delete" && (
        <Confirm
          title={`Delete ${user.name}?`}
          text={`${user.name} is logged out everywhere, and their requests and API keys are deleted. This cannot be undone.`}
          action="Delete user"
          danger
          pending={remove.isPending}
          error={remove.error?.message}
          onConfirm={() => remove.mutate()}
          onClose={() => setOpen(null)}
        />
      )}
    </details>
  );
}

/** The two password fields every "choose a password" form has. */
function NewPassword({
  password,
  again,
  onPassword,
  onAgain,
}: {
  password: string;
  again: string;
  onPassword: (value: string) => void;
  onAgain: (value: string) => void;
}) {
  const mismatch = again !== "" && again !== password;
  return (
    <>
      <label className={ui.field}>
        Password
        <input
          className={ui.input}
          type="password"
          value={password}
          onChange={(event) => onPassword(event.target.value)}
          autoComplete="new-password"
          minLength={8}
          required
        />
      </label>
      <label className={ui.field}>
        Password again
        <input
          className={ui.input}
          type="password"
          value={again}
          onChange={(event) => onAgain(event.target.value)}
          autoComplete="new-password"
          aria-invalid={mismatch}
          required
        />
      </label>
      {mismatch && (
        <p className={ui.error} role="alert">
          The passwords differ.
        </p>
      )}
    </>
  );
}

function RolePicker({ role, onChange }: { role: Role; onChange: (role: Role) => void }) {
  return (
    <div className={ui.field}>
      Role
      <div className={`${ui.segmented} ${styles.roles}`} role="group" aria-label="Role">
        {(["user", "admin"] as const).map((value) => (
          <button
            key={value}
            type="button"
            className={ui.segment}
            aria-pressed={role === value}
            onClick={() => onChange(value)}
          >
            {value === "admin" ? "Admin" : "User"}
          </button>
        ))}
      </div>
    </div>
  );
}

function AddUser({ onClose, onAdded }: { onClose: () => void; onAdded: () => void }) {
  const [form, setForm] = useState({ name: "", email: "", password: "", again: "" });
  const [role, setRole] = useState<Role>("user");
  const change = (part: Partial<typeof form>) => setForm({ ...form, ...part });
  const add = useMutation({
    mutationFn: () =>
      api("/users", "POST", { name: form.name, email: form.email, password: form.password, role }),
    onSuccess: () => {
      onAdded();
      onClose();
    },
  });
  const mismatch = form.again !== "" && form.again !== form.password;
  return (
    <Modal
      title="Add a user"
      lead="The account can log in right away with this name and password."
      onClose={onClose}
      locked={add.isPending}
    >
      <form
        onSubmit={(event) => {
          event.preventDefault();
          if (!mismatch) add.mutate();
        }}
      >
        <label className={ui.field}>
          Name
          <input
            className={ui.input}
            value={form.name}
            onChange={(event) => change({ name: event.target.value })}
            autoComplete="off"
            required
            autoFocus
          />
        </label>
        <label className={ui.field}>
          <span>
            Email <span className={styles.faint}>(optional, for password reset)</span>
          </span>
          <input
            className={ui.input}
            type="email"
            value={form.email}
            onChange={(event) => change({ email: event.target.value })}
            autoComplete="off"
            placeholder="name@example.org"
          />
        </label>
        <NewPassword
          password={form.password}
          again={form.again}
          onPassword={(password) => change({ password })}
          onAgain={(again) => change({ again })}
        />
        <RolePicker role={role} onChange={setRole} />
        {add.isError && (
          <p className={ui.error} role="alert">
            {add.error.message}
          </p>
        )}
        <div className={ui.modalActions}>
          <button className={ui.primary} disabled={add.isPending || mismatch}>
            Create account
          </button>
        </div>
      </form>
    </Modal>
  );
}

function SetPassword({ user, onClose }: { user: UserEntry; onClose: () => void }) {
  const [password, setPassword] = useState("");
  const [again, setAgain] = useState("");
  const set = useMutation({
    mutationFn: () => api(`/users/${user.id}/password`, "PUT", { password }),
  });
  const mismatch = again !== "" && again !== password;
  return (
    <Modal
      title={`New password for ${user.name}`}
      lead={`${user.name} is logged out everywhere and their API keys are deleted. Tell them the new password yourself.`}
      onClose={onClose}
      locked={set.isPending}
    >
      {set.isSuccess ? (
        <>
          <p className={styles.done} role="status">
            The password of {user.name} is set.
          </p>
          <div className={ui.modalActions}>
            <button type="button" className={ui.primary} onClick={onClose} autoFocus>
              Done
            </button>
          </div>
        </>
      ) : (
        <form
          onSubmit={(event) => {
            event.preventDefault();
            if (!mismatch) set.mutate();
          }}
        >
          <NewPassword
            password={password}
            again={again}
            onPassword={setPassword}
            onAgain={setAgain}
          />
          {set.isError && (
            <p className={ui.error} role="alert">
              {set.error.message}
            </p>
          )}
          <div className={ui.modalActions}>
            <button className={ui.primary} disabled={set.isPending || mismatch}>
              Set password
            </button>
          </div>
        </form>
      )}
    </Modal>
  );
}

function InviteUser({ onClose, onSent }: { onClose: () => void; onSent: () => void }) {
  const [email, setEmail] = useState("");
  const [role, setRole] = useState<Role>("user");
  const send = useMutation({
    mutationFn: () => api<Invite>("/users/invite", "POST", { email, role }),
    onSuccess: onSent,
  });
  return (
    <Modal
      title="Invite by email"
      lead="Kura sends a link; with it, they choose their own name and password. It works once and for 7 days."
      onClose={onClose}
      locked={send.isPending}
    >
      {send.isSuccess ? (
        <>
          <p className={styles.done} role="status">
            Invite sent to {send.data.email}.
          </p>
          <div className={ui.modalActions}>
            <button type="button" className={ui.primary} onClick={onClose} autoFocus>
              Done
            </button>
          </div>
        </>
      ) : (
        <form
          onSubmit={(event) => {
            event.preventDefault();
            send.mutate();
          }}
        >
          <label className={ui.field}>
            Email
            <input
              className={ui.input}
              type="email"
              value={email}
              onChange={(event) => setEmail(event.target.value)}
              placeholder="name@example.org"
              autoComplete="off"
              required
              autoFocus
            />
          </label>
          <RolePicker role={role} onChange={setRole} />
          {send.isError && (
            <p className={ui.error} role="alert">
              {send.error.message}
            </p>
          )}
          <div className={ui.modalActions}>
            <button className={ui.primary} disabled={send.isPending}>
              {send.isPending ? "Sending…" : "Send invite"}
            </button>
          </div>
        </form>
      )}
    </Modal>
  );
}

function InviteRow({ invite, onChanged }: { invite: Invite; onChanged: () => void }) {
  const [asking, setAsking] = useState(false);
  const revoke = useMutation({
    mutationFn: () => api(`/invites/${invite.id}`, "DELETE"),
    onSuccess: onChanged,
  });
  const until = new Date(invite.expires_at * 1000).toLocaleDateString();
  return (
    <li className={`${ui.row} ${styles.entry}`}>
      <span className={ui.grow}>{invite.email}</span>
      <span className={styles.faint}>until {until}</span>
      <span className={ui.muted}>{invite.role === "admin" ? "Admin" : "User"}</span>
      <button type="button" className={ui.danger} onClick={() => setAsking(true)}>
        Revoke
      </button>
      {asking && (
        <Confirm
          title="Revoke this invite?"
          text={`The link sent to ${invite.email} stops working.`}
          action="Revoke invite"
          danger
          pending={revoke.isPending}
          error={revoke.error?.message}
          onConfirm={() => revoke.mutate()}
          onClose={() => setAsking(false)}
        />
      )}
    </li>
  );
}

const ports: Record<MailEncryption, number> = { starttls: 587, tls: 465, none: 25 };

/** The mail server Kura sends password reset mails through. */
function MailSection() {
  const client = useQueryClient();
  const stored = useQuery({ queryKey: ["mail"], queryFn: () => api<MailSettings>("/admin/mail") });
  if (stored.isError) {
    return (
      <p className={ui.error} role="alert">
        {stored.error.message}
      </p>
    );
  }
  if (!stored.data) return null;
  return (
    <MailForm
      key={JSON.stringify(stored.data)}
      stored={stored.data}
      onSaved={(next) => {
        client.setQueryData(["mail"], next);
        client.invalidateQueries({ queryKey: ["status"] });
      }}
    />
  );
}

function MailForm({
  stored,
  onSaved,
}: {
  stored: MailSettings;
  onSaved: (next: MailSettings) => void;
}) {
  const [draft, setDraft] = useState({ ...stored, password: "" });
  const change = (part: Partial<typeof draft>) => setDraft({ ...draft, ...part });
  const save = useMutation({
    mutationFn: (mail: typeof draft) => api<MailSettings>("/admin/mail", "PUT", mail),
    onSuccess: onSaved,
  });
  const off = useMutation({
    mutationFn: () => api<MailSettings>("/admin/mail", "PUT", { host: "" }),
    onSuccess: onSaved,
  });
  const usernameKept = draft.username === stored.username;
  const [askingOff, setAskingOff] = useState(false);

  function encryption(next: MailEncryption) {
    // The usual port follows, unless someone chose another one.
    const usual = Object.values(ports).includes(draft.port) || draft.port === 0;
    change({ encryption: next, port: usual ? ports[next] : draft.port });
  }

  return (
    <section className={styles.form}>
      {stored.enabled ? (
        <p className={`${ui.notice} ${styles.left}`} role="status">
          Mail is on: the login page offers <em>Forgot password?</em>
        </p>
      ) : (
        <p className={`${ui.notice} ${styles.left}`} role="status">
          Mail is off. Until a mail server is set up here, forgotten passwords are reset by an
          admin under Users.
        </p>
      )}
      <form
        className={styles.mail}
        onSubmit={(event) => {
          event.preventDefault();
          save.mutate(draft);
        }}
      >
        <div className={`${ui.actions} ${styles.left}`}>
          <span className={ui.muted}>Fill in for</span>
          <button
            type="button"
            className={ui.ghost}
            onClick={() => change({ host: "smtp.protonmail.ch", port: 587, encryption: "starttls" })}
          >
            Proton Mail
          </button>
        </div>
        <div className={styles.pair}>
          <label className={ui.field}>
            Mail server
            <input
              className={ui.input}
              value={draft.host}
              onChange={(event) => change({ host: event.target.value })}
              placeholder="smtp.example.org"
              autoComplete="off"
              spellCheck={false}
              required
            />
          </label>
          <label className={ui.field}>
            Port
            <input
              className={`${ui.input} ${styles.short}`}
              type="number"
              min={1}
              max={65535}
              value={draft.port || ""}
              onChange={(event) => change({ port: Number(event.target.value) })}
              required
            />
          </label>
        </div>
        <label className={ui.field}>
          Encryption
          <select
            className={ui.input}
            value={draft.encryption}
            onChange={(event) => encryption(event.target.value as MailEncryption)}
          >
            <option value="starttls">STARTTLS (port 587, usual)</option>
            <option value="tls">TLS (port 465)</option>
            <option value="none">None (only a relay in your own network)</option>
          </select>
        </label>
        {draft.encryption === "none" && (
          <p className={styles.hint}>
            Without encryption Kura sends no password, so this only works with a relay that does
            not ask for one.
          </p>
        )}
        <label className={ui.field}>
          User name
          <input
            className={ui.input}
            value={draft.username}
            onChange={(event) => change({ username: event.target.value })}
            placeholder="noreply@example.org"
            autoComplete="off"
            spellCheck={false}
          />
        </label>
        <label className={ui.field}>
          Password or app token
          <input
            className={ui.input}
            type="password"
            value={draft.password}
            onChange={(event) => change({ password: event.target.value })}
            placeholder={
              stored.password_set && usernameKept ? "Stored; leave empty to keep it" : ""
            }
            autoComplete="new-password"
          />
        </label>
        <p className={styles.hint}>
          Proton Mail: create an SMTP token under Settings → IMAP/SMTP (paid plans); the address
          it belongs to is the user name, the token the password.
        </p>
        <label className={ui.field}>
          Sender
          <input
            className={ui.input}
            value={draft.from}
            onChange={(event) => change({ from: event.target.value })}
            placeholder="Kura <noreply@example.org>"
            autoComplete="off"
            spellCheck={false}
            required
          />
        </label>
        <label className={ui.field}>
          Kura's address
          <span className={styles.oneLine}>
            <input
              className={ui.input}
              type="url"
              value={draft.public_url}
              onChange={(event) => change({ public_url: event.target.value })}
              placeholder="https://kura.example.org"
              spellCheck={false}
              required
            />
            <button
              type="button"
              className={ui.ghost}
              onClick={() => change({ public_url: window.location.origin })}
            >
              Use this one
            </button>
          </span>
        </label>
        <p className={styles.hint}>
          Links in mails start with this address, so it must be the one people open Kura under.
        </p>
        <div className={`${ui.actions} ${styles.left}`}>
          <button className={ui.primary} disabled={save.isPending}>
            Save
          </button>
          {stored.host && (
            <button
              type="button"
              className={ui.danger}
              disabled={off.isPending}
              onClick={() => setAskingOff(true)}
            >
              Switch mail off
            </button>
          )}
          <Outcome mutation={save} done="Saved." />
        </div>
        {off.isError && (
          <p className={ui.error} role="alert">
            {off.error.message}
          </p>
        )}
      </form>
      {stored.enabled && <TestMail />}
      {askingOff && (
        <Confirm
          title="Switch mail off?"
          text="Kura forgets the mail server and its password. Password reset and invites stop until mail is set up again."
          action="Switch mail off"
          danger
          pending={off.isPending}
          error={off.error?.message}
          onConfirm={() => off.mutate(undefined, { onSuccess: () => setAskingOff(false) })}
          onClose={() => setAskingOff(false)}
        />
      )}
    </section>
  );
}

function TestMail() {
  const account = useQuery(accountQuery);
  const [to, setTo] = useState<string | null>(null);
  const address = to ?? account.data?.email ?? "";
  const send = useMutation({ mutationFn: () => api("/admin/mail/test", "POST", { to: address }) });
  return (
    <details className={styles.expander} open>
      <summary className={styles.summary}>Send a test mail</summary>
      <form
        className={`${styles.oneLine} ${styles.expanded}`}
        onSubmit={(event) => {
          event.preventDefault();
          send.mutate();
        }}
      >
        <label className={ui.field}>
          To
          <input
            className={ui.input}
            type="email"
            value={address}
            onChange={(event) => setTo(event.target.value)}
            placeholder="you@example.org"
            required
          />
        </label>
        <button className={ui.primary} disabled={send.isPending}>
          {send.isPending ? "Sending…" : "Send"}
        </button>
        <Outcome mutation={send} done={`Sent. Look in the inbox of ${address}.`} />
      </form>
    </details>
  );
}
