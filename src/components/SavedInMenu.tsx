import { Check, Pin, Plus } from "lucide-react";
import { useEffect, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { backend } from "../lib/backend";
import { plural } from "../lib/format";
import type { Playlist } from "../lib/types";
import { useApp } from "../stores/app";
import { isLiked, likedMinRating, ratingHint, setLiked, useLikes, type SavedInAsk } from "../stores/likes";
import { createPlaylist } from "../stores/playlists";
import { toast } from "../stores/toasts";
import { Art } from "./Art";
import { Button } from "./Button";
import { placeMenu } from "./ContextMenu";
import { LikedCover } from "./LikedCover";
import "./likes.css";
import { failText } from "../lib/errorText";

/** Mounted once in the shell: shows the saved-in menu when one is asked for. */
export function SavedInHost() {
  const ask = useLikes((s) => s.savedIn);
  if (!ask) return null;
  return <SavedInMenu key={ask.track.id} ask={ask} />;
}


/** Playlists the song is in when the menu opened (null while loading). */
function usePlaylistsWith(trackId: string): Set<string> | null {
  const [ids, setIds] = useState<Set<string> | null>(null);
  useEffect(() => {
    let live = true;
    backend.playlistsWithTrack(trackId).then(
      (list) => live && setIds(new Set(list)),
      () => live && setIds(new Set()),
    );
    return () => {
      live = false;
    };
  }, [trackId]);
  return ids;
}

/** Remove every row of `trackId` from a playlist. */
async function removeFrom(p: Playlist, trackId: string) {
  const page = await backend.playlistPage(p.id);
  const items = page.itemIds.filter((_, i) => page.tracks[i]?.id === trackId);
  if (items.length) await backend.playlistRemove(p.id, items);
}

interface Change {
  label: string;
  run: () => Promise<unknown>;
}

/** One summary toast for what Done changed. */
function doneMessage(added: string[], removed: string[]): string {
  if (added.length + removed.length > 1) return "Changes saved";
  if (added.length) return `Added to ${added[0]}`;
  return `Removed from ${removed[0]}`;
}

/**
 * "Add to playlist" for one song: Liked Songs and the playlists it's in
 * (ticked), then your other playlists. Ticks toggle like checkboxes and
 * Done applies them all at once; Cancel, Escape or a click outside changes
 * nothing.
 */
export function SavedInMenu({ ask }: { ask: SavedInAsk }) {
  const { track } = ask;
  const close = useLikes((s) => s.closeSavedIn);
  const all = useApp((s) => s.playlists);
  const likedCount = useLikes((s) => s.ids.size);
  const containing = usePlaylistsWith(track.id);
  const [likedAtOpen] = useState(() => isLiked(track.id));
  const [liked, setLikedTick] = useState(likedAtOpen);
  /** Playlist id → ticked, for the ones changed here. */
  const [ticks, setTicks] = useState<Map<string, boolean>>(() => new Map());
  const [query, setQuery] = useState("");
  const ref = useRef<HTMLDivElement>(null);
  const find = useRef<HTMLInputElement>(null);
  const [pos, setPos] = useState<{ x: number; y: number } | null>(null);

  /** Close and give focus back to the control it opened from. */
  const finish = () => {
    close();
    const back = ask.returnFocus;
    if (back?.isConnected) back.focus();
  };
  const finishRef = useRef(finish);
  finishRef.current = finish;

  // Place beside the control once measured; centred above the player bar without one.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    const view = { width: window.innerWidth, height: window.innerHeight };
    if (ask.anchor) setPos(placeMenu({ x: ask.anchor.left, y: ask.anchor.bottom, anchor: ask.anchor }, { width: r.width, height: r.height }, view));
    else setPos({ x: Math.max(8, (view.width - r.width) / 2), y: Math.max(8, view.height - r.height - 120) });
  }, [ask.anchor, containing]);

  useEffect(() => {
    if (pos) find.current?.focus();
  }, [pos]);

  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) close();
    };
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopImmediatePropagation();
        finishRef.current();
      }
    };
    const onResize = () => close();
    window.addEventListener("mousedown", onDown, true);
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("resize", onResize);
    return () => {
      window.removeEventListener("mousedown", onDown, true);
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("resize", onResize);
    };
  }, [close]);

  const q = query.trim().toLowerCase();
  const matches = (title: string) => !q || title.toLowerCase().includes(q);
  const playlists = all ?? [];
  const savedIn = useMemo(
    () => (containing ? [...containing].map((id) => playlists.find((p) => p.id === id)).filter((p): p is Playlist => !!p) : []),
    [containing, playlists],
  );
  const others = useMemo(
    () =>
      playlists
        .filter((p) => !p.smart && !containing?.has(p.id))
        .sort((a, b) => (b.updatedAt ?? 0) - (a.updatedAt ?? 0) || a.title.localeCompare(b.title)),
    [playlists, containing],
  );
  const ticked = (p: Playlist) => ticks.get(p.id) ?? !!containing?.has(p.id);
  const toggle = (p: Playlist) =>
    setTicks((m) => {
      const next = new Map(m);
      const was = !!containing?.has(p.id);
      const now = !(m.get(p.id) ?? was);
      if (now === was) next.delete(p.id);
      else next.set(p.id, now);
      return next;
    });

  const changes = (): { list: Change[]; added: string[]; removed: string[] } => {
    const list: Change[] = [];
    const added: string[] = [];
    const removed: string[] = [];
    if (liked !== likedAtOpen) {
      (liked ? added : removed).push("Liked Songs");
      list.push({ label: "Liked Songs", run: () => setLiked([track], liked, { toast: false }) });
    }
    for (const [id, on] of ticks) {
      const p = playlists.find((x) => x.id === id);
      if (!p) continue;
      (on ? added : removed).push(p.title);
      list.push({ label: p.title, run: () => (on ? backend.playlistAdd(p.id, [track.id]) : removeFrom(p, track.id)) });
    }
    return { list, added, removed };
  };
  const dirty = liked !== likedAtOpen || ticks.size > 0;

  /** Apply every change; one toast for all of them. */
  const apply = async (andThen?: () => Promise<unknown>) => {
    const { list, added, removed } = changes();
    finish();
    const failed: string[] = [];
    for (const c of list) {
      try {
        const ok = await c.run();
        if (ok === false) failed.push(c.label);
      } catch (e) {
        failed.push(c.label);
        toast.error(failText(`Couldn't change ${c.label}`, e));
      }
    }
    if (list.length && failed.length < list.length) toast.info(doneMessage(added.filter((a) => !failed.includes(a)), removed.filter((r) => !failed.includes(r))));
    await andThen?.();
  };

  const newPlaylist = () => void apply(() => createPlaylist({ trackIds: [track.id] }));

  const rowKeys = (e: KeyboardEvent) => {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const els = Array.from(ref.current?.querySelectorAll<HTMLElement>("input, .saved-in__row:not([aria-disabled='true']), .saved-in__new") ?? []);
    const i = els.indexOf(document.activeElement as HTMLElement);
    els[(i + (e.key === "ArrowDown" ? 1 : -1) + els.length) % els.length]?.focus();
  };

  const hint = ratingHint(track, liked, likedMinRating());
  const showLiked = matches("Liked Songs");
  const shownSaved = savedIn.filter((p) => matches(p.title));
  const shownOthers = others.filter((p) => matches(p.title));

  return createPortal(
    <div
      ref={ref}
      className="saved-in"
      role="dialog"
      aria-label={`Add ${track.title} to playlist`}
      style={{ left: pos?.x ?? 0, top: pos?.y ?? 0, visibility: pos ? "visible" : "hidden" }}
      onKeyDown={rowKeys}
      onContextMenu={(e) => e.preventDefault()}
    >
      <h2 className="saved-in__title">Add to playlist</h2>
      <input
        ref={find}
        className="pl-picker__find saved-in__find"
        type="search"
        placeholder="Find a playlist"
        aria-label="Find a playlist"
        value={query}
        spellCheck={false}
        autoComplete="off"
        onChange={(e) => setQuery(e.target.value)}
      />
      <button type="button" className="saved-in__new" onClick={newPlaylist}>
        <span className="pl-picker__plus" aria-hidden="true">
          <Plus size={16} />
        </span>
        <span>New playlist</span>
      </button>
      <div className="saved-in__list">
        {(showLiked || shownSaved.length > 0) && (
          <Section title="Saved in">
            {showLiked && (
              <Row
                art={<LikedCover size={32} />}
                title="Liked Songs"
                sub={
                  <>
                    <Pin size={11} className="saved-in__pin" aria-label="Pinned" />
                    {plural(likedCount + (liked && !likedAtOpen ? 1 : 0) - (!liked && likedAtOpen ? 1 : 0), "song")}
                  </>
                }
                hint={hint}
                checked={liked}
                onToggle={() => setLikedTick((v) => !v)}
              />
            )}
            {shownSaved.map((p) => (
              <Row
                key={p.id}
                art={<Art path={p.thumb} size={32} kind="playlist" />}
                title={p.title}
                sub={plural(p.trackCount, "song")}
                checked={ticked(p)}
                disabled={p.smart}
                onToggle={() => toggle(p)}
              />
            ))}
          </Section>
        )}
        {containing && shownOthers.length > 0 && (
          <Section title="Other playlists">
            {shownOthers.map((p) => (
              <Row
                key={p.id}
                art={<Art path={p.thumb} size={32} kind="playlist" />}
                title={p.title}
                sub={plural(p.trackCount, "song")}
                checked={ticked(p)}
                onToggle={() => toggle(p)}
              />
            ))}
          </Section>
        )}
        {q && !showLiked && shownSaved.length === 0 && shownOthers.length === 0 && <p className="pl-picker__none">No playlists match "{query.trim()}"</p>}
      </div>
      <div className="saved-in__foot">
        <Button size="sm" variant="ghost" onClick={finish}>
          Cancel
        </Button>
        <Button size="sm" variant="primary" onClick={() => (dirty ? void apply() : finish())}>
          Done
        </Button>
      </div>
    </div>,
    document.body,
  );
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div className="saved-in__section" role="group" aria-label={title}>
      <h3 className="saved-in__heading">{title}</h3>
      {children}
    </div>
  );
}

function Row({
  art,
  title,
  sub,
  hint,
  checked,
  disabled,
  onToggle,
}: {
  art: ReactNode;
  title: string;
  sub: ReactNode;
  hint?: string | null;
  checked: boolean;
  disabled?: boolean;
  onToggle: () => void;
}) {
  return (
    <button
      type="button"
      role="checkbox"
      aria-checked={checked}
      aria-disabled={disabled || undefined}
      title={disabled ? "Smart playlists change by their rules" : undefined}
      className={`saved-in__row ${checked ? "is-checked" : ""}`}
      onClick={() => !disabled && onToggle()}
    >
      <span className="saved-in__art">{art}</span>
      <span className="saved-in__text">
        <span className="saved-in__name truncate">{title}</span>
        <span className="saved-in__sub truncate">
          {sub}
          {hint && <span className="saved-in__hint"> · {hint}</span>}
        </span>
      </span>
      <span className="saved-in__box" aria-hidden="true">
        {checked && <Check size={12} strokeWidth={3} />}
      </span>
    </button>
  );
}
