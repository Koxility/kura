import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { type ChangeEvent, type FormEvent, useState } from "react";
import { NavLink, useParams } from "react-router";
import {
  type Account,
  accountQuery,
  type ApiKey,
  api,
  hosterNames,
  type Jellyfin as JellyfinSettings,
  type Language,
  type Library,
  languageNames,
  languages,
  type Settings,
  type Site,
  settingsQuery,
  siteNames,
  sites,
  supportedHosters,
  statusQuery,
  type Theme,
  type User,
} from "../api";
import { type AsciiScene, asciiScenes } from "../ThemeEffects";
import ui from "../ui.module.css";
import styles from "./Settings.module.css";

/** Server settings are for admins; everyone has their own account and theme. */
const sections = [
  { id: "downloads", title: "Downloads", admin: true },
  { id: "library", title: "Library", admin: true },
  { id: "sites", title: "Sites", admin: true },
  { id: "auto-sync", title: "Auto-Sync", admin: true },
  { id: "jellyfin", title: "Jellyfin", admin: true },
  { id: "users", title: "Users", userTitle: "User" },
  { id: "theme", title: "Theme" },
  { id: "administration", title: "Administration", admin: true },
];

export function SettingsPage({ user }: { user: User }) {
  const admin = user.role === "admin";
  const settings = useQuery({ ...settingsQuery, enabled: admin });
  const shown = sections
    .filter((section) => admin || !section.admin)
    .map((section) => ({
      ...section,
      title: !admin && section.userTitle ? section.userTitle : section.title,
    }));
  const section = shown.find((section) => section.id === useParams().section) ?? shown[0];

  return (
    <>
      <h1>Settings</h1>
      <div className={styles.layout}>
        <nav className={styles.sections} aria-label="Settings sections">
          {shown.map((section) => (
            <NavLink key={section.id} to={`/settings/${section.id}`} className={styles.section}>
              {section.title}
            </NavLink>
          ))}
        </nav>
        <div className={styles.content}>
          <h2 className={styles.heading}>{section.title}</h2>
          {settings.isError && (
            <p className={ui.error} role="alert">
              {settings.error.message}
            </p>
          )}
          {section.id === "users" ? (
            <>
              <AccountSection />
              {admin && <Users me={user} />}
            </>
          ) : section.id === "theme" ? (
            <ThemeSection admin={admin} />
          ) : (
            settings.data && (
              // Remounted per section, so leaving one drops its unsaved changes.
              <SettingsForm
                key={section.id}
                initial={settings.data}
                section={section.id}
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
        {section === "administration" && (
          <Administration settings={initial} onImport={(imported) => save.mutate(imported)} />
        )}
        {!readOnly && section !== "administration" && (
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
        {section === "administration" && save.isSuccess && (
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
        On: only episodes after the newest downloaded one. Off: missing earlier episodes are
        downloaded too.
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
          placeholder="From Jellyfin: Dashboard, API Keys"
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

function Administration({
  settings,
  onImport,
}: {
  settings: Settings;
  onImport: (settings: Settings) => void;
}) {
  const [problem, setProblem] = useState("");
  const [ipShown, setIpShown] = useState(false);
  const ip = useMutation({ mutationFn: () => api<{ ip: string }>("/admin/ip") });

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
      <h2 className={`${ui.section} ${styles.left}`}>Configuration</h2>
      <p className={styles.hint}>
        The file holds all settings, libraries and Auto-Sync exclusions. Accounts and passwords are
        not part of it.
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

      <h2 className={`${ui.section} ${styles.left}`}>IP check</h2>
      {!ipShown ? (
        <button type="button" className={ui.button} onClick={() => setIpShown(true)}>
          Show IP check
        </button>
      ) : (
        <>
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
        </>
      )}
    </>
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
      <h3 className={`${ui.section} ${styles.left}`}>Your account</h3>
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
      <button className={ui.button} disabled={rename.isPending || name.trim() === account.name}>
        Change name
      </button>
      <Outcome mutation={rename} done="Name changed." />
    </form>
  );
}

function EmailForm({ account, onSaved }: { account: Account; onSaved: (next: Account) => void }) {
  const [email, setEmail] = useState(account.email ?? "");
  const save = useMutation({
    mutationFn: () => api<Account>("/account/email", "PUT", { email }),
    onSuccess: onSaved,
  });
  return (
    <form
      className={styles.oneLine}
      onSubmit={(event) => {
        event.preventDefault();
        save.mutate();
      }}
    >
      <label className={ui.field}>
        <span>
          Email <span className={styles.faint}>(optional)</span>
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
      <button className={ui.button} disabled={save.isPending || email.trim() === (account.email ?? "")}>
        Save email
      </button>
      <Outcome mutation={save} done="Saved. Kura does not send anything to it yet." />
    </form>
  );
}

function PasswordForm() {
  const [current, setCurrent] = useState("");
  const [next, setNext] = useState("");
  const [again, setAgain] = useState("");
  const change = useMutation({
    mutationFn: () => api("/account/password", "PUT", { current, new: next }),
    onSuccess: () => {
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
        <button className={ui.button} disabled={change.isPending || mismatch}>
          Change password
        </button>
        <Outcome
          mutation={change}
          done="Password changed. Other devices with this account are logged out."
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
        what your account can.
      </p>
      {keys.data && keys.data.length > 0 && (
        <ul className={`${ui.list} ${styles.top}`}>
          {keys.data.map((key) => (
            <li key={key.id} className={`${ui.row} ${styles.entry}`}>
              <span className={ui.grow}>{key.name}</span>
              <span className={ui.muted}>
                {key.last_used_at ? `Used ${date(key.last_used_at)}` : `Created ${date(key.created_at)}`}
              </span>
              <button
                type="button"
                className={ui.ghost}
                onClick={() => {
                  if (confirm(`Delete the key “${key.name}”? Programs using it stop working.`))
                    remove.mutate(key.id);
                }}
              >
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
        <button className={ui.button} disabled={create.isPending}>
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

/** Music and background, per user. Admins also upload the song here. */
function ThemeSection({ admin }: { admin: boolean }) {
  const client = useQueryClient();
  const account = useQuery(accountQuery);
  const status = useQuery(statusQuery);
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
  const hasSong = status.data?.music ?? false;

  return (
    <section className={styles.form}>
      <label className={`${ui.check} ${styles.toggle}`}>
        <input
          type="checkbox"
          checked={theme.music}
          onChange={(event) => set({ music: event.target.checked })}
        />
        Wii Shop Channel Main Theme
      </label>
      <p className={styles.hint}>
        Plays on a loop while Kura is open. Browsers start sound only after the first click or key
        on the page.
        {!hasSong && !admin && " An admin has not added the song yet."}
      </p>
      {admin && <SongUpload hasSong={hasSong} />}

      <label className={`${ui.check} ${styles.toggle}`}>
        <input
          type="checkbox"
          checked={theme.ascii}
          onChange={(event) => set({ ascii: event.target.checked })}
        />
        ASCII background
      </label>
      <p className={styles.hint}>
        A faint animated scene behind the pages, from{" "}
        <a className={styles.link} href="https://ascii.rest/#pieces" target="_blank" rel="noreferrer">
          ascii.rest
        </a>
        .
      </p>
      <label className={`${ui.field} ${styles.top}`}>
        Scene
        <select
          className={ui.input}
          value={theme.ascii_piece}
          disabled={!theme.ascii}
          onChange={(event) => set({ ascii_piece: event.target.value })}
        >
          {(Object.keys(asciiScenes) as AsciiScene[]).map((id) => (
            <option key={id} value={id}>
              {asciiScenes[id].name}
            </option>
          ))}
        </select>
      </label>
      {save.isError && (
        <p className={ui.error} role="alert">
          {save.error.message}
        </p>
      )}
    </section>
  );
}

/** The song lives on this server only; Kura ships none of its own. */
function SongUpload({ hasSong }: { hasSong: boolean }) {
  const client = useQueryClient();
  const refresh = () => client.invalidateQueries({ queryKey: ["status"] });
  const upload = useMutation({
    mutationFn: async (file: File) => {
      const response = await fetch("/api/v1/admin/theme/music", {
        method: "PUT",
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
    mutationFn: () => api("/admin/theme/music", "DELETE"),
    onSuccess: refresh,
  });
  const failure = upload.error ?? remove.error;

  return (
    <div className={styles.song}>
      <p className={styles.hint}>
        {hasSong
          ? "A song is uploaded. It stays on this server."
          : "Kura ships without the song. Upload an MP3 of it (at most 20 MB); it stays on this server."}
      </p>
      <div className={`${ui.actions} ${styles.left}`}>
        <label className={ui.button}>
          {upload.isPending ? "Uploading…" : hasSong ? "Replace song" : "Upload song"}
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
        {hasSong && (
          <button
            type="button"
            className={ui.ghost}
            disabled={remove.isPending}
            onClick={() => remove.mutate()}
          >
            Remove
          </button>
        )}
      </div>
      {failure && (
        <p className={ui.error} role="alert">
          {failure.message}
        </p>
      )}
    </div>
  );
}

function Users({ me }: { me: User }) {
  const client = useQueryClient();
  const users = useQuery({ queryKey: ["users"], queryFn: () => api<User[]>("/users") });
  const refresh = () => client.invalidateQueries({ queryKey: ["users"] });
  const add = useMutation({
    mutationFn: (form: Record<string, FormDataEntryValue>) => api("/users", "POST", form),
    onSuccess: refresh,
  });
  const remove = useMutation({
    mutationFn: (id: number) => api(`/users/${id}`, "DELETE"),
    onSuccess: refresh,
  });

  function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const form = event.currentTarget;
    add.mutate(Object.fromEntries(new FormData(form)), { onSuccess: () => form.reset() });
  }
  const failure = add.error ?? remove.error ?? users.error;

  return (
    <section className={`${styles.form} ${styles.top}`}>
      <h3 className={`${ui.section} ${styles.left}`}>All users</h3>
      <ul className={`${ui.list} ${styles.top}`}>
        {users.data?.map((user) => (
          <li key={user.id} className={`${ui.row} ${styles.entry}`}>
            <span className={ui.grow}>{user.name}</span>
            <span className={ui.muted}>{user.role === "admin" ? "Admin" : "User"}</span>
            <button
              type="button"
              className={ui.ghost}
              disabled={user.id === me.id}
              aria-label={`Delete ${user.name}`}
              onClick={() => {
                if (confirm(`Delete ${user.name}? This cannot be undone.`)) remove.mutate(user.id);
              }}
            >
              Delete
            </button>
          </li>
        ))}
      </ul>
      <form className={styles.newUser} onSubmit={onSubmit}>
        <label className={ui.field}>
          Name
          <input className={ui.input} name="name" autoComplete="off" required />
        </label>
        <label className={ui.field}>
          Password
          <input
            className={ui.input}
            name="password"
            type="password"
            autoComplete="new-password"
            minLength={8}
            required
          />
        </label>
        <label className={ui.field}>
          Role
          <select className={ui.input} name="role" defaultValue="user">
            <option value="user">User</option>
            <option value="admin">Admin</option>
          </select>
        </label>
        <button className={ui.button} disabled={add.isPending}>
          Add user
        </button>
      </form>
      {failure && (
        <p className={ui.error} role="alert">
          {failure.message}
        </p>
      )}
    </section>
  );
}
