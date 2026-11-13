import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { api, type Entry, formatBytes, type LibraryRoot } from "../api";
import { Check } from "../Check";
import ui from "../ui.module.css";
import styles from "./Library.module.css";

/** The libraries as a folder tree that opens in place. */
export function LibraryPage() {
  const roots = useQuery({ queryKey: ["library"], queryFn: () => api<LibraryRoot[]>("/library") });

  return (
    <>
      <h1>Library</h1>
      {roots.isError && (
        <p className={ui.error} role="alert">
          {roots.error.message}
        </p>
      )}
      {roots.isSuccess && roots.data.length === 0 && (
        <p className={ui.notice}>No library is set up yet.</p>
      )}
      <ul className={styles.tree} role="tree" aria-label="Libraries">
        {roots.data?.map((root, index) => (
          <Folder
            key={root.id}
            root={root.id}
            path=""
            name={root.name}
            detail={index === 0 ? `${root.path} · main` : root.path}
            depth={0}
          />
        ))}
      </ul>
    </>
  );
}

function Folder({
  root,
  path,
  name,
  detail,
  depth,
}: {
  root: number;
  path: string;
  name: string;
  detail?: string;
  depth: number;
}) {
  const [open, setOpen] = useState(false);
  // Loaded only when opened, and again each time it is opened.
  const entries = useQuery({
    queryKey: ["library", root, path],
    queryFn: () => api<Entry[]>(`/library/${root}?path=${encodeURIComponent(path)}`),
    enabled: open,
    staleTime: 0,
  });
  const indent = { paddingLeft: `${depth * 20 + 4}px` };

  return (
    <li role="treeitem" aria-expanded={open} aria-selected={false}>
      <button
        type="button"
        className={`${styles.entry} ${depth === 0 ? styles.root : ""}`}
        style={indent}
        onClick={() => setOpen(!open)}
      >
        <span className={styles.chevron} aria-hidden="true">
          {open ? "▾" : "▸"}
        </span>
        <span className={styles.name}>{name}</span>
        {detail && <span className={`${ui.mono} ${ui.muted} ${styles.detail}`}>{detail}</span>}
        {isTitle(path) && <Check label="Downloaded series" />}
      </button>
      {open && (
        <ul role="group" className={styles.group}>
          {entries.isPending && (
            <li className={`${styles.entry} ${ui.muted}`} style={{ paddingLeft: `${(depth + 1) * 20 + 26}px` }}>
              Loading…
            </li>
          )}
          {entries.isError && (
            <li className={`${styles.entry} ${ui.bad}`} style={{ paddingLeft: `${(depth + 1) * 20 + 26}px` }}>
              {entries.error.message}
            </li>
          )}
          {entries.isSuccess && entries.data.length === 0 && (
            <li className={`${styles.entry} ${ui.muted}`} style={{ paddingLeft: `${(depth + 1) * 20 + 26}px` }}>
              Empty
            </li>
          )}
          {entries.data?.map((entry) => {
            const below = path ? `${path}/${entry.name}` : entry.name;
            return entry.folder ? (
              <Folder key={entry.name} root={root} path={below} name={entry.name} depth={depth + 1} />
            ) : (
              <li
                key={entry.name}
                role="treeitem"
                aria-selected={false}
                className={styles.entry}
                style={{ paddingLeft: `${(depth + 1) * 20 + 26}px` }}
              >
                <span className={`${ui.grow} ${ui.muted}`}>{entry.name}</span>
                <span className={`${ui.mono} ${ui.muted}`}>{formatBytes(entry.size)}</span>
              </li>
            );
          })}
        </ul>
      )}
    </li>
  );
}

/** A series folder: directly in a library, or in the `Series` folder older versions used. */
function isTitle(path: string) {
  const parts = path.split("/");
  return (parts.length === 1 && parts[0] !== "" && parts[0] !== "Series") ||
    (parts.length === 2 && parts[0] === "Series");
}
