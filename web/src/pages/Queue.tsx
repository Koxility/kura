import { useMutation, useQuery } from "@tanstack/react-query";
import { useState } from "react";
import {
  api,
  episodeCode,
  formatBytes,
  type Job,
  type JobState,
  jobsQuery,
  languageLabel,
} from "../api";
import ui from "../ui.module.css";
import styles from "./Queue.module.css";

/** All queued episodes of one title, shown as one entry. */
interface Group {
  key: string;
  title: string;
  jobs: Job[];
  state: GroupState;
}

/** What a title's downloads add up to; the filters work on this. */
type GroupState = "active" | "done" | "failed" | "cancelled";

const filters: { id: "all" | GroupState; title: string }[] = [
  { id: "all", title: "All" },
  { id: "active", title: "Active" },
  { id: "done", title: "Done" },
  { id: "failed", title: "Failed" },
  { id: "cancelled", title: "Cancelled" },
];

const sorts = {
  queue: "Queue order",
  newest: "Newest first",
  oldest: "Oldest first",
};
type Sort = keyof typeof sorts;

const stateNames: Record<JobState, string> = {
  running: "Downloading",
  queued: "Waiting",
  failed: "Failed",
  cancelled: "Cancelled",
  done: "Done",
};
const stateStyles: Partial<Record<JobState, string>> = {
  queued: ui.warn,
  failed: ui.bad,
  done: ui.ok,
};

const code = (job: Job) => episodeCode(job.episode.season, job.episode.number);
/** Running episodes named in a title's summary; more are counted, so the line stays short. */
const NAMED = 3;
const isActive = (job: Job) => job.state === "queued" || job.state === "running";

function groupState(jobs: Job[]): GroupState {
  if (jobs.some(isActive)) return "active";
  if (jobs.some((job) => job.state === "failed")) return "failed";
  if (jobs.every((job) => job.state === "cancelled")) return "cancelled";
  return "done";
}

function group(jobs: Job[]): Group[] {
  const groups = new Map<string, Job[]>();
  for (const job of jobs) {
    const key = `${job.episode.site}/${job.episode.slug}`;
    groups.set(key, [...(groups.get(key) ?? []), job]);
  }
  return [...groups].map(([key, jobs]) => {
    jobs.sort((a, b) => a.episode.season - b.episode.season || a.episode.number - b.episode.number);
    return { key, title: jobs[0].series_title, jobs, state: groupState(jobs) };
  });
}

/** Everyone sees the queue; only admins change it. */
export function Queue({ admin }: { admin: boolean }) {
  const jobs = useQuery(jobsQuery);
  const [filter, setFilter] = useState<"all" | GroupState>("all");
  const [sort, setSort] = useState<Sort>("queue");
  const [term, setTerm] = useState("");
  const [open, setOpen] = useState<string[]>([]);
  const act = useMutation({
    mutationFn: (paths: string[]) => Promise.all(paths.map((path) => api(`/queue/${path}`, "POST"))),
  });

  // Cleared entries stay known to the server but are gone from this page.
  const visible = (jobs.data ?? []).filter((job) => !job.hidden);
  const wanted = term.trim().toLowerCase();
  const first = (group: Group) => Math.min(...group.jobs.map((job) => job.queued_at));
  const last = (group: Group) => Math.max(...group.jobs.map((job) => job.queued_at));
  const firstId = (group: Group) => Math.min(...group.jobs.map((job) => job.id));
  const shown = group(visible)
    .filter((group) => filter === "all" || group.state === filter)
    .filter((group) => group.title.toLowerCase().includes(wanted))
    .sort((a, b) => {
      if (sort === "newest") return last(b) - last(a);
      if (sort === "oldest") return first(a) - first(b);
      // Titles with a running download come first, then the order they were queued in.
      const running = (group: Group) => Number(group.jobs.some((job) => job.state === "running"));
      return running(b) - running(a) || firstId(a) - firstId(b);
    });
  const finished = visible.some((job) => !isActive(job));
  const toggle = (key: string) =>
    setOpen(open.includes(key) ? open.filter((other) => other !== key) : [...open, key]);

  return (
    <>
      <h1>Queue</h1>
      <div className={ui.toolbar}>
        <div className={ui.segmented} role="group" aria-label="Show">
          {filters.map(({ id, title }) => (
            <button
              key={id}
              type="button"
              className={ui.segment}
              aria-pressed={id === filter}
              onClick={() => setFilter(id)}
            >
              {title}
            </button>
          ))}
        </div>
        <input
          className={`${ui.input} ${ui.compact}`}
          type="search"
          value={term}
          onChange={(event) => setTerm(event.target.value)}
          placeholder="Search the queue"
          aria-label="Search the queue"
        />
        <select
          className={`${ui.input} ${ui.compact}`}
          value={sort}
          onChange={(event) => setSort(event.target.value as Sort)}
          aria-label="Sort order"
        >
          {(Object.keys(sorts) as Sort[]).map((key) => (
            <option key={key} value={key}>
              {sorts[key]}
            </option>
          ))}
        </select>
        {admin && (
          <button
            type="button"
            className={ui.button}
            disabled={!finished || act.isPending}
            onClick={() => act.mutate(["clear"])}
          >
            Clear Finished
          </button>
        )}
      </div>

      {(jobs.error ?? act.error) && (
        <p className={ui.error} role="alert">
          {(jobs.error ?? act.error)?.message}
        </p>
      )}
      {jobs.isSuccess && shown.length === 0 && (
        <p className={ui.notice}>
          {visible.length === 0
            ? "The queue is empty. Add something from a title's page."
            : "Nothing matches."}
        </p>
      )}
      <ul className={ui.list}>
        {shown.map((group) => (
          <GroupRow
            key={group.key}
            group={group}
            open={open.includes(group.key)}
            onToggle={() => toggle(group.key)}
            busy={act.isPending}
            admin={admin}
            act={(paths) => act.mutate(paths)}
          />
        ))}
      </ul>
    </>
  );
}

function GroupRow({
  group,
  open,
  onToggle,
  busy,
  admin,
  act,
}: {
  group: Group;
  open: boolean;
  onToggle: () => void;
  busy: boolean;
  admin: boolean;
  act: (paths: string[]) => void;
}) {
  const { jobs } = group;
  const done = jobs.filter((job) => job.state === "done").length;
  const running = jobs.filter((job) => job.state === "running");
  const waiting = jobs.filter((job) => job.state === "queued").length;
  const failed = jobs.filter((job) => job.state === "failed");
  const active = jobs.filter(isActive);
  const retryable = jobs.filter((job) => job.state === "failed" || job.state === "cancelled");
  // Finished episodes count whole, running ones by how far they are.
  const progress = (done + running.reduce((sum, job) => sum + job.progress, 0)) / jobs.length;
  const speed = running.reduce((sum, job) => sum + (job.speed ?? 0), 0);

  const named = running.slice(0, NAMED).map(code).join(", ");
  const more = running.length > NAMED ? ` +${running.length - NAMED} more` : "";
  // A finished job holds the language it was downloaded in, a waiting one the picked one.
  const languages = [...new Set(jobs.map((job) => languageLabel(job.language)))];
  const status = [
    languages.join(", "),
    running.length > 0 &&
      `Downloading ${named}${more}${speed > 0 ? ` · ${formatBytes(speed)}/s` : ""}`,
    waiting > 0 && `${waiting} waiting`,
    failed.length > 0 && `${failed.length} failed`,
  ].filter(Boolean);

  return (
    <li className={styles.group}>
      <div className={ui.row}>
        <button
          type="button"
          className={styles.toggle}
          aria-expanded={open}
          aria-label={`${open ? "Hide" : "Show"} the episodes of ${group.title}`}
          onClick={onToggle}
        >
          {open ? "▾" : "▸"}
        </button>
        <div className={ui.grow}>
          <button type="button" className={styles.title} onClick={onToggle}>
            {group.title}
          </button>
          <p className={styles.summary} title={status.join(" · ")}>
            <span className={done === jobs.length ? ui.ok : undefined}>
              {done} of {jobs.length} {jobs.length === 1 ? "episode" : "episodes"} downloaded
            </span>
            {status.length > 0 && <span className={ui.muted}> · {status.join(" · ")}</span>}
          </p>
        </div>
        {admin && active.length > 0 && (
          <button
            type="button"
            className={ui.button}
            disabled={busy}
            aria-label={`Cancel the remaining downloads of ${group.title}`}
            onClick={() => act(active.map((job) => `${job.id}/cancel`))}
          >
            Cancel
          </button>
        )}
        {admin && active.length === 0 && retryable.length > 0 && (
          <button
            type="button"
            className={ui.button}
            disabled={busy}
            aria-label={`Retry the failed and cancelled downloads of ${group.title}`}
            onClick={() => act(retryable.map((job) => `${job.id}/retry`))}
          >
            Retry
          </button>
        )}
        {group.state === "active" && (
          <span
            className={ui.progress}
            style={{ width: `${progress * 100}%` }}
            role="progressbar"
            aria-label={group.title}
            aria-valuenow={Math.round(progress * 100)}
            aria-valuemin={0}
            aria-valuemax={100}
          />
        )}
      </div>

      {open && (
        <ul className={styles.episodes}>
          {jobs.map((job) => (
            <li key={job.id} className={ui.row}>
              <span className={`${ui.mono} ${ui.muted}`}>{code(job)}</span>
              <span className={ui.muted}>{languageLabel(job.language)}</span>
              <div className={ui.grow}>
                {job.state === "failed" && job.error && <p className={ui.error}>{job.error}</p>}
              </div>
              {job.state === "running" && (
                <span className={`${ui.mono} ${ui.muted}`}>
                  {job.speed !== undefined && `${formatBytes(job.speed)}/s · `}
                  {job.bytes !== undefined && `${formatBytes(job.bytes)} · `}
                  {Math.round(job.progress * 100)}%
                </span>
              )}
              <span className={stateStyles[job.state] ?? ui.muted}>{stateNames[job.state]}</span>
              {admin && isActive(job) && (
                <button
                  type="button"
                  className={ui.ghost}
                  disabled={busy}
                  aria-label={`Cancel ${group.title} ${code(job)}`}
                  onClick={() => act([`${job.id}/cancel`])}
                >
                  Cancel
                </button>
              )}
              {admin && (job.state === "failed" || job.state === "cancelled") && (
                <button
                  type="button"
                  className={ui.ghost}
                  disabled={busy}
                  aria-label={`Retry ${group.title} ${code(job)}`}
                  onClick={() => act([`${job.id}/retry`])}
                >
                  Retry
                </button>
              )}
            </li>
          ))}
        </ul>
      )}
    </li>
  );
}
