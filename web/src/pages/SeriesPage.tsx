import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { type FormEvent, useRef, useState } from "react";
import { useLocation, useNavigate, useParams } from "react-router";
import {
  api,
  type Episode,
  episodeCode,
  hosterNames,
  imdbPattern,
  languageNames,
  languages,
  type Picked,
  seasonName,
  type Series,
  settingsQuery,
  siteNames,
  statusQuery,
  supportedHosters,
} from "../api";
import { Check } from "../Check";
import ui from "../ui.module.css";
import styles from "./SeriesPage.module.css";

/** A title with its seasons as a tree. Admins download, users request. */
export function SeriesPage() {
  const { site, slug } = useParams();
  const client = useQueryClient();
  const navigate = useNavigate();
  const location = useLocation();
  const status = useQuery(statusQuery);
  const admin = status.data?.user?.role === "admin";
  const base = `/series/${site}/${slug}`;
  const series = useQuery({ queryKey: ["series", site, slug], queryFn: () => api<Series>(base) });
  // Only admins may read the settings; users have no Auto-Sync to exclude from.
  const settings = useQuery({ ...settingsQuery, enabled: admin });
  // What is already in a library, by database and by folders on disk.
  const title = series.data?.summary.title;
  const imdb = series.data?.imdb_id;
  const downloaded = useQuery({
    queryKey: ["downloaded", site, slug],
    queryFn: () => {
      const query = new URLSearchParams({ title: title ?? "" });
      if (imdb) query.set("imdb", imdb);
      return api<[number, number][]>(`${base}/downloaded?${query}`);
    },
    enabled: title !== undefined,
  });
  const have = new Set((downloaded.data ?? []).map(([season, number]) => `${season}:${number}`));

  // An empty choice leaves it to the settings.
  const [language, setLanguage] = useState("");
  const [hoster, setHoster] = useState("");
  const [library, setLibrary] = useState("");
  const [wholeSeasons, setWholeSeasons] = useState<number[]>([]);
  // Single episodes as "season:episode".
  const [single, setSingle] = useState<string[]>([]);
  // Asked for when the site has no IMDb ID; holds what to send once answered.
  const [asking, setAsking] = useState<Picked | null>(null);

  const send = useMutation({
    mutationFn: ({ picked, imdb }: { picked: Picked; imdb?: string }) =>
      api<{ queued?: number; id?: number }>(admin ? "/queue" : "/requests", "POST", {
        site,
        slug,
        ...picked,
        language: language || undefined,
        hoster: hoster || undefined,
        library: (admin && library) || undefined,
        imdb_id: imdb,
      }),
    onSuccess: () => {
      setWholeSeasons([]);
      setSingle([]);
      if (!admin) client.invalidateQueries({ queryKey: ["requests"] });
    },
  });
  const exclude = useMutation({
    mutationFn: (excluded: boolean) =>
      api("/autosync/exclude", "POST", { site, slug, title: series.data?.summary.title, excluded }),
    onSettled: () => client.invalidateQueries({ queryKey: ["settings"] }),
  });

  if (series.isPending) return <p className={ui.notice}>Loading…</p>;
  if (series.isError) {
    return (
      <p className={ui.error} role="alert">
        {series.error.message}
      </p>
    );
  }
  const { summary, description, seasons, imdb_id } = series.data;
  const libraries = status.data?.libraries ?? [];
  const excluded =
    settings.data?.auto_sync.excluded.some((title) => title.site === site && title.slug === slug) ??
    false;
  const verb = admin ? "Download" : "Request";
  const nothing = wholeSeasons.length === 0 && single.length === 0;
  const count = [
    wholeSeasons.length > 0 &&
      `${wholeSeasons.length} ${wholeSeasons.length === 1 ? "season" : "seasons"}`,
    single.length > 0 && `${single.length} ${single.length === 1 ? "episode" : "episodes"}`,
  ]
    .filter(Boolean)
    .join(", ");

  /** Sends at once when the site knows the IMDb ID, else asks for it first. */
  function start(picked: Picked) {
    if (imdb_id) send.mutate({ picked });
    else setAsking(picked);
  }
  function selected(): Picked {
    // Single episodes of a whole season are covered by the season.
    const pairs = single
      .map((key) => key.split(":").map(Number) as [number, number])
      .filter(([season]) => !wholeSeasons.includes(season));
    return { all: false, seasons: wholeSeasons, episodes: pairs };
  }
  function back() {
    // Opened directly, there is nowhere to go back to inside Kura.
    if (location.key !== "default") navigate(-1);
    else navigate("/");
  }

  return (
    <>
      <button type="button" className={`${ui.round} ${styles.back}`} aria-label="Back" onClick={back}>
        <svg width="16" height="16" viewBox="0 0 16 16" fill="none" aria-hidden="true">
          <path d="M10 3 5 8l5 5" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      </button>

      <header className={styles.header}>
        {summary.cover_url && (
          <img className={styles.cover} src={summary.cover_url} alt="" referrerPolicy="no-referrer" />
        )}
        <div>
          <p className={ui.muted}>
            {siteNames[summary.site]}
            {imdb_id && ` · ${imdb_id}`}
          </p>
          <h1>{summary.title}</h1>
          {description && <p className={`${ui.lead} ${styles.description}`}>{description}</p>}
        </div>
      </header>

      <ul className={styles.tree} aria-label="Seasons">
        {seasons.map((season) => (
          <SeasonRow
            key={season}
            site={site ?? ""}
            slug={slug ?? ""}
            season={season}
            have={have}
            whole={wholeSeasons.includes(season)}
            onWhole={(on) =>
              setWholeSeasons(
                on ? [...wholeSeasons, season] : wholeSeasons.filter((other) => other !== season),
              )
            }
            single={single}
            onSingle={(key) =>
              setSingle(single.includes(key) ? single.filter((other) => other !== key) : [...single, key])
            }
          />
        ))}
      </ul>

      <section className={styles.options} aria-label={`${verb} options`}>
        <label className={ui.field}>
          Language
          <select className={ui.input} value={language} onChange={(event) => setLanguage(event.target.value)}>
            <option value="">Automatic</option>
            {languages.map((option) => (
              <option key={option} value={option}>
                {languageNames[option]}
              </option>
            ))}
          </select>
        </label>
        <label className={ui.field}>
          Provider
          <select className={ui.input} value={hoster} onChange={(event) => setHoster(event.target.value)}>
            <option value="">Automatic</option>
            {supportedHosters.map((option) => (
              <option key={option} value={option}>
                {hosterNames[option] ?? option}
              </option>
            ))}
          </select>
        </label>
        {admin && libraries.length > 1 && (
          <label className={ui.field}>
            Library
            <select className={ui.input} value={library} onChange={(event) => setLibrary(event.target.value)}>
              <option value="">Default</option>
              {libraries.map((name) => (
                <option key={name} value={name}>
                  {name}
                </option>
              ))}
            </select>
          </label>
        )}
        {admin && (
          <label className={`${ui.check} ${styles.exclude}`}>
            <input
              type="checkbox"
              checked={excluded}
              disabled={exclude.isPending || !settings.data}
              onChange={(event) => exclude.mutate(event.target.checked)}
            />
            Exclude this title from Auto-Sync
          </label>
        )}
      </section>

      <div className={ui.actions}>
        <span className={ui.muted} role="status">
          {send.isPending
            ? admin
              ? "Adding to the queue…"
              : "Sending the request…"
            : !nothing
              ? `${count} selected`
              : send.isSuccess &&
                (admin
                  ? send.data.queued === 0
                    ? "Already in the queue or downloaded."
                    : `Added ${send.data.queued} to the queue.`
                  : "Request sent. An admin will look at it.")}
        </span>
        <button
          type="button"
          className={ui.primary}
          disabled={nothing || send.isPending}
          onClick={() => start(selected())}
        >
          {verb} Selected
        </button>
        <button
          type="button"
          className={ui.button}
          disabled={send.isPending}
          onClick={() => start({ all: true, seasons: [], episodes: [] })}
        >
          {verb} All
        </button>
      </div>
      {(send.error ?? exclude.error) && (
        <p className={ui.error} role="alert">
          {(send.error ?? exclude.error)?.message}
        </p>
      )}

      {asking && (
        <ImdbPrompt
          title={summary.title}
          onAnswer={(imdb) => {
            send.mutate({ picked: asking, imdb });
            setAsking(null);
          }}
          onCancel={() => setAsking(null)}
        />
      )}
    </>
  );
}

function SeasonRow({
  site,
  slug,
  season,
  have,
  whole,
  onWhole,
  single,
  onSingle,
}: {
  site: string;
  slug: string;
  season: number;
  /** Downloaded episodes as "season:episode". */
  have: Set<string>;
  whole: boolean;
  onWhole: (on: boolean) => void;
  single: string[];
  onSingle: (key: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const episodes = useQuery({
    queryKey: ["episodes", site, slug, season],
    queryFn: () => api<Episode[]>(`/series/${site}/${slug}/seasons/${season}`),
    enabled: open,
  });
  const picked = single.filter((key) => key.startsWith(`${season}:`)).length;
  const got = [...have].filter((key) => key.startsWith(`${season}:`)).length;
  // Complete once the episode list is known and every episode is there.
  const complete = episodes.data !== undefined && episodes.data.length > 0 && got >= episodes.data.length;

  return (
    <li>
      <div className={styles.season}>
        <button type="button" className={styles.seasonToggle} aria-expanded={open} onClick={() => setOpen(!open)}>
          <span className={styles.chevron} aria-hidden="true">
            {open ? "▾" : "▸"}
          </span>
          <span className={ui.grow}>{seasonName(season)}</span>
          {got > 0 && !complete && <span className={ui.ok}>{got} downloaded</span>}
          {complete && <Check label={`${seasonName(season)} is downloaded`} />}
          {episodes.data && (
            <span className={ui.muted}>
              {episodes.data.length} {episodes.data.length === 1 ? "episode" : "episodes"}
            </span>
          )}
          {!whole && picked > 0 && <span className={ui.muted}>{picked} picked</span>}
        </button>
        <label className={`${ui.check} ${styles.wholeCheck}`}>
          <span className={styles.wholeLabel}>Whole {season === 0 ? "films" : "season"}</span>
          <input
            type="checkbox"
            checked={whole}
            aria-label={`${seasonName(season)}, all episodes`}
            onChange={(event) => onWhole(event.target.checked)}
          />
        </label>
      </div>

      {open && (
        <ul className={styles.episodes}>
          {episodes.isPending && <li className={`${styles.episode} ${ui.muted}`}>Loading…</li>}
          {episodes.isError && <li className={`${styles.episode} ${ui.bad}`}>{episodes.error.message}</li>}
          {episodes.data?.map((episode) => {
            const key = `${season}:${episode.number}`;
            return (
              <li key={episode.number} className={styles.episode}>
                <span className={styles.status}>
                  {have.has(key) && <Check label={`${episodeCode(season, episode.number)} is downloaded`} />}
                </span>
                <label className={`${ui.check} ${ui.grow}`}>
                  <input
                    type="checkbox"
                    checked={whole || single.includes(key)}
                    disabled={whole}
                    onChange={() => onSingle(key)}
                  />
                  <span className={`${ui.mono} ${ui.muted}`}>{episodeCode(season, episode.number)}</span>
                  {episode.title || `Episode ${episode.number}`}
                </label>
                <span className={ui.tags}>
                  {episode.languages.map((option) => (
                    <span key={option} className={ui.tag}>
                      {languageNames[option]}
                    </span>
                  ))}
                </span>
              </li>
            );
          })}
        </ul>
      )}
    </li>
  );
}

/** Asks for the IMDb ID the site does not have; it goes into the folder name. */
function ImdbPrompt({
  title,
  onAnswer,
  onCancel,
}: {
  title: string;
  onAnswer: (imdb: string) => void;
  onCancel: () => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [id, setId] = useState("");
  const valid = imdbPattern.test(id.trim());

  function onSubmit(event: FormEvent) {
    event.preventDefault();
    if (valid) onAnswer(id.trim());
  }

  return (
    <dialog
      ref={(element) => {
        dialog.current = element;
        if (element && !element.open) element.showModal();
      }}
      className={ui.dialog}
      onCancel={onCancel}
      aria-labelledby="imdb-title"
    >
      <form onSubmit={onSubmit}>
        <h2 id="imdb-title">IMDb ID for “{title}”</h2>
        <p>
          The site has no IMDb ID for this title. Kura puts it in the folder name, so Jellyfin
          finds the right series. Skip to name the folder [imdbid-none].
        </p>
        <label className={ui.field}>
          IMDb ID
          <input
            className={ui.input}
            value={id}
            onChange={(event) => setId(event.target.value)}
            placeholder="tt0388629"
            autoFocus
          />
        </label>
        <div className={ui.actions}>
          <button type="button" className={ui.ghost} onClick={onCancel}>
            Cancel
          </button>
          <button type="button" className={ui.button} onClick={() => onAnswer("none")}>
            Skip
          </button>
          <button className={ui.primary} disabled={!valid}>
            Use this ID
          </button>
        </div>
      </form>
    </dialog>
  );
}
