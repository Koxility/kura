import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { Link } from "react-router";
import { api, formatTime, settingsQuery, siteNames, type SyncStatus, type TitleRef } from "../api";
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
              ? "Checked every title. Nothing new."
              : `Checked every title. ${syncNow.data.queued} new ${
                  syncNow.data.queued === 1 ? "episode" : "episodes"
                } queued.`)}
          {syncNow.isError && <span className={ui.bad}>{syncNow.error.message}</span>}
        </span>
        <button
          type="button"
          className={ui.primary}
          disabled={syncNow.isPending || tracked.length === 0}
          title={tracked.length === 0 ? "Nothing is tracked yet" : undefined}
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
        </p>
      )}
      {(status.error ?? exclude.error) && (
        <p className={ui.error} role="alert">
          {(status.error ?? exclude.error)?.message}
        </p>
      )}

      <h2 className={ui.section}>Tracked titles</h2>
      {status.isSuccess && tracked.length === 0 && (
        <p className={ui.notice}>
          {excluded.length > 0
            ? "No title is tracked. Removed titles are listed under Exclude titles."
            : "Nothing is tracked yet. A title is tracked once one of its episodes is downloaded."}
        </p>
      )}
      <ul className={ui.list}>
        {tracked.map((title) => (
          <li key={`${title.site}/${title.slug}`} className={ui.row}>
            <div className={ui.grow}>
              <Link to={`/series/${title.site}/${title.slug}`}>{title.title}</Link>{" "}
              <span className={ui.muted}>
                · {siteNames[title.site]} · {title.downloaded}{" "}
                {title.downloaded === 1 ? "episode" : "episodes"}
              </span>
            </div>
            <span className={ui.muted}>
              {title.result ?? "Not checked yet"}
              {title.checked_at && ` · ${formatTime(title.checked_at)}`}
            </span>
            <button
              type="button"
              className={ui.ghost}
              disabled={exclude.isPending}
              title="Stop checking this title. It moves to the excluded titles below."
              aria-label={`Remove ${title.title} from Auto-Sync`}
              onClick={() =>
                exclude.mutate({
                  title: { site: title.site, slug: title.slug, title: title.title },
                  excluded: true,
                })
              }
            >
              Remove
            </button>
          </li>
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
