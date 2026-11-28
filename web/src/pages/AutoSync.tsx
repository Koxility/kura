import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { Link } from "react-router";
import {
  api,
  episodeCode,
  formatTime,
  languageNames,
  settingsQuery,
  siteNames,
  type SyncReport,
  type SyncStatus,
  type TitleRef,
  type TrackedTitle,
} from "../api";
import { Flag } from "../Flag";
import ui from "../ui.module.css";
import styles from "./AutoSync.module.css";

const same = (a: TitleRef, b: TitleRef) => a.site === b.site && a.slug === b.slug;

export function AutoSyncPage() {
  const client = useQueryClient();
  const status = useQuery({
    queryKey: ["autosync"],
    queryFn: () => api<SyncStatus>("/autosync"),
    // Results come from the server's own schedule, so the page looks again by itself.
    refetchInterval: 30_000,
  });
  const settings = useQuery(settingsQuery);
  const [term, setTerm] = useState("");
  const exclude = useMutation({
    mutationFn: ({ title, excluded }: { title: TitleRef; excluded: boolean }) =>
      api("/autosync/exclude", "POST", { ...title, excluded }),
    onSettled: () => {
      client.invalidateQueries({ queryKey: ["settings"] });
      client.invalidateQueries({ queryKey: ["autosync"] });
    },
  });

  const remove = useMutation({
    mutationFn: (title: TitleRef) => api("/autosync/remove", "POST", title),
    onSettled: () => {
      client.invalidateQueries({ queryKey: ["settings"] });
      client.invalidateQueries({ queryKey: ["autosync"] });
    },
  });

  const syncNow = useMutation({
    mutationFn: () => api<{ queued: number }>("/autosync/run", "POST"),
    onSettled: () => {
      client.invalidateQueries({ queryKey: ["autosync"] });
      client.invalidateQueries({ queryKey: ["queue"] });
    },
  });

  const tracked = status.data?.titles ?? [];
  const excluded = settings.data?.auto_sync.excluded ?? [];
  const wanted = term.trim().toLowerCase();
  // Searching offers tracked titles to exclude; excluded ones are always listed.
  const found = wanted ? tracked.filter((title) => title.title.toLowerCase().includes(wanted)) : [];
  const stillExcluded = excluded.filter((title) => title.title.toLowerCase().includes(wanted));

  return (
    <>
      <h1>Auto-Sync</h1>
      <div className={styles.bar}>
        <span className={ui.muted} role="status">
          {syncNow.isSuccess &&
            (syncNow.data.queued === 0
              ? "Checked every series in full. Nothing missing."
              : `Checked every series in full. ${syncNow.data.queued} ${
                  syncNow.data.queued === 1 ? "episode" : "episodes"
                } queued.`)}
          {syncNow.isError && <span className={ui.bad}>{syncNow.error.message}</span>}
        </span>
        <button
          type="button"
          className={ui.primary}
          disabled={syncNow.isPending || tracked.length === 0}
          title={
            tracked.length === 0
              ? "Nothing is tracked yet"
              : "Checks every series in full, also those not in the list of new episodes"
          }
          onClick={() => syncNow.mutate()}
        >
          {syncNow.isPending ? "Syncing…" : "Sync now"}
        </button>
      </div>
      {status.data && (
        <p className={ui.lead}>
          {status.data.enabled
            ? [
                "Auto-Sync is on.",
                status.data.last_run && `Last check ${formatTime(status.data.last_run)}.`,
                status.data.next_run && `Next check ${formatTime(status.data.next_run)}.`,
              ]
                .filter(Boolean)
                .join(" ")
            : "Auto-Sync is off. An admin can switch it on in the settings."}{" "}
          <Link to="/settings/auto-sync" className={ui.muted}>
            Settings
          </Link>
          {status.data.last_report && (
            <>
              <br />
              <LastRun report={status.data.last_report} />
            </>
          )}
        </p>
      )}
      {(status.error ?? exclude.error ?? remove.error) && (
        <p className={ui.error} role="alert">
          {(status.error ?? exclude.error ?? remove.error)?.message}
        </p>
      )}

      <h2 className={ui.section}>Tracked titles</h2>
      {status.isSuccess && tracked.length === 0 && (
        <p className={ui.notice}>
          Nothing is tracked. A title is tracked once one of its episodes is downloaded.
        </p>
      )}
      <ul className={styles.cards}>
        {tracked.map((title) => (
          <TitleCard
            key={`${title.site}/${title.slug}`}
            title={title}
            removing={remove.isPending}
            onRemove={() => remove.mutate({ site: title.site, slug: title.slug, title: title.title })}
          />
        ))}
      </ul>

      <h2 className={ui.section}>Exclude titles</h2>
      <div className={ui.toolbar}>
        <input
          className={`${ui.input} ${ui.compact}`}
          type="search"
          value={term}
          onChange={(event) => setTerm(event.target.value)}
          placeholder="Search tracked titles"
          aria-label="Search tracked titles to exclude"
        />
      </div>
      {wanted && found.length === 0 && stillExcluded.length === 0 && (
        <p className={ui.notice}>No tracked title matches.</p>
      )}
      {!wanted && excluded.length === 0 && (
        <p className={ui.notice}>No title is excluded. Search for one to exclude it.</p>
      )}
      <ul className={ui.list}>
        {found.map((title) => (
          <li key={`${title.site}/${title.slug}`} className={ui.row}>
            <span className={ui.grow}>
              {title.title} <span className={ui.muted}>· {siteNames[title.site]}</span>
            </span>
            <button
              type="button"
              className={ui.button}
              disabled={exclude.isPending}
              aria-label={`Exclude ${title.title}`}
              onClick={() => exclude.mutate({ title, excluded: true })}
            >
              Exclude
            </button>
          </li>
        ))}
        {stillExcluded
          .filter((title) => !found.some((other) => same(other, title)))
          .map((title) => (
            <li key={`${title.site}/${title.slug}`} className={ui.row}>
              <span className={ui.grow}>
                {title.title} <span className={ui.muted}>· {siteNames[title.site]} · Excluded</span>
              </span>
              <button
                type="button"
                className={ui.button}
                disabled={exclude.isPending}
                aria-label={`Include ${title.title} again`}
                onClick={() => exclude.mutate({ title, excluded: false })}
              >
                Include again
              </button>
            </li>
          ))}
      </ul>
    </>
  );
}

type Tone = "ok" | "bad" | "warn" | "idle";

/** The state of a title in a word or two, and its colour. */
function state(title: TrackedTitle): { tone: Tone; short: string } {
  const check = title.check;
  if (!check) return { tone: "idle", short: "Not checked yet" };
  if (check.error) return { tone: "bad", short: "Check failed" };
  if (check.queued.length > 0) return { tone: "ok", short: `${check.queued.length} queued` };
  if (check.waiting.length > 0) return { tone: "warn", short: `${check.waiting.length} waiting` };
  return { tone: "idle", short: "Nothing new" };
}

const onDisk = (title: TrackedTitle) =>
  `${title.on_disk} ${title.on_disk === 1 ? "episode" : "episodes"} on disk`;

function Dot({ tone }: { tone: Tone }) {
  return <span className={`${styles.dot} ${styles[tone]}`} aria-hidden="true" />;
}

/** One line on what the last run did. */
function LastRun({ report }: { report: SyncReport }) {
  const episodes = `${report.queued} ${report.queued === 1 ? "episode" : "episodes"} queued`;
  const failed = report.failed > 0 ? `, ${report.failed} failed` : "";
  // "series" is the same in the plural.
  const series = (count: number) => `${count} series`;
  const text =
    report.trigger === "feed"
      ? `Last run: ${series(report.checked)} in the list of new episodes, ${episodes}${failed}.`
      : `Last run (Sync now): ${series(report.checked)} checked in full, ${episodes}${failed}.`;
  return (
    <>
      {text}
      {report.error && <span className={ui.bad}> {report.error}.</span>}
    </>
  );
}

/** Episodes that wait for the title's language, one per line. */
function WaitingList({ title }: { title: TrackedTitle }) {
  const waiting = title.check?.waiting ?? [];
  if (waiting.length === 0) return null;
  const languages = title.languages.map((language) => languageNames[language]).join(" or ");
  return (
    <div className={styles.waiting}>
      <div className={styles.label}>Waiting for {languages || "a language"}</div>
      <ul>
        {waiting.map(({ episode, offered }) => (
          <li key={episode.join("-")}>
            <span className={ui.mono}>{episodeCode(...episode)}</span>
            <span className={ui.muted}>
              {offered.length > 0
                ? `offered: ${offered.map((offer) => languageNames[offer]).join(", ")}`
                : "not offered in any language yet"}
            </span>
          </li>
        ))}
      </ul>
    </div>
  );
}

/** The last check: what it did, and when. */
function LastCheck({ title }: { title: TrackedTitle }) {
  const check = title.check;
  if (!check) return <p className={styles.status}>Not checked yet</p>;
  return (
    <div className={styles.status}>
      <p className={check.error ? ui.bad : undefined}>{check.summary}</p>
      <p className={ui.muted}>
        Checked {formatTime(check.checked_at)},{" "}
        {check.trigger === "feed" ? "from the list of new episodes" : "by Sync now"}
      </p>
    </div>
  );
}

interface TitleProps {
  title: TrackedTitle;
  removing: boolean;
  onRemove: () => void;
}

function RemoveButton({ title, removing, onRemove }: TitleProps) {
  return (
    <button
      type="button"
      className={ui.ghost}
      disabled={removing}
      title="Stop checking this title. It is tracked again once another episode of it is downloaded."
      aria-label={`Remove ${title.title} from Auto-Sync`}
      onClick={onRemove}
    >
      Remove
    </button>
  );
}

/** Language, episodes on disk and the site's newest episode, small, under the name. */
function Chips({ title }: { title: TrackedTitle }) {
  return (
    <span className={styles.chips}>
      {title.languages.map((language) => (
        <span key={language} className={`${ui.tag} ${styles.chip}`}>
          <Flag language={language} /> {languageNames[language]}
        </span>
      ))}
      <span className={`${ui.tag} ${styles.chip}`}>
        {onDisk(title)}
        {title.newest_on_disk && ` · up to ${episodeCode(...title.newest_on_disk)}`}
      </span>
      {title.check?.newest_listed && (
        <span className={`${ui.tag} ${styles.chip}`}>
          Site: up to {episodeCode(...title.check.newest_listed)}
        </span>
      )}
    </span>
  );
}

/** A title on a card of its own: name, language and what is on disk at a glance; a click
 *  shows the site, the last check, the waiting episodes and the actions. Cards with a failed check or
 *  waiting episodes start open. */
function TitleCard(props: TitleProps) {
  const { title } = props;
  const { tone, short } = state(title);
  const [open, setOpen] = useState(tone === "bad" || (title.check?.waiting.length ?? 0) > 0);
  return (
    <li className={styles.card}>
      <button
        type="button"
        className={styles.head}
        aria-expanded={open}
        onClick={() => setOpen(!open)}
      >
        <span className={styles.chevron} aria-hidden="true">
          {open ? "▾" : "▸"}
        </span>
        <span className={ui.grow}>
          <span className={styles.name}>{title.title}</span>
          <Chips title={title} />
        </span>
        <span className={styles.state}>
          <Dot tone={tone} />
          {short}
        </span>
      </button>
      <div className={styles.body}>
        {open && (
          <div className={styles.details}>
            <p className={`${ui.muted} ${styles.siteLine}`}>On {siteNames[title.site]}</p>
            <LastCheck title={title} />
            <WaitingList title={title} />
            <div className={styles.detailActions}>
              <Link to={`/series/${title.site}/${title.slug}`} className={ui.button}>
                Open title
              </Link>
              <RemoveButton {...props} />
            </div>
          </div>
        )}
      </div>
    </li>
  );
}
