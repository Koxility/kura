import { useQuery } from "@tanstack/react-query";
import { load, mount, names, type Piece, type PieceName } from "ascii.rest";
import { type CSSProperties, useEffect, useRef, useState } from "react";
import { accountQuery, songsQuery } from "./api";
import styles from "./ThemeEffects.module.css";

/** Whether ascii.rest has a piece of this name; every piece ships with Kura, each loaded
 *  only when it is shown. */
export const isPieceName = (name: string): name is PieceName =>
  (names as readonly string[]).includes(name);

/** The piece name in an ascii.rest link (`https://ascii.rest/aurora-fjord/`), or a bare
 *  name. `null` when it names no piece. */
export function pieceFromLink(text: string): PieceName | null {
  const trimmed = text.trim().toLowerCase();
  const path = trimmed.replace(/^https?:\/\//, "").replace(/^(www\.)?ascii\.rest/, "");
  const name = path.replace(/^[/#]+/, "").split(/[/?#]/)[0];
  return isPieceName(name) ? name : null;
}

/** "aurora-fjord" as "Aurora fjord". */
export const pieceTitle = (name: string) => {
  const words = name.replaceAll("-", " ");
  return words.charAt(0).toUpperCase() + words.slice(1);
};

/** The user's theme: the looping song and the ASCII background. */
export function ThemeEffects() {
  const account = useQuery(accountQuery);
  const songs = useQuery(songsQuery);
  const theme = account.data?.theme;
  const list = songs.data ?? [];
  const song = list.find((known) => known.id === theme?.song) ?? list[0];
  return (
    <>
      {theme?.music && song && <Music id={song.id} volume={theme.volume} />}
      {theme?.ascii && <Background piece={theme.ascii_piece} />}
    </>
  );
}

/**
 * Plays a song on a loop while mounted. Browsers refuse sound before the person has clicked
 * or typed on the page, so a refused start waits for the first click or key. The volume
 * changes without starting the song again.
 */
function Music({ id, volume }: { id: string; volume: number }) {
  const audio = useRef<HTMLAudioElement | null>(null);
  useEffect(() => {
    const song = new Audio(`/api/v1/theme/songs/${encodeURIComponent(id)}`);
    song.loop = true;
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
  }, [id]);
  useEffect(() => {
    if (audio.current) audio.current.volume = Math.min(1, Math.max(0, volume / 100));
  }, [volume, id]);
  return null;
}

/** A faint animated scene behind the pages. Coloured pieces keep their colours, washed out;
 *  plain ones are drawn in the faintest text colour. */
function Background({ piece: name }: { piece: string }) {
  const [piece, setPiece] = useState<Piece | null>(null);
  const art = useRef<HTMLElement | null>(null);
  useEffect(() => {
    if (!isPieceName(name)) return;
    let live = true;
    setPiece(null);
    load[name]().then((module) => live && setPiece(module as unknown as Piece));
    return () => {
      live = false;
    };
  }, [name]);
  // While the picture runs, text on the page gets a plain plate (see --plate in tokens.css).
  useEffect(() => {
    document.documentElement.dataset.backdrop = "ascii";
    return () => {
      delete document.documentElement.dataset.backdrop;
    };
  }, []);
  const coloured = Boolean(piece?.meta.palette);
  useEffect(() => {
    if (!piece || !art.current) return;
    return mount(art.current, piece);
  }, [piece, coloured]);
  if (!piece) return null;
  const { cols, rows } = piece.meta;
  return (
    <div
      className={coloured ? `${styles.background} ${styles.coloured}` : styles.background}
      aria-hidden="true"
      style={{ "--cols": cols, "--rows": rows } as CSSProperties}
    >
      {coloured ? (
        <canvas ref={(element) => { art.current = element; }} className={styles.canvas} />
      ) : (
        <pre ref={(element) => { art.current = element; }} className={styles.art} />
      )}
    </div>
  );
}
