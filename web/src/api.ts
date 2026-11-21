export type Site = "aniworld" | "serienstream";
export type Language = "german_dub" | "german_sub" | "english_sub" | "english";
export type JobState = "queued" | "running" | "done" | "failed" | "cancelled";
export type Role = "admin" | "user";

export interface SeriesSummary {
  site: Site;
  slug: string;
  title: string;
  cover_url: string | null;
  /** At least one episode is in a library. */
  downloaded?: boolean;
}

export interface Series {
  summary: SeriesSummary;
  description: string;
  seasons: number[];
  /** Like `tt0388629`, when the site links to IMDb. */
  imdb_id: string | null;
}

export interface Episode {
  season: number;
  number: number;
  title: string;
  languages: Language[];
}

export interface Genre {
  slug: string;
  name: string;
}

export interface Job {
  id: number;
  episode: { site: Site; slug: string; season: number; number: number };
  series_title: string;
  state: JobState;
  progress: number;
  error: string | null;
  path: string | null;
  language: Language | null;
  hoster: string | null;
  dir: string;
  hidden: boolean;
  queued_at: number;
  imdb_id: string | null;
  /** Filled in from live events, not sent by the server. */
  bytes?: number;
  speed?: number;
}

export interface User {
  id: number;
  name: string;
  role: Role;
}

/** A user's look and sound of the web app. */
export interface Theme {
  /** Plays a song an admin uploaded, on a loop. */
  music: boolean;
  /** Id of the song; null plays the first one. */
  song: string | null;
  /** 0 to 100. */
  volume: number;
  /** A faint animated ASCII scene behind the pages. */
  ascii: boolean;
  /** One of `ascii_library`. */
  ascii_piece: string;
  /** ascii.rest piece names this account keeps to choose from. */
  ascii_library: string[];
}

/** "glass": frosted glass over the whole window; "halo": a dark glow around text. */
export type Backdrop = "glass" | "halo";

/** A song an admin uploaded for the background music. */
export interface Song {
  id: string;
  name: string;
}

/** The own account, as Settings → User shows it. */
export interface Account extends User {
  /** Stored, not used yet. */
  email: string | null;
  theme: Theme;
}

/** A key another program (e.g. Musubi) sends instead of logging in. */
export interface ApiKey {
  id: number;
  name: string;
  created_at: number;
  last_used_at: number | null;
}

/** An episode the site published recently. */
export interface NewEpisode {
  site: Site;
  slug: string;
  title: string;
  /** 0 for films and specials. */
  season: number;
  number: number;
  language: Language | null;
  /** As the site shows it, e.g. `21.11.2026 19:05`. */
  released: string;
}

export interface CalendarEntry {
  slug: string;
  title: string;
  cover_url: string | null;
  /** 0 for films. */
  season: number;
  number: number;
  /** `13:40`, approximate. */
  time: string;
  language: Language | null;
  /** Watchable already. */
  online: boolean;
}

export interface CalendarDay {
  /** 1 is Monday, 7 Sunday. */
  weekday: number;
  /** `21.11.2026`. */
  date: string;
  today: boolean;
  entries: CalendarEntry[];
}

export interface Library {
  name: string;
  path: string;
  default_for: Site[];
  /** The same folder as Jellyfin sees it; absent or null when it is the same path. */
  jellyfin_path?: string | null;
}

/** Off while `url` is empty. */
export interface Jellyfin {
  url: string;
  api_key: string;
}

export type Schedule = { kind: "every"; minutes: number } | { kind: "daily"; at: string };

export interface TitleRef {
  site: Site;
  slug: string;
  title: string;
}

export interface Settings {
  parallel: number;
  languages: Language[];
  hosters: string[];
  output_format: "mkv" | "mp4";
  /** `null` uses `naming_template_default`. */
  naming_template: string | null;
  /** Sent by the server: what applies while `naming_template` is null. Ignored on save. */
  naming_template_default?: string;
  /** The first is the main library. */
  libraries: Library[];
  disabled_sites: Site[];
  auto_sync: {
    enabled: boolean;
    only_new: boolean;
    schedule: Schedule;
    excluded: TitleRef[];
    /** Taken off the tracked list until a later download; `at` in Unix seconds. */
    removed: (TitleRef & { at: number })[];
  };
  jellyfin: Jellyfin;
  backdrop: Backdrop;
}

export interface Status {
  needs_setup: boolean;
  user: User | null;
  version: string;
  /** Short commit hash of the build, or `unknown`. */
  commit: string;
  /** How text stays readable over the ASCII background, the same for every account. */
  backdrop: Backdrop;
  repository: string;
  /** A newer release on GitHub, like `1.2.0`; absent while this one is the newest. */
  latest_version: string | null;
  needs_library: boolean;
  suggested_library_path: string | null;
  /** Sites that are switched on. */
  sites: Site[];
  /** Library names, for admins only. */
  libraries: string[];
  /** What a download uses when nothing is picked; absent while logged out. */
  defaults: Defaults | null;
}

export interface Defaults {
  /** The first language of the settings; later ones are tried when it has no link. */
  language: Language | null;
  /** The first provider of the settings that Kura supports. */
  hoster: string | null;
  /** Each enabled site's library, for admins only. */
  libraries: Partial<Record<Site, string>>;
}

export type RequestState = "open" | "accepted" | "rejected";

export interface Picked {
  all: boolean;
  seasons: number[];
  episodes: [number, number][];
}

export interface Request {
  id: number;
  user_id: number;
  user_name: string;
  site: Site;
  slug: string;
  series_title: string;
  picked: Picked;
  language: Language | null;
  hoster: string | null;
  imdb_id: string | null;
  state: RequestState;
  created_at: number;
}

export interface SyncStatus {
  enabled: boolean;
  last_run: number | null;
  next_run: number | null;
  titles: (TitleRef & { downloaded: number; checked_at: number | null; result: string | null })[];
}

export interface LibraryRoot {
  id: number;
  name: string;
  path: string;
}

export interface Entry {
  name: string;
  folder: boolean;
  size: number;
}

export type LiveEvent =
  | { type: "queue_changed" }
  | { type: "requests_changed" }
  | { type: "progress"; id: number; fraction: number; bytes: number };

export class ApiError extends Error {
  constructor(
    message: string,
    readonly status: number,
  ) {
    super(message);
  }
}

export async function api<T>(path: string, method = "GET", body?: unknown): Promise<T> {
  const response = await fetch(`/api/v1${path}`, {
    method,
    headers: body === undefined ? undefined : { "Content-Type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (response.status === 204) return undefined as T;
  const data = await response.json().catch(() => null);
  if (!response.ok) {
    throw new ApiError(data?.error ?? `The request failed (${response.status}).`, response.status);
  }
  return data as T;
}

export const jobsQuery = {
  queryKey: ["queue"],
  queryFn: () => api<Job[]>("/queue"),
  // Live events keep the queue current; this catches what they miss while something is
  // downloading or waiting, e.g. behind a proxy that holds events back.
  refetchInterval: (query: { state: { data?: Job[] } }) =>
    query.state.data?.some((job) => job.state === "queued" || job.state === "running")
      ? 5_000
      : false,
};
export const statusQuery = {
  queryKey: ["status"],
  queryFn: () => api<Status>("/status"),
  // Every page reads it; fetched again when the settings change, not on every page.
  staleTime: 5 * 60_000,
  // Also picks up a new release while the page stays open.
  refetchInterval: 30 * 60_000,
};
export const requestsQuery = {
  queryKey: ["requests"],
  queryFn: () => api<Request[]>("/requests"),
};

/** `tt` and 7 to 10 digits, as the server accepts it. */
export const imdbPattern = /^tt\d{7,10}$/;

/** "Whole series", or e.g. "Season 1, 3 episodes". */
export function describePicked(picked: Picked) {
  if (picked.all) return "Whole series";
  const parts = [
    ...picked.seasons.map(seasonName),
    picked.episodes.length > 0 &&
      `${picked.episodes.length} ${picked.episodes.length === 1 ? "episode" : "episodes"}`,
  ];
  return parts.filter(Boolean).join(", ");
}
export const settingsQuery = {
  queryKey: ["settings"],
  queryFn: () => api<Settings>("/settings"),
};

export const accountQuery = {
  queryKey: ["account"],
  queryFn: () => api<Account>("/account"),
};

export const songsQuery = {
  queryKey: ["songs"],
  queryFn: () => api<Song[]>("/theme/songs"),
};

export const sites: Site[] = ["aniworld", "serienstream"];
export const siteNames: Record<Site, string> = { aniworld: "AniWorld", serienstream: "SerienStream" };
/** What a site calls its titles, for headings like "Popular Animes". */
export const siteNouns: Record<Site, string> = { aniworld: "Animes", serienstream: "Series" };

export const languageNames: Record<Language, string> = {
  german_dub: "German dub",
  german_sub: "German sub",
  english_sub: "English sub",
  english: "English",
};
export const languages = Object.keys(languageNames) as Language[];

/** A picked language, or the default that applies when none was picked. */
export const languageLabel = (language: Language | null, defaults?: Defaults | null) => {
  const shown = language ?? defaults?.language;
  return shown ? languageNames[shown] : "Default language";
};

/** A picked provider, or the default that applies when none was picked. */
export const hosterLabel = (hoster: string | null, defaults?: Defaults | null) => {
  const shown = hoster ?? defaults?.hoster;
  return shown ? (hosterNames[shown] ?? shown) : "Default provider";
};

// shortcut: mirrors the hosters the server can extract from; serve the list from the API
// once hosters change more often than this file.
export const supportedHosters = ["voe", "vidmoly"];
export const hosterNames: Record<string, string> = { voe: "VOE", vidmoly: "Vidmoly" };

export const seasonName = (season: number) => (season === 0 ? "Films" : `Season ${season}`);

/** `S01E001`, the same form as in file names. */
export const episodeCode = (season: number, number: number) =>
  `S${String(season).padStart(2, "0")}E${String(number).padStart(3, "0")}`;

export function formatBytes(bytes: number) {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let unit = 0;
  while (bytes >= 1024 && unit < units.length - 1) {
    bytes /= 1024;
    unit += 1;
  }
  return `${bytes.toFixed(unit < 2 ? 0 : 1)} ${units[unit]}`;
}

/** A Unix time in seconds as a short local date and time. */
export function formatTime(seconds: number) {
  return new Date(seconds * 1000).toLocaleString(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  });
}
