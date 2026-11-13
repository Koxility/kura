import { useInfiniteQuery, useMutation, useQuery } from "@tanstack/react-query";
import { type FormEvent, useEffect, useRef } from "react";
import { useNavigate, useSearchParams } from "react-router";
import {
  api,
  type Genre,
  type SeriesSummary,
  type Site,
  siteNames,
  siteNouns,
  statusQuery,
} from "../api";
import ui from "../ui.module.css";
import styles from "./Home.module.css";
import { SeriesGrid } from "./SeriesGrid";

/** Two rows on a wide screen. */
const SHOWN = 12;

export function Home() {
  const [params, setParams] = useSearchParams();
  const navigate = useNavigate();
  const status = useQuery(statusQuery);
  const enabled = status.data?.sites ?? [];
  const site = enabled.find((site) => site === params.get("site")) ?? enabled[0];
  const term = params.get("q") ?? "";
  const genre = params.get("genre") ?? "";

  const list = (key: string, path: string, on = true) => ({
    queryKey: [key, site, path],
    queryFn: () => api<SeriesSummary[]>(path),
    enabled: site !== undefined && on,
    staleTime: 10 * 60_000,
  });
  const results = useQuery(
    list("search", `/search?site=${site}&q=${encodeURIComponent(term)}`, term !== ""),
  );
  const inGenre = useInfiniteQuery({
    queryKey: ["genre", site, genre],
    queryFn: ({ pageParam }) =>
      api<SeriesSummary[]>(`/genres/${site}/${encodeURIComponent(genre)}?page=${pageParam}`),
    initialPageParam: 1,
    // An empty page means the genre has no more.
    getNextPageParam: (last, pages) => (last.length > 0 ? pages.length + 1 : undefined),
    enabled: site !== undefined && genre !== "",
    staleTime: 10 * 60_000,
  });
  const genreTitles = inGenre.data?.pages.flat();
  const latest = useQuery(list("new", `/new/${site}`));
  const popular = useQuery(list("popular", `/popular/${site}`));
  const genres = useQuery({
    queryKey: ["genres", site],
    queryFn: () => api<Genre[]>(`/genres/${site}`),
    enabled: site !== undefined,
    staleTime: 60 * 60_000,
  });
  const random = useMutation({
    mutationFn: () => api<SeriesSummary>(`/random/${site}`),
    onSuccess: (series) => navigate(`/series/${series.site}/${series.slug}`),
  });

  /** Keeps the site in the address and sets or clears the rest. */
  function show(next: { site?: Site; q?: string; genre?: string }) {
    const merged = { site: next.site ?? site ?? "", q: next.q ?? "", genre: next.genre ?? "" };
    setParams(Object.fromEntries(Object.entries(merged).filter(([, value]) => value !== "")));
  }

  function onSearch(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    show({ q: String(new FormData(event.currentTarget).get("q")).trim() });
  }

  if (status.isSuccess && site === undefined) {
    return (
      <>
        <h1 className={styles.title}>Kura</h1>
        <p className={ui.notice}>Every site is switched off. An admin can enable one in the settings.</p>
      </>
    );
  }
  if (site === undefined) return null;
  const noun = siteNouns[site];

  return (
    <>
      <h1 className={styles.title}>Kura</h1>
      <div className={ui.segmented} role="group" aria-label="Site">
        {enabled.map((option) => (
          <button
            key={option}
            type="button"
            className={ui.segment}
            aria-pressed={option === site}
            onClick={() => show({ site: option })}
          >
            {siteNames[option]}
          </button>
        ))}
      </div>

      <form role="search" className={styles.search} onSubmit={onSearch}>
        <input
          // Remounts when the term changes elsewhere, e.g. with the back button.
          key={`${site}/${term}`}
          className={ui.input}
          name="q"
          type="search"
          defaultValue={term}
          placeholder={`Search ${siteNames[site]}`}
          aria-label={`Search ${siteNames[site]}`}
        />
        <button className={ui.primary}>Search</button>
        <button
          type="button"
          className={ui.button}
          disabled={random.isPending}
          onClick={() => random.mutate()}
        >
          Random
        </button>
      </form>
      {random.isError && (
        <p className={ui.error} role="alert">
          {random.error.message}
        </p>
      )}

      {term ? (
        <SeriesGrid query={results} empty={`Nothing found for “${term}”.`} />
      ) : (
        <>
          <section>
            <h2 className={ui.section}>Discover by Genre</h2>
            {genres.isError && (
              <p className={ui.error} role="alert">
                {genres.error.message}
              </p>
            )}
            <ul className={ui.chips}>
              {genres.data?.map((option) => (
                <li key={option.slug}>
                  <button
                    type="button"
                    className={ui.chip}
                    aria-pressed={option.slug === genre}
                    onClick={() => show({ genre: option.slug === genre ? "" : option.slug })}
                  >
                    {option.name}
                  </button>
                </li>
              ))}
            </ul>
          </section>
          {genre ? (
            <section className={styles.genre}>
              <SeriesGrid
                query={{ ...inGenre, data: genreTitles }}
                empty="Nothing in this genre."
              />
              <MoreWhenSeen
                hasMore={inGenre.hasNextPage}
                loading={inGenre.isFetchingNextPage}
                onSeen={() => inGenre.fetchNextPage()}
              />
            </section>
          ) : (
            <>
              <section>
                <h2 className={ui.section}>New {noun}</h2>
                <SeriesGrid query={latest} limit={SHOWN} empty="Nothing new right now." />
              </section>
              <section>
                <h2 className={ui.section}>Popular {noun}</h2>
                <SeriesGrid query={popular} limit={SHOWN} empty="Nothing listed right now." />
              </section>
            </>
          )}
        </>
      )}
    </>
  );
}

/** Loads the next page when the end of the list scrolls into view. */
function MoreWhenSeen({
  hasMore,
  loading,
  onSeen,
}: {
  hasMore: boolean;
  loading: boolean;
  onSeen: () => void;
}) {
  const marker = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const element = marker.current;
    if (!element || !hasMore || loading) return;
    // Starts loading a little before the end is reached.
    const watcher = new IntersectionObserver(
      (entries) => entries.some((entry) => entry.isIntersecting) && onSeen(),
      { rootMargin: "600px" },
    );
    watcher.observe(element);
    return () => watcher.disconnect();
  }, [hasMore, loading, onSeen]);

  return (
    <div ref={marker} className={ui.notice} aria-live="polite">
      {loading ? "Loading more…" : hasMore ? "" : "That is everything in this genre."}
    </div>
  );
}
