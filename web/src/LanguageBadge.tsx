import { type Language, languageNames } from "./api";
import styles from "./LanguageBadge.module.css";

/** Short codes: audio language, plus "Sub" for subtitles over Japanese audio. */
const codes: Record<Language, string> = {
  german_dub: "DE",
  german_sub: "DE",
  english_sub: "EN",
  english: "EN",
};

/** A small mark for the language of an episode: a speaker for dubs, a caption box for subs. */
export function LanguageBadge({ language }: { language: Language | null }) {
  if (!language) return <span className={styles.badge} aria-hidden="true" />;
  const sub = language.endsWith("_sub");
  return (
    <span className={styles.badge} title={languageNames[language]}>
      <svg width="12" height="12" viewBox="0 0 16 16" fill="none" aria-hidden="true">
        {sub ? (
          <>
            <rect x="1.5" y="3" width="13" height="10" rx="2" stroke="currentColor" strokeWidth="1.4" />
            <path d="M4.5 9.5h3M9 9.5h2.5" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
          </>
        ) : (
          <>
            <path d="M2.5 6h2.5l3.5-3v10L5 10H2.5z" stroke="currentColor" strokeWidth="1.4" strokeLinejoin="round" />
            <path d="M11 5.5a3.5 3.5 0 0 1 0 5" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
          </>
        )}
      </svg>
      <span className={styles.code}>{codes[language]}</span>
      <span className={styles.label}>{languageNames[language]}</span>
    </span>
  );
}
