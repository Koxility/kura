import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import { Link, Navigate, NavLink, Route, Routes } from "react-router";
import {
  api,
  type Job,
  type LiveEvent,
  requestsQuery,
  type Status,
  statusQuery,
  type User,
} from "./api";
import styles from "./App.module.css";
import { Auth } from "./pages/Auth";
import { AutoSyncPage } from "./pages/AutoSync";
import { Home } from "./pages/Home";
import { LibraryPage } from "./pages/Library";
import { LibrarySetup } from "./pages/LibrarySetup";
import { Queue } from "./pages/Queue";
import { Requests } from "./pages/Requests";
import { SeriesPage } from "./pages/SeriesPage";
import { SettingsPage } from "./pages/Settings";
import ui from "./ui.module.css";

const pages = [
  { path: "/", title: "Home" },
  { path: "/library", title: "Library", admin: true },
  { path: "/auto-sync", title: "Auto-Sync", admin: true },
  { path: "/queue", title: "Queue" },
  { path: "/requests", title: "Request" },
  { path: "/settings", title: "Settings", admin: true },
];

export function App() {
  const status = useQuery(statusQuery);
  if (status.isPending) return null;
  if (status.isError) {
    return (
      <p className={styles.centered} role="alert">
        Kura's server is not answering.
      </p>
    );
  }
  if (!status.data.user) return <Auth setup={status.data.needs_setup} />;
  return <Shell status={status.data} />;
}

function Shell({ status }: { status: Status }) {
  useLiveQueue();
  const client = useQueryClient();
  const logout = useMutation({
    mutationFn: () => api("/logout", "POST"),
    // Reset, not just clear: the status is fetched again and brings up the login.
    onSuccess: () => client.resetQueries(),
  });
  const user = status.user!;
  const admin = user.role === "admin";
  const requests = useQuery({ ...requestsQuery, enabled: admin });
  const open = requests.data?.filter((request) => request.state === "open").length ?? 0;

  return (
    <>
      <header className={styles.bar}>
        <div className={styles.brand}>
          <Link to="/" className={styles.wordmark}>
            Kura
          </Link>
          <a className={styles.version} href={status.repository} target="_blank" rel="noreferrer">
            v{status.version}
          </a>
          {status.update_available && (
            <a
              className={ui.badge}
              href={`${status.repository}/releases/latest`}
              target="_blank"
              rel="noreferrer"
              title="A newer release is available"
            >
              New
            </a>
          )}
        </div>
        <nav className={styles.nav} aria-label="Main">
          {pages
            .filter((page) => admin || !page.admin)
            .map((page) => (
              <NavLink key={page.path} to={page.path} end={page.path === "/"} className={styles.link}>
                {page.title}
                {page.path === "/requests" && admin && open > 0 && (
                  <span className={styles.count} aria-label={`${open} open`}>
                    {open}
                  </span>
                )}
              </NavLink>
            ))}
          <a
            className={styles.icon}
            href={status.repository}
            target="_blank"
            rel="noreferrer"
            aria-label="Kura on GitHub"
          >
            <svg width="18" height="18" viewBox="0 0 16 16" fill="currentColor" aria-hidden="true">
              <path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27s1.36.09 2 .27c1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.01 8.01 0 0 0 16 8c0-4.42-3.58-8-8-8Z" />
            </svg>
          </a>
          <span className={styles.user}>{user.name}</span>
          <button type="button" className={ui.ghost} onClick={() => logout.mutate()}>
            Logout
          </button>
        </nav>
      </header>
      <main className={styles.main}>
        {status.needs_library && admin ? (
          <LibrarySetup suggestedPath={status.suggested_library_path} />
        ) : (
          <Pages needsLibrary={status.needs_library} user={user} />
        )}
      </main>
    </>
  );
}

function Pages({ needsLibrary, user }: { needsLibrary: boolean; user: User }) {
  return (
    <>
      {needsLibrary && (
        <p className={ui.notice} role="status">
          An admin needs to set up a library before anything can be downloaded.
        </p>
      )}
      <Routes>
          <Route path="/" element={<Home />} />
          <Route path="/series/:site/:slug" element={<SeriesPage />} />
          {user.role === "admin" && (
            <>
              <Route path="/library" element={<LibraryPage />} />
              <Route path="/auto-sync" element={<AutoSyncPage />} />
              <Route path="/settings/:section?" element={<SettingsPage user={user} />} />
            </>
          )}
          <Route path="/queue" element={<Queue admin={user.role === "admin"} />} />
          <Route path="/requests" element={<Requests admin={user.role === "admin"} />} />
          <Route path="*" element={<Navigate to="/" replace />} />
      </Routes>
    </>
  );
}

/** Keeps the cached queue current from the server's event stream. */
function useLiveQueue() {
  const client = useQueryClient();
  useEffect(() => {
    const source = new EventSource("/api/v1/events");
    const seen = new Map<number, { bytes: number; at: number }>();
    source.onmessage = (message) => {
      const event = JSON.parse(message.data) as LiveEvent;
      if (event.type === "queue_changed") {
        client.invalidateQueries({ queryKey: ["queue"] });
        return;
      }
      if (event.type === "requests_changed") {
        client.invalidateQueries({ queryKey: ["requests"] });
        return;
      }
      const now = performance.now();
      const last = seen.get(event.id);
      seen.set(event.id, { bytes: event.bytes, at: now });
      const speed =
        last && now > last.at ? ((event.bytes - last.bytes) * 1000) / (now - last.at) : undefined;
      client.setQueryData<Job[]>(["queue"], (jobs) =>
        jobs?.map((job) =>
          job.id === event.id
            ? { ...job, progress: event.fraction, bytes: event.bytes, speed: speed ?? job.speed }
            : job,
        ),
      );
    };
    return () => source.close();
  }, [client]);
}
