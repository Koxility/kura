import { Link } from "react-router";
import { type SeriesSummary, siteNames } from "../api";
import ui from "../ui.module.css";

/** A grid of covers for a list of series that may still be loading or have failed. */
export function SeriesGrid({
  query,
  empty,
  limit,
}: {
  /** A query's state; plain fields, so an endless list can pass its pages joined. */
  query: { isPending: boolean; error: Error | null; data?: SeriesSummary[] };
  empty: string;
  /** Show only the first few. */
  limit?: number;
}) {
  if (query.error) {
    return (
      <p className={ui.error} role="alert">
        {query.error.message}
      </p>
    );
  }
  if (query.isPending || !query.data) return <p className={ui.notice}>Loading…</p>;
  if (query.data.length === 0) return <p className={ui.notice}>{empty}</p>;

  return (
    <ul className={ui.grid}>
      {query.data.slice(0, limit).map((series) => (
        <li key={`${series.site}/${series.slug}`}>
          <Link to={`/series/${series.site}/${series.slug}`} className={ui.card}>
            {series.cover_url ? (
              // The title below names the series, so the image is decoration.
              <img
                className={ui.cover}
                src={series.cover_url}
                alt=""
                loading="lazy"
                referrerPolicy="no-referrer"
              />
            ) : (
              <span className={ui.cover} aria-hidden="true">
                {series.title.charAt(0)}
              </span>
            )}
            <span className={ui.cardTitle}>{series.title}</span>
            <span className={ui.cardSite}>{siteNames[series.site]}</span>
          </Link>
        </li>
      ))}
    </ul>
  );
}
