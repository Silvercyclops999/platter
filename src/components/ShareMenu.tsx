import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { backend } from "../lib/backend";
import { formatDuration } from "../lib/format";
import type { ShareInfo, ShareKind } from "../lib/types";
import { usePlayer } from "../stores/player";
import { copyLink, copyLinksAt, copyPublicLink } from "../stores/share";
import type { MenuItem, SubmenuContext } from "../stores/ui";
import { MenuIcon } from "./ContextMenu";
import { Image as ImageIcon, Link } from "lucide-react";
import { openShareCard, type CardSubject } from "../stores/shareCard";
import "./share.css";

const NOUN: Record<ShareKind, string> = { track: "song", album: "album", artist: "artist", playlist: "playlist" };

/** What a song's share card shows (other kinds are looked up). */
export type CardDetails = Pick<CardSubject, "title" | "subtitle" | "durationMs">;

/** The "Share" entry for a song, album, artist or playlist menu. */
export function shareItem(kind: ShareKind, id: string, thumb?: string | null, card?: CardDetails): MenuItem {
  return { label: "Share", submenu: (ctx) => <ShareSubmenu kind={kind} id={id} thumb={thumb} card={card} ctx={ctx} /> };
}

/** Where the song is now, in whole seconds, when it's the one playing. */
export function playingAt(kind: ShareKind, id: string): number | null {
  if (kind !== "track") return null;
  const { state, positionMs } = usePlayer.getState();
  if (state?.track?.id !== id || (state.status !== "playing" && state.status !== "paused")) return null;
  return Math.floor(positionMs / 1000);
}

/**
 * The Share submenu: the public link (released songs and albums; greyed
 * "Not released" for unreleased music, left out when turned off in
 * Settings), the Platter link, and for the song playing now, the links at
 * the current time.
 */
export function ShareSubmenu({
  kind,
  id,
  thumb,
  card,
  ctx,
}: {
  kind: ShareKind;
  id: string;
  thumb?: string | null;
  card?: CardDetails;
  ctx: SubmenuContext;
}) {
  const art = thumb ? { path: thumb, kind: kind === "track" ? undefined : kind } : undefined;
  const [at] = useState(() => playingAt(kind, id));
  const [info, setInfo] = useState<ShareInfo | null>(null);
  const root = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let live = true;
    backend.shareInfo(kind, id).then(
      (i) => live && setInfo(i),
      () => {},
    );
    return () => {
      live = false;
    };
  }, [kind, id]);

  const run = (fn: () => unknown) => {
    ctx.close();
    fn();
  };

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const els = Array.from(root.current?.querySelectorAll<HTMLElement>("[role='menuitem']:not([aria-disabled='true'])") ?? []);
    const i = els.indexOf(document.activeElement as HTMLElement);
    els[(i + (e.key === "ArrowDown" ? 1 : -1) + els.length) % els.length]?.focus();
  };

  const showPublic = info?.public === "available" || info?.public === "unreleased";
  const unreleased = info?.public === "unreleased";
  const appLink = () => info?.appLink ?? backend.shareInfo(kind, id).then((i) => i.appLink);

  return (
    <div ref={root} className="share-menu" onKeyDown={onKeyDown}>
      {showPublic && (
        <button
          type="button"
          role="menuitem"
          tabIndex={-1}
          aria-disabled={unreleased || undefined}
          title={unreleased ? "Unreleased music doesn't get a public link" : undefined}
          className="context-menu__item share-menu__item"
          onClick={() => !unreleased && info && run(() => copyPublicLink(kind, id, info.appLink, art))}
        >
          <MenuIcon icon={Link} />
          <span className="truncate context-menu__label">Copy {NOUN[kind]} link</span>
          {unreleased && <span className="share-menu__note">Not released</span>}
        </button>
      )}
      <button type="button" role="menuitem" tabIndex={-1} className="context-menu__item share-menu__item" onClick={() => run(() => copyLink(appLink(), undefined, art))}>
        <MenuIcon icon={Link} />
          <span className="truncate context-menu__label">Copy Platter link</span>
      </button>
      {at !== null && (
        <button
          type="button"
          role="menuitem"
          tabIndex={-1}
          className="context-menu__item share-menu__item"
          onClick={() => run(() => copyLinksAt(kind, id, at, art))}
        >
          <MenuIcon icon={Link} />
          <span className="truncate context-menu__label">Copy link at {formatDuration(at * 1000)}</span>
        </button>
      )}
      {kind !== "artist" && (
        <button
          type="button"
          role="menuitem"
          tabIndex={-1}
          className="context-menu__item share-menu__item"
          onClick={() => run(() => openShareCard({ kind, id, thumb: thumb ?? null, ...card }))}
        >
          <MenuIcon icon={ImageIcon} />
          <span className="truncate context-menu__label">Share card…</span>
        </button>
      )}
    </div>
  );
}
