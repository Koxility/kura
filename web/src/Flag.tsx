import { type Language, languageNames } from "./api";
import styles from "./Flag.module.css";

/** Drawn the way AniWorld marks languages: a flag for the audio, and for subtitles the
 *  Japanese flag with the subtitle language's flag in its corner. */
export function Flag({ language }: { language: Language | null }) {
  if (!language) return <span className={styles.flag} aria-hidden="true" />;
  const name = languageNames[language];
  return (
    <svg className={styles.flag} viewBox="0 0 24 16" role="img" aria-label={name}>
      <title>{name}</title>
      {language === "german_dub" && <German />}
      {language === "english" && <British />}
      {(language === "german_sub" || language === "english_sub") && (
        <>
          <Japanese />
          <svg x="11" y="7" width="13" height="9" viewBox="0 0 24 16" preserveAspectRatio="none">
            {language === "german_sub" ? <German /> : <British />}
            <rect width="24" height="16" fill="none" stroke="#0a0a0a" strokeWidth="2" />
          </svg>
        </>
      )}
    </svg>
  );
}

function German() {
  return (
    <>
      <rect width="24" height="5.34" fill="#000" />
      <rect y="5.33" width="24" height="5.34" fill="#dd0000" />
      <rect y="10.66" width="24" height="5.34" fill="#ffce00" />
    </>
  );
}

function Japanese() {
  return (
    <>
      <rect width="24" height="16" fill="#f5f5f5" />
      <circle cx="10" cy="7" r="4.2" fill="#bc002d" />
    </>
  );
}

function British() {
  return (
    <>
      <rect width="24" height="16" fill="#012169" />
      <path d="M0 0 24 16M24 0 0 16" stroke="#fff" strokeWidth="3.2" />
      <path d="M0 0 24 16M24 0 0 16" stroke="#c8102e" strokeWidth="1.2" />
      <path d="M12 0v16M0 8h24" stroke="#fff" strokeWidth="5" />
      <path d="M12 0v16M0 8h24" stroke="#c8102e" strokeWidth="2.6" />
    </>
  );
}
