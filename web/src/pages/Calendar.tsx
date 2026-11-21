import { useQuery } from "@tanstack/react-query";
import { Link } from "react-router";
import { api, type CalendarDay, type CalendarEntry, type Language, statusQuery } from "../api";
import { Flag } from "../Flag";
import ui from "../ui.module.css";
import styles from "./Calendar.module.css";

const weekdays = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];

/** One card: an episode with every language it comes out in at that time. */
interface Card extends Omit<CalendarEntry, "language"> {
  languages: Language[];
}

/** AniWorld's calendar: today first, then the following days, each with its titles. */
export function CalendarPage() {
  const status = useQuery(statusQuery);
  // Only AniWorld publishes a calendar.
  const enabled = status.data?.sites.includes("aniworld") ?? false;
  const calendar = useQuery({
    queryKey: ["calendar", "aniworld"],
    queryFn: () => api<CalendarDay[]>("/calendar/aniworld"),
    enabled,
    staleTime: 5 * 60_000,
  });
  const days = calendar.data ?? [];

  /** The sections run from today for a week, so each weekday's is its next one. */
  function jump(weekday: number) {
    document
      .getElementById(`day-${weekday}`)
      ?.scrollIntoView({ behavior: "smooth", block: "start" });
  }

  return (
    <>
      <h1>Calendar</h1>
      <p className={ui.lead}>
        When new episodes come out on AniWorld. Times are approximate and in German time.
      </p>
      {status.isSuccess && !enabled && (
        <p className={ui.notice}>AniWorld is switched off, and only AniWorld has a calendar.</p>
      )}
      {calendar.isError && (
        <p className={ui.error} role="alert">
          {calendar.error.message}
        </p>
      )}
      {calendar.isPending && enabled && <p className={ui.notice}>Loading the calendar…</p>}

      {days.length > 0 && (
        <>
          <nav className={styles.week} aria-label="Weekday">
            {weekdays.map((name, index) => {
              const day = days.find((day) => day.weekday === index + 1);
              return (
                <button
                  key={name}
                  type="button"
                  className={styles.weekday}
                  aria-current={day?.today ? "date" : undefined}
                  disabled={!day}
                  onClick={() => jump(index + 1)}
                >
                  <span className={styles.long}>{name}</span>
                  <span className={styles.short} aria-hidden="true">
                    {name.slice(0, 2)}
                  </span>
                </button>
              );
            })}
          </nav>

          {days.map((day) => (
            <section key={day.date} id={`day-${day.weekday}`} className={styles.day}>
              <h2 className={styles.dayHeading}>
                {weekdays[day.weekday - 1]}
                <span className={styles.date}>{day.date}</span>
                {day.today && <span className={styles.today}>Today</span>}
              </h2>
              {day.entries.length === 0 ? (
                <p className={ui.notice}>Nothing announced.</p>
              ) : (
                <ul className={ui.grid}>
                  {cards(day.entries).map((card) => (
                    <li key={`${card.slug}/${card.season}/${card.number}/${card.time}`}>
                      <CalendarCard card={card} />
                    </li>
                  ))}
                </ul>
              )}
            </section>
          ))}
        </>
      )}
    </>
  );
}

function CalendarCard({ card }: { card: Card }) {
  return (
    <Link to={`/series/aniworld/${card.slug}`} className={ui.card}>
      {card.cover_url ? (
        // The title below names the series, so the image is decoration.
        <img className={ui.cover} src={card.cover_url} alt="" loading="lazy" referrerPolicy="no-referrer" />
      ) : (
        <span className={ui.cover} aria-hidden="true">
          {card.title.charAt(0)}
        </span>
      )}
      <span className={`${ui.cardTitle} ${styles.title}`} title={card.title}>
        {card.title}
      </span>
      <span className={styles.meta}>
        {card.online && (
          <svg className={styles.out} width="12" height="12" viewBox="0 0 16 16" role="img" aria-label="Out now">
            <title>Out now</title>
            <path d="M3 8.5 6.5 12 13 4.5" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
          </svg>
        )}
        <span className={styles.code}>{code(card)}</span>
        <span className={styles.flags}>
          {card.languages.map((language) => (
            <Flag key={language} language={language} />
          ))}
        </span>
        <span className={styles.time}>{card.time}</span>
      </span>
    </Link>
  );
}

/** One card per episode and time; AniWorld lists each language on its own. */
function cards(entries: CalendarEntry[]): Card[] {
  const merged: Card[] = [];
  for (const { language, ...entry } of entries) {
    const same = merged.find(
      (card) =>
        card.slug === entry.slug &&
        card.season === entry.season &&
        card.number === entry.number &&
        card.time === entry.time,
    );
    if (same) {
      if (language && !same.languages.includes(language)) same.languages.push(language);
      same.online ||= entry.online;
    } else {
      merged.push({ ...entry, languages: language ? [language] : [] });
    }
  }
  return merged;
}

/** `S02E07`, or `Film 22`. */
function code({ season, number }: { season: number; number: number }) {
  const two = (value: number) => String(value).padStart(2, "0");
  return season === 0 ? `Film ${number}` : `S${two(season)}E${two(number)}`;
}
