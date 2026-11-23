import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { type FormEvent, useEffect, useState } from "react";
import { Link, Navigate, NavLink, Route, Routes, useLocation } from "react-router";
import {
  type Account,
  accountQuery,
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
import { CalendarPage } from "./pages/Calendar";
import { Home } from "./pages/Home";
import { LibraryPage } from "./pages/Library";
import { LibrarySetup } from "./pages/LibrarySetup";
import { Queue } from "./pages/Queue";
import { Requests } from "./pages/Requests";
import { ResetPassword } from "./pages/Reset";
import { SeriesPage } from "./pages/SeriesPage";
import { SettingsPage } from "./pages/Settings";
import { ThemeEffects } from "./ThemeEffects";
import ui from "./ui.module.css";

const pages = [
  { path: "/", title: "Home" },
  { path: "/library", title: "Library", admin: true },
  { path: "/calendar", title: "Calendar" },
  { path: "/auto-sync", title: "Auto-Sync", admin: true },
  { path: "/queue", title: "Queue" },
  { path: "/requests", title: "Request" },
  { path: "/settings", title: "Settings" },
];

export function App() {
  const status = useQuery(statusQuery);
  const { pathname } = useLocation();
  // A reset link opens this page whether or not someone is logged in on this device.
  if (pathname === "/reset-password") return <ResetPassword />;
  if (status.isPending) return null;
  if (status.isError) {
    return (
      <p className={styles.centered} role="alert">
        Kura's server is not answering.
      </p>
    );
  }
  if (!status.data.user) {
    return <Auth setup={status.data.needs_setup} passwordReset={status.data.password_reset} />;
  }
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
          {status.latest_version && (
            <a
              className={styles.update}
              href={`${status.repository}/releases/tag/v${status.latest_version}`}
              target="_blank"
              rel="noreferrer"
              title={`Kura ${status.latest_version} is available`}
            >
              Update
            </a>
          )}
        </div>
        <nav className={styles.nav} aria-label="Main">
          <div className={styles.links}>
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
          </div>
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
      <ThemeEffects />
      <main className={styles.main}>
        <EmailPrompt mailOn={status.password_reset} />
        {status.needs_library && admin ? (
          <LibrarySetup suggestedPath={status.suggested_library_path} />
        ) : (
          <Pages needsLibrary={status.needs_library} user={user} />
        )}
      </main>
      <Footer status={status} />
    </>
  );
}

/** Asks once, after login, for an email address to send a reset link to. Optional: "Not now"
 *  ends it for good, on every device. Right after logging in no password is needed; later the
 *  server asks for it and the card shows the field. */
function EmailPrompt({ mailOn }: { mailOn: boolean }) {
  const client = useQueryClient();
  const account = useQuery(accountQuery);
  const { pathname } = useLocation();
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [needsPassword, setNeedsPassword] = useState(false);
  const done = (next?: Account) => {
    if (next) client.setQueryData(accountQuery.queryKey, next);
    else client.invalidateQueries({ queryKey: accountQuery.queryKey });
  };
  const save = useMutation({
    mutationFn: () => api<Account>("/account/email", "PUT", { email, password }),
    onSuccess: done,
    onError: (error) => {
      if (error.message.includes("password")) setNeedsPassword(true);
    },
  });
  const skip = useMutation({
    mutationFn: () => api("/account/email/skip", "POST"),
    onSuccess: () => done(),
  });
  // The account page has its own email field.
  if (!account.data?.ask_email || pathname === "/settings/account") return null;

  function onSubmit(event: FormEvent) {
    event.preventDefault();
    save.mutate();
  }

  return (
    <form className={styles.prompt} onSubmit={onSubmit} aria-labelledby="email-prompt">
      <div className={styles.promptText}>
        <h2 id="email-prompt">Add an email address?</h2>
        <p>
          {mailOn
            ? "If you forget your password, a link to choose a new one is sent there."
            : "If you forget your password, a link to choose a new one can be sent there once an admin sets up mail."}{" "}
          Optional; you can also add it later under Settings → Account.
        </p>
      </div>
      <div className={styles.promptFields}>
        <input
          className={ui.input}
          type="email"
          value={email}
          onChange={(event) => setEmail(event.target.value)}
          placeholder="you@example.org"
          autoComplete="email"
          aria-label="Email"
          required
        />
        {needsPassword && (
          <input
            className={ui.input}
            type="password"
            value={password}
            onChange={(event) => setPassword(event.target.value)}
            placeholder="Current password"
            autoComplete="current-password"
            aria-label="Current password"
            required
          />
        )}
        <button className={ui.primary} disabled={save.isPending}>
          Save
        </button>
        <button
          type="button"
          className={ui.ghost}
          disabled={skip.isPending}
          onClick={() => skip.mutate()}
        >
          Not now
        </button>
      </div>
      {(save.isError || skip.isError) && (
        <p className={ui.error} role="alert">
          {(save.error ?? skip.error)?.message}
        </p>
      )}
    </form>
  );
}

/** Copyright, version and the commit the build came from; the version only once logged in. */
function Footer({ status }: { status: Status }) {
  const { version, commit } = status;
  return (
    <footer className={styles.footer}>
      <span>© {new Date().getFullYear()} Koxility</span>
      {version && (
        <>
          <span aria-hidden="true">·</span>
          <a href={`${status.repository}/releases/tag/v${version}`} target="_blank" rel="noreferrer">
            Kura v{version}
          </a>
        </>
      )}
      {commit && (
        <>
          <span aria-hidden="true">·</span>
          {commit !== "unknown" ? (
            <a
              className={styles.commit}
              href={`${status.repository}/commit/${commit}`}
              target="_blank"
              rel="noreferrer"
              title="The commit this build came from"
            >
              {commit}
            </a>
          ) : (
            <span className={styles.commit}>{commit}</span>
          )}
        </>
      )}
    </footer>
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
            </>
          )}
          <Route path="/settings/:section?/:part?" element={<SettingsPage user={user} />} />
          <Route path="/calendar" element={<CalendarPage />} />
          <Route path="/queue" element={<Queue admin={user.role === "admin"} />} />
          <Route path="/requests" element={<Requests admin={user.role === "admin"} />} />
          <Route path="*" element={<Navigate to="/" replace />} />
      </Routes>
    </>
  );
}

/** The download speed is the average over this many milliseconds. */
const SPEED_WINDOW = 10_000;
/** No speed is shown before the reports span this long. */
const SPEED_MIN_SPAN = 2_000;

/** Keeps the cached queue current from the server's event stream. */
function useLiveQueue() {
  const client = useQueryClient();
  useEffect(() => {
    // Recent byte counts per download. ffmpeg writes a stream part by part, so two reports in
    // a row often show the same count; the speed is taken over several seconds instead.
    const seen = new Map<number, { bytes: number; at: number }[]>();
    let source: EventSource | undefined;
    let reconnect: number | undefined;
    const refresh = () => {
      client.invalidateQueries({ queryKey: ["queue"] });
      client.invalidateQueries({ queryKey: ["requests"] });
      // A finished download changes what shows as downloaded.
      client.invalidateQueries({ queryKey: ["downloaded"] });
    };
    const connect = () => {
      window.clearTimeout(reconnect);
      source?.close();
      source = new EventSource("/api/v1/events");
      // Events sent while the stream was down are lost, e.g. while a phone slept or the tab
      // was in the background, so every new connection reads everything live again.
      source.onopen = () => {
        seen.clear();
        refresh();
      };
      // The browser reconnects by itself after a dropped connection, but gives up for good
      // when the server answers with an error, e.g. a proxy while Kura restarts.
      source.onerror = () => {
        if (source?.readyState !== EventSource.CLOSED) return;
        window.clearTimeout(reconnect);
        reconnect = window.setTimeout(connect, 5000);
      };
      source.onmessage = onMessage;
    };
    const disconnect = () => {
      window.clearTimeout(reconnect);
      source?.close();
      source = undefined;
    };
    // A browser keeps only six connections to a server over plain HTTP. A stream held open by
    // every background tab leaves none for loading pages, so only a visible tab listens.
    const onVisibility = () => {
      if (document.visibilityState === "visible") {
        if (!source) connect();
      } else {
        disconnect();
      }
    };
    const onMessage = (message: MessageEvent<string>) => {
      const event = JSON.parse(message.data) as LiveEvent;
      if (event.type === "queue_changed") {
        client.invalidateQueries({ queryKey: ["queue"] });
        client.invalidateQueries({ queryKey: ["downloaded"] });
        return;
      }
      if (event.type === "requests_changed") {
        client.invalidateQueries({ queryKey: ["requests"] });
        return;
      }
      const now = performance.now();
      let samples = seen.get(event.id) ?? [];
      // Fewer bytes than before: a new try started from nothing.
      const restarted = samples.length > 0 && event.bytes < samples[samples.length - 1].bytes;
      if (restarted) samples = [];
      samples.push({ bytes: event.bytes, at: now });
      samples = samples.filter((sample) => now - sample.at <= SPEED_WINDOW);
      seen.set(event.id, samples);
      const oldest = samples[0];
      const speed =
        now - oldest.at >= SPEED_MIN_SPAN
          ? ((event.bytes - oldest.bytes) * 1000) / (now - oldest.at)
          : undefined;
      const jobs = client.getQueryData<Job[]>(["queue"]);
      const job = jobs?.find((job) => job.id === event.id);
      // Progress for a download the page does not show as running means a change was missed.
      if (jobs && job?.state !== "running") {
        client.invalidateQueries({ queryKey: ["queue"] });
        return;
      }
      client.setQueryData<Job[]>(["queue"], (jobs) =>
        jobs?.map((job) =>
          job.id === event.id
            ? {
                ...job,
                progress: event.fraction,
                bytes: event.bytes,
                speed: restarted ? undefined : (speed ?? job.speed),
              }
            : job,
        ),
      );
    };
    if (document.visibilityState === "visible") connect();
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      document.removeEventListener("visibilitychange", onVisibility);
      disconnect();
    };
  }, [client]);
}
