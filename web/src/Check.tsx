import styles from "./Check.module.css";

/** A green checkmark for something that is already downloaded. */
export function Check({ label = "Downloaded", badge = false }: { label?: string; badge?: boolean }) {
  return (
    <span className={badge ? styles.badge : styles.check} role="img" aria-label={label} title={label}>
      <svg width="12" height="12" viewBox="0 0 12 12" fill="none" aria-hidden="true">
        <path
          d="M2.5 6.3 5 8.7l4.5-5.2"
          stroke="currentColor"
          strokeWidth="1.8"
          strokeLinecap="round"
          strokeLinejoin="round"
        />
      </svg>
    </span>
  );
}
