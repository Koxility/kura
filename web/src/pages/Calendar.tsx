import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { Link } from "react-router";
import { api, type CalendarDay, statusQuery } from "../api";
import { LanguageBadge } from "../LanguageBadge";
import ui from "../ui.module.css";
import styles from "./Calendar.module.css";

const dayNames: Record<number, string> = {
  [-2]: "Day before yesterday",
  [-1]: "Yesterday",
  0: "Today",
  1: "Tomorrow",
  2: "Day after tomorrow",
};

/** AniWorld's release calendar: a few days around today, one tab each. */
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
  const today = Math.max(
    0,
    days.findIndex((day) => day.offset === 0),
  );
  const [picked, setPicked] = useState<number | null>(null);
  const shown = days[picked ?? today];

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
          <div className={styles.days} role="tablist" aria-label="Day">
            {days.map((day, index) => (
              <button
                key={day.date}
                type="button"
                role="tab"
                aria-selected={day === shown}
                className={styles.day}
                onClick={() => setPicked(index)}
              >
                <span className={styles.date}>{day.date}</span>
                <span className={styles.dayName}>
                  {day.offset !== null ? (dayNames[day.offset] ?? day.date) : day.date}
                </span>
                <span className={styles.count}>{day.entries.length}</span>
              </button>
            ))}
          </div>

          {shown && (
            <ul className={ui.list} role="tabpanel">
              {shown.entries.length === 0 && <li className={ui.notice}>Nothing announced.</li>}
              {shown.entries.map((entry, index) => (
                // A title often appears once per language at the same time.
                // biome-ignore lint/suspicious/noArrayIndexKey: entries have no id of their own
                <li key={index}>
                  <Link to={`/series/aniworld/${entry.slug}`} className={styles.entry}>
                    <span className={styles.time}>{entry.time}</span>
                    <span className={styles.what}>
                      <span className={styles.title}>{entry.title}</span>
                      <span className={styles.episode}>
                        {entry.season === 0 ? "Films" : `Season ${entry.season}`} · Episode{" "}
                        {entry.number}
                      </span>
                    </span>
                    <LanguageBadge language={entry.language} />
                    <span
                      className={entry.online ? styles.online : styles.waiting}
                      title={entry.online ? "Out now" : "Not out yet"}
                    >
                      {entry.online ? "Out" : "Soon"}
                    </span>
                  </Link>
                </li>
              ))}
            </ul>
          )}
        </>
      )}
    </>
  );
}
