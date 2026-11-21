import { useQuery } from "@tanstack/react-query";
import { mount, type Piece } from "ascii.rest";
import { type CSSProperties, useEffect, useRef, useState } from "react";
import { accountQuery } from "./api";
import styles from "./ThemeEffects.module.css";

/**
 * The scenes the background offers. Each loads on its own when picked, so the app carries
 * these few and not all of ascii.rest's pieces.
 */
export const asciiScenes = {
  "misty-forest": { name: "Misty forest", load: () => import("ascii.rest/pieces/misty-forest") },
  "tokyo-rain": { name: "Tokyo rain", load: () => import("ascii.rest/pieces/tokyo-rain") },
  "night-coast": { name: "Night coast", load: () => import("ascii.rest/pieces/night-coast") },
  "kyoto-dusk": { name: "Kyoto dusk", load: () => import("ascii.rest/pieces/kyoto-dusk") },
  "lantern-lake": { name: "Lantern lake", load: () => import("ascii.rest/pieces/lantern-lake") },
  "ocean-sunset": { name: "Ocean sunset", load: () => import("ascii.rest/pieces/ocean-sunset") },
  "aurora-fjord": { name: "Aurora fjord", load: () => import("ascii.rest/pieces/aurora-fjord") },
  snowfall: { name: "Snowfall", load: () => import("ascii.rest/pieces/snowfall") },
} satisfies Record<string, { name: string; load: () => Promise<unknown> }>;

export type AsciiScene = keyof typeof asciiScenes;

/** The user's theme: the looping song and the ASCII background. */
export function ThemeEffects({ music }: { music: boolean }) {
  const account = useQuery(accountQuery);
  const theme = account.data?.theme;
  return (
    <>
      {theme?.music && music && <Music />}
      {theme?.ascii && <Background scene={theme.ascii_piece} />}
    </>
  );
}

/**
 * Plays the theme song on a loop while mounted. Browsers refuse sound before the person has
 * clicked or typed on the page, so a refused start waits for the first click or key.
 */
function Music() {
  const audio = useRef<HTMLAudioElement | null>(null);
  useEffect(() => {
    const song = new Audio("/api/v1/theme/music");
    song.loop = true;
    song.volume = 0.5;
    audio.current = song;
    const events = ["pointerdown", "keydown"] as const;
    const start = () => {
      song.play().then(
        () => events.forEach((name) => window.removeEventListener(name, start)),
        () => {},
      );
    };
    song.play().catch(() => events.forEach((name) => window.addEventListener(name, start)));
    return () => {
      events.forEach((name) => window.removeEventListener(name, start));
      song.pause();
      song.removeAttribute("src");
      song.load();
      audio.current = null;
    };
  }, []);
  return null;
}

/** A faint animated scene behind the pages, in the text colour, fading out at the edges. */
function Background({ scene }: { scene: string }) {
  const [piece, setPiece] = useState<Piece | null>(null);
  const art = useRef<HTMLPreElement>(null);
  useEffect(() => {
    const known = asciiScenes[scene as AsciiScene] ?? asciiScenes["misty-forest"];
    let live = true;
    known.load().then((module) => live && setPiece(module as unknown as Piece));
    return () => {
      live = false;
    };
  }, [scene]);
  // Drawn into a <pre> as text in one ink. The library's React component would bring every
  // one of its pieces into the build; mounting directly keeps only the scenes above.
  useEffect(() => {
    if (!piece || !art.current) return;
    return mount(art.current, piece);
  }, [piece]);
  if (!piece) return null;
  const { cols, rows } = piece.meta;
  return (
    <div
      className={styles.background}
      aria-hidden="true"
      style={{ "--cols": cols, "--rows": rows } as CSSProperties}
    >
      <pre ref={art} className={styles.art} />
    </div>
  );
}
