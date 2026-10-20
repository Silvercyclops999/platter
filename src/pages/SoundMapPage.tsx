/**
 * The sound map: every analysed song as a dot, placed so songs that sound
 * alike sit together, coloured by genre or by era, with unreleased songs
 * ringed. Hover a dot for the song, click it to play, draw a lasso round a
 * patch to play it or make it a playlist, or search to fly to a song or an
 * artist. It fills in as Automix analyses songs; everything is local.
 */
import { Crosshair, Lasso, Move, Play, X } from "lucide-react";
import { useCallback, useEffect, useId, useMemo, useRef, useState } from "react";
import { Art } from "../components/Art";
import { Button, IconButton } from "../components/Button";
import { Dialog } from "../components/Dialog";
import { SearchField } from "../components/SearchField";
import { SkeletonLines } from "../components/Skeleton";
import { backend } from "../lib/backend";
import type { SoundMapChunk, SoundMapFound, SoundMapInfo, SoundMapSong } from "../lib/types";
import { player } from "../stores/player";
import { createPlaylist } from "../stores/playlists";
import { nav } from "../stores/router";
import type { PageId } from "./settingsMeta";
import { useSettings } from "../stores/settings";
import { useSoundMap, useSoundMapOn } from "../stores/soundmap";
import { MapCanvas, type MapApi, type MapMode } from "./soundmap/MapCanvas";
import { emptyData, joinChunks, legend as makeLegend, type ColourBy, type MapData } from "./soundmap/geometry";
import "./ledger.css";
import "./soundmap.css";
import { LoadError, TakingLong } from "../components/LoadError";
import { useTakingLong } from "../lib/useLoad";

/** Points per request. */
export const CHUNK = 20_000;
/** How long a dot is hovered before its card shows. */
const CARD_DELAY_MS = 120;
/** Selections up to this size play in a walk from one song to the most alike next. */
const WALK_MAX = 1500;

const fmt = (n: number) => n.toLocaleString();
const songs = (n: number) => (n === 1 ? "1 song" : `${fmt(n)} songs`);

/** Opens Settings at one row (on its page, with its More open). */
export function openSetting(key: string) {
  openSettingsAt(`[data-setting="${key}"]`, { row: key });
}

/** Opens Settings (or one of its pages) and scrolls to what `selector` finds (a row or a section), focusing its first control. */
export function openSettingsAt(selector: string, to: { page?: PageId; row?: string } = {}) {
  nav.go({ name: "settings", ...to });
  let tries = 0;
  const look = () => {
    const row = document.querySelector<HTMLElement>(selector);
    if (row) {
      row.scrollIntoView?.({ block: "center" });
      row.querySelector<HTMLElement>("button, [role=switch], input")?.focus({ preventScroll: true });
    } else if (tries++ < 60) requestAnimationFrame(look);
  };
  requestAnimationFrame(look);
}

/** The whole map, chunk by chunk. */
export async function loadAll(): Promise<MapData> {
  const chunks: SoundMapChunk[] = [];
  let after: string | null = null;
  for (;;) {
    const c: SoundMapChunk = await backend.soundmapPoints(after, CHUNK, null);
    chunks.push(c);
    if (!c.next) break;
    after = c.next;
  }
  return joinChunks(chunks);
}

/** Play order for a selection: each song followed by the most alike one left (a walk across the patch). */
export function walkOrder(d: MapData, picked: number[]): number[] {
  if (picked.length <= 2) return picked;
  if (picked.length > WALK_MAX) return [...picked].sort((a, b) => d.x[a] - d.x[b]);
  const left = new Set(picked);
  let at = picked.reduce((a, b) => (d.x[b] < d.x[a] ? b : a));
  const out = [at];
  left.delete(at);
  while (left.size) {
    let best = -1;
    let bestD = Infinity;
    for (const j of left) {
      const dd = (d.x[j] - d.x[at]) ** 2 + (d.y[j] - d.y[at]) ** 2;
      if (dd < bestD) {
        bestD = dd;
        best = j;
      }
    }
    out.push(best);
    left.delete(best);
    at = best;
  }
  return out;
}

/** The theme's colours for the dots (read from CSS so light and dark themes each get theirs). */
export function useColours(el: HTMLElement | null, by: ColourBy, groups: { slot: number }[]) {
  const theme = useSettings((s) => s.settings?.theme);
  const [tick, setTick] = useState(0);
  useEffect(() => {
    if (typeof MutationObserver === "undefined") return;
    const mo = new MutationObserver(() => setTick((t) => t + 1));
    mo.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme-base", "data-theme", "style"] });
    return () => mo.disconnect();
  }, []);
  return useMemo(() => {
    const css = el ? getComputedStyle(el) : null;
    const read = (name: string, fallback: string) => css?.getPropertyValue(name).trim() || fallback;
    const prefix = by === "era" ? "--sm-era-" : "--sm-genre-";
    const colours = groups.map((g) => (g.slot < 0 ? read("--sm-other", "#777") : read(`${prefix}${g.slot}`, "#999")));
    return { colours, ring: read("--sm-ring", "#fff"), highlight: read("--sm-highlight", "#fff") };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [el, by, groups, theme, tick]);
}

/** The map, or (`album`) the map with that record's songs picked and in view. */
export function SoundMapPage({ album }: { album?: string } = {}) {
  const on = useSoundMapOn();
  const [info, setInfo] = useState<SoundMapInfo | null>(null);
  const [data, setData] = useState<MapData | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [by, setBy] = useState<ColourBy>("genre");
  const [mode, setMode] = useState<MapMode>("move");
  const [picked, setPicked] = useState<number[]>([]);
  const [focus, setFocus] = useState(-1);
  const [naming, setNaming] = useState(false);
  const [root, setRoot] = useState<HTMLElement | null>(null);
  /** What the live region says (selection size, the song in focus). */
  const [said, setSaid] = useState("");
  const api = useRef<MapApi | null>(null);
  /** The points shown and what they were loaded for, so a reload with nothing new is skipped. */
  const shown = useRef<{ data: MapData; placed: number; revision: number } | null>(null);
  /** Each load's number: a slower, older one finishing late is ignored. */
  const request = useRef(0);

  const refresh = useCallback(async () => {
    const n = ++request.current;
    try {
      const i = await backend.soundmapInfo();
      if (n !== request.current) return;
      setInfo(i);
      // Explore's entry on Home follows what the page knows.
      useSoundMap.setState({ info: i });
      const was = shown.current;
      if (was && was.placed === i.placed && was.revision === i.revision) {
        setError(null);
        return;
      }
      const d = i.placed > 0 ? await loadAll() : emptyData();
      if (n !== request.current) return;
      shown.current = { data: d, placed: i.placed, revision: i.revision };
      setData(d);
      // The selection and the song in focus follow their songs to the new points.
      if (was) {
        const move = (k: number) => {
          const id = was.data.ids[k];
          return id === undefined ? -1 : (d.index.get(id) ?? -1);
        };
        setPicked((p) => p.map(move).filter((k) => k >= 0));
        setFocus((f) => (f < 0 ? -1 : move(f)));
      }
      setError(null);
    } catch (e) {
      if (n !== request.current) return;
      setError(e && typeof e === "object" && "message" in e ? String((e as { message: unknown }).message) : "");
    }
  }, []);

  useEffect(() => {
    if (!on) return;
    void refresh();
    let t: ReturnType<typeof setTimeout> | null = null;
    const later = () => {
      if (t) clearTimeout(t);
      t = setTimeout(() => void refresh(), 800);
    };
    const a = backend.listen("soundmap://changed", later);
    const b = backend.listen("analysis://changed", () => void backend.soundmapInfo().then(setInfo, () => {}));
    return () => {
      if (t) clearTimeout(t);
      // Anything still loading is from before: ignored when it lands.
      request.current++;
      void a.then((f) => f());
      void b.then((f) => f());
    };
  }, [on, refresh]);

  // Say how many songs are selected as it changes.
  useEffect(() => {
    if (picked.length > 0) setSaid(`${songs(picked.length)} selected`);
  }, [picked.length]);

  const legend = useMemo(() => makeLegend(data ?? emptyData(), by), [data, by]);
  const { colours, ring, highlight } = useColours(root, by, legend.groups);
  const selected = useMemo(() => {
    if (!data || picked.length === 0) return null;
    const m = new Uint8Array(data.ids.length);
    for (const i of picked) m[i] = 1;
    return m;
  }, [data, picked]);

  const hover = useHoverCard(data);

  // Opened on a record (a crate's "Sounds like this record"): pick its songs
  // once the map is in, and move there.
  const [record, setRecord] = useState<{ title: string; count: number } | null>(null);
  const aimed = useRef(false);
  const hasPoints = !!data && data.ids.length > 0;
  useEffect(() => {
    if (!album || !hasPoints || aimed.current) return;
    aimed.current = true;
    let alive = true;
    backend.albumPage(album).then(
      (page) => {
        const d = shown.current?.data;
        if (!alive || !d) return;
        const ids = page.tracks.map((t) => d.index.get(t.id) ?? -1).filter((i) => i >= 0);
        if (!ids.length) {
          setSaid(`${page.album.title} isn't on the map yet.`);
          return;
        }
        let x = 0;
        let y = 0;
        for (const i of ids) {
          x += d.x[i];
          y += d.y[i];
        }
        setFocus(-1);
        setPicked(ids);
        setRecord({ title: page.album.title, count: ids.length });
        setSaid(`${songs(ids.length)} from ${page.album.title} selected. Songs that sound like it sit around them.`);
        api.current?.flyTo(x / ids.length, y / ids.length, 6);
      },
      () => {},
    );
    return () => {
      alive = false;
    };
  }, [album, hasPoints]);

  if (!on) {
    return (
      <div className="page-pad soundmap" data-testid="soundmap-page" ref={setRoot}>
        <h1 className="page-title">Sound map</h1>
        <div className="soundmap__empty">
          <p>The sound map is turned off.</p>
          <Button variant="secondary" onClick={() => openSetting("showSoundMap")}>
            Sound map settings
          </Button>
        </div>
      </div>
    );
  }

  const placed = info?.placed ?? 0;
  const ready = !!data && !!info;
  const slow = useTakingLong(!ready && !error);
  const partial = info && data && placed > 0 && info.analysing && info.analysed < info.total ? info : null;

  const play = (ids: number[]) => {
    if (!data || ids.length === 0) return;
    const order = walkOrder(data, ids).map((i) => data.ids[i]);
    void player.playTracks(order, order[0]);
  };

  return (
    <div className="page-pad soundmap" data-testid="soundmap-page" ref={setRoot}>
      <div className="sr-only" role="status" aria-live="polite" aria-atomic="true" data-testid="soundmap-status">
        {said}
      </div>
      <header className="soundmap__head">
        <div className="soundmap__titles">
          <h1 className="page-title">Sound map</h1>
          {ready && placed > 0 ? <MapLead placed={placed} groups={legend.groups} by={by} /> : <p className="soundmap__sub">Songs that sound alike sit together.</p>}
        </div>
        {ready && placed > 0 && (
          <div className="soundmap__tools">
            <FindBox
              onFound={(f) => {
                if (!data) return;
                if (f.kind === "song") {
                  const i = data.index.get(f.id) ?? -1;
                  setPicked([]);
                  setFocus(i);
                  api.current?.flyTo(f.x, f.y, 12);
                } else {
                  setFocus(-1);
                  api.current?.flyTo(f.x, f.y, 4);
                }
              }}
              onPlay={(f) => {
                const i = data?.index.get(f.id) ?? -1;
                if (i >= 0) {
                  setFocus(i);
                  api.current?.flyTo(f.x, f.y, 12);
                }
                void player.playTracks([f.id], f.id);
              }}
            />
            <div className="soundmap__seg" role="group" aria-label="Colour by">
              {(["genre", "era"] as const).map((k) => (
                <button key={k} type="button" aria-pressed={by === k} className="soundmap__seg-btn" onClick={() => setBy(k)}>
                  {k === "genre" ? "Genre" : "Era"}
                </button>
              ))}
            </div>
            <div className="soundmap__seg" role="group" aria-label="Dragging">
              <button type="button" aria-pressed={mode === "move"} className="soundmap__seg-btn" onClick={() => setMode("move")}>
                <Move size={14} aria-hidden="true" /> Move
              </button>
              <button type="button" aria-pressed={mode === "lasso"} className="soundmap__seg-btn" onClick={() => setMode("lasso")}>
                <Lasso size={14} aria-hidden="true" /> Lasso
              </button>
            </div>
            <IconButton label="Show the whole map" icon={<Crosshair size={18} />} onClick={() => api.current?.reset()} />
          </div>
        )}
      </header>

      {partial && (
        <p className="soundmap__note">
          {fmt(partial.analysed)} of {fmt(partial.total)} songs analysed. More appear as Automix analyses them.{" "}
          <button type="button" className="soundmap__link" onClick={() => openSetting("automix")}>
            Automix settings
          </button>
        </p>
      )}

      {error && !data && <LoadError what="the sound map" error={error} onRetry={() => void refresh()} inline />}
      {!ready && !error && <SkeletonLines lines={3} widths={["60%", "80%", "40%"]} gap={20} />}
      {slow && <TakingLong what="the sound map" onRetry={() => void refresh()} />}

      {info && data && placed === 0 && <EmptyState info={info} />}

      {info && data && placed > 0 && (
        <>
          <Legend groups={legend.groups} colours={colours} by={by} />
          <div className="soundmap__stage">
            <MapCanvas
              data={data}
              legend={legend}
              colours={colours}
              ring={ring}
              highlight={highlight}
              mode={mode}
              selected={selected}
              focus={focus}
              api={api}
              label={`Sound map of ${songs(placed)}. Drag to move, scroll to zoom. With the map focused, arrow keys move it and plus or minus zoom; Enter picks the song in the middle, then arrow keys step to the nearest song that way and Enter plays it.`}
              onHover={hover.onHover}
              onPick={(i) => {
                setFocus(i);
                void player.playTracks([data.ids[i]], data.ids[i]);
              }}
              onFocus={(i) => {
                setPicked([]);
                setFocus(i);
              }}
              onPlay={(i) => play([i])}
              onLasso={(ids) => {
                setPicked(ids);
                if (ids.length) setFocus(-1);
              }}
              onEscape={() => {
                setPicked([]);
                setFocus(-1);
              }}
            />
            {hover.card && (
              <div className="soundmap__card" ref={hover.cardRef} aria-hidden="true" data-testid="soundmap-card">
                <Art path={hover.card.song.thumb} size={40} />
                <div className="soundmap__card-text">
                  <span className="soundmap__card-title">{hover.card.song.title}</span>
                  <span className="soundmap__card-sub">{hover.card.song.artist}</span>
                  {hover.card.unreleased && <span className="soundmap__card-tag">Unreleased</span>}
                </div>
              </div>
            )}
            {focus >= 0 && picked.length === 0 && (
              <FocusCard
                id={data.ids[focus]}
                onPlay={() => play([focus])}
                onClose={() => setFocus(-1)}
                onShown={(s) => setSaid(`${s.title} by ${s.artist}. Enter on the map plays it.`)}
              />
            )}
            {picked.length > 0 && (
              <div className="soundmap__selection" role="region" aria-label="Selection">
                <span className="soundmap__selection-count">
                  {record && record.count === picked.length ? `${songs(picked.length)} from ${record.title}` : `${songs(picked.length)} selected`}
                </span>
                <Button size="sm" variant="primary" icon={<Play size={14} />} onClick={() => play(picked)}>
                  Play
                </Button>
                <Button size="sm" variant="secondary" onClick={() => setNaming(true)}>
                  Make playlist
                </Button>
                <IconButton label="Clear selection" size="sm" icon={<X size={16} />} onClick={() => setPicked([])} />
              </div>
            )}
          </div>
          <p className="soundmap__hint">
            {mode === "lasso" ? "Draw round a patch of songs to select them." : "Hold Shift and drag, or switch to Lasso, to select a patch of songs."} Ringed dots are
            unreleased.
          </p>
        </>
      )}

      {naming && data && (
        <NameDialog
          count={picked.length}
          onClose={() => setNaming(false)}
          onSave={async (title) => {
            const ids = walkOrder(data, picked).map((i) => data.ids[i]);
            const p = await createPlaylist({ trackIds: ids, title, open: false });
            if (p) {
              setNaming(false);
              setPicked([]);
            }
          }}
        />
      )}
    </div>
  );
}

function EmptyState({ info }: { info: SoundMapInfo }) {
  return (
    <div className="soundmap__empty" data-testid="soundmap-empty">
      <p className="soundmap__empty-title">The map fills in as songs are analysed</p>
      <p>
        Automix works out how each song sounds on this computer while it's idle, and each song takes its place here once it's done.{" "}
        {fmt(info.analysed)} of {fmt(info.total)} songs analysed so far.
        {!info.analysing && " Turn on Automix to start."}
      </p>
      <Button variant="secondary" onClick={() => openSetting("automix")}>
        Automix settings
      </Button>
    </div>
  );
}

/**
 * The page's sentence: how many songs are on the map, then its biggest
 * patch by genre, or the decade most of them come from.
 */
export function MapLead({ placed, groups, by, className = "" }: { placed: number; groups: { label: string; count: number; slot: number }[]; by: ColourBy; className?: string }) {
  const top = groups.filter((g) => g.slot >= 0 && g.count > 0).sort((a, b) => b.count - a.count)[0];
  return (
    <p className={`lg-lead lg-lead--sm soundmap__sub ${className}`} data-testid="soundmap-lead">
      {songs(placed)} on the map, each one near the songs it sounds like.
      {top &&
        (by === "genre" ? (
          <>
            {" "}
            <strong>{top.label}</strong> is the biggest patch, with {fmt(top.count)}.
          </>
        ) : (
          <>
            {" "}
            The most come from the <strong>{top.label}</strong>, {fmt(top.count)} of them.
          </>
        ))}
    </p>
  );
}

export function Legend({ groups, colours, by }: { groups: { label: string; count: number }[]; colours: string[]; by: ColourBy }) {
  return (
    <ul className="soundmap__legend" aria-label={by === "genre" ? "Genres" : "Eras"}>
      {groups.map((g, i) =>
        g.count > 0 ? (
          <li key={g.label} className="soundmap__legend-item">
            <span className="soundmap__swatch" style={{ background: colours[i] }} aria-hidden="true" />
            <span>{g.label}</span>
            <span className="soundmap__legend-count lg-num">{fmt(g.count)}</span>
          </li>
        ) : null,
      )}
      <li className="soundmap__legend-item">
        <span className="soundmap__swatch soundmap__swatch--ring" aria-hidden="true" />
        <span>Unreleased</span>
      </li>
    </ul>
  );
}

const songCache = new Map<string, Promise<SoundMapSong | null>>();
function songOf(id: string): Promise<SoundMapSong | null> {
  let p = songCache.get(id);
  if (!p) {
    p = backend.soundmapSong(id).catch(() => null);
    songCache.set(id, p);
    if (songCache.size > 500) songCache.delete(songCache.keys().next().value!);
  }
  return p;
}

/**
 * The hover card: shows after a short hover and follows the pointer. Its
 * place is set on the element directly, so moving over a dot doesn't
 * render the page again.
 */
export function useHoverCard(data: MapData | null) {
  const [card, setCard] = useState<{ song: SoundMapSong; unreleased: boolean } | null>(null);
  const at = useRef(-1);
  const pos = useRef<[number, number]>([0, 0]);
  const el = useRef<HTMLElement | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => () => void (timer.current && clearTimeout(timer.current)), []);
  const place = (e: HTMLElement | null) => {
    if (!e) return;
    e.style.left = `${pos.current[0]}px`;
    e.style.top = `${pos.current[1]}px`;
  };
  const cardRef = useCallback((e: HTMLElement | null) => {
    el.current = e;
    place(e);
  }, []);
  const onHover = useCallback(
    (i: number, sx: number, sy: number) => {
      pos.current = [sx, sy];
      if (i === at.current) {
        if (i >= 0) place(el.current);
        return;
      }
      at.current = i;
      if (timer.current) clearTimeout(timer.current);
      setCard(null);
      if (i < 0 || !data) return;
      const id = data.ids[i];
      const unreleased = data.unreleased[i] === 1;
      timer.current = setTimeout(() => {
        void songOf(id).then((song) => {
          if (song && at.current === i) setCard({ song, unreleased });
        });
      }, CARD_DELAY_MS);
    },
    [data],
  );
  return { card, onHover, cardRef };
}

/** A song found by search or clicked: its card with Play. */
function FocusCard({ id, onPlay, onClose, onShown }: { id: string; onPlay: () => void; onClose: () => void; onShown: (s: SoundMapSong) => void }) {
  const [song, setSong] = useState<SoundMapSong | null>(null);
  const shown = useRef(onShown);
  shown.current = onShown;
  useEffect(() => {
    let alive = true;
    void songOf(id).then((s) => {
      if (!alive) return;
      setSong(s);
      if (s) shown.current(s);
    });
    return () => {
      alive = false;
    };
  }, [id]);
  if (!song) return null;
  return (
    <div className="soundmap__focus" role="region" aria-label={`${song.title} by ${song.artist}`}>
      <Art path={song.thumb} size={48} />
      <div className="soundmap__card-text">
        <span className="soundmap__card-title">{song.title}</span>
        <span className="soundmap__card-sub">{song.artist}</span>
      </div>
      <IconButton label={`Play ${song.title}`} variant="accent" icon={<Play size={16} />} onClick={onPlay} />
      <IconButton label="Close" size="sm" icon={<X size={16} />} onClick={onClose} />
    </div>
  );
}

/**
 * Search to fly to a song or an artist: a combobox. Arrow keys move through
 * the results, Enter goes to one (Shift+Enter plays a song), Escape closes
 * the list and then clears the search.
 */
export function FindBox({ onFound, onPlay }: { onFound: (f: SoundMapFound) => void; onPlay: (f: SoundMapFound) => void }) {
  const [q, setQ] = useState("");
  const [found, setFound] = useState<SoundMapFound[]>([]);
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(-1);
  const listId = useId();
  const hintId = useId();
  const optionId = (i: number) => `${listId}-o${i}`;
  useEffect(() => {
    const query = q.trim();
    if (!query) {
      setFound([]);
      setActive(-1);
      return;
    }
    let alive = true;
    const t = setTimeout(
      () =>
        void backend.soundmapFind(query).then(
          (f) => {
            if (!alive) return;
            setFound(f);
            setActive(-1);
          },
          () => {},
        ),
      150,
    );
    return () => {
      alive = false;
      clearTimeout(t);
    };
  }, [q]);
  const showing = open && q.trim() !== "";
  const pick = (f: SoundMapFound) => {
    onFound(f);
    setOpen(false);
  };
  const play = (f: SoundMapFound) => {
    onPlay(f);
    setOpen(false);
  };
  const onKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    const n = found.length;
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      if (!showing) setOpen(true);
      if (n === 0) return;
      const down = e.key === "ArrowDown";
      setActive((a) => (a < 0 ? (down ? 0 : n - 1) : (a + (down ? 1 : n - 1)) % n));
      return;
    }
    if (e.key === "Enter") {
      e.preventDefault();
      const f = found[active] ?? found[0];
      if (!f) return;
      if (e.shiftKey && f.kind === "song") play(f);
      else pick(f);
      return;
    }
    if (e.key === "Escape" && showing) {
      // First Escape closes the list; the next one clears the search.
      e.preventDefault();
      e.stopPropagation();
      setOpen(false);
      setActive(-1);
    }
  };
  return (
    <div className="soundmap__find" onBlur={(e) => !e.currentTarget.contains(e.relatedTarget as Node) && setOpen(false)}>
      <SearchField
        value={q}
        onChange={(v) => {
          setQ(v);
          setOpen(true);
        }}
        onKeyDown={onKeyDown}
        label="Find a song or artist on the map"
        placeholder="Find a song or artist"
        inputProps={{
          role: "combobox",
          "aria-autocomplete": "list",
          "aria-controls": listId,
          "aria-expanded": showing && found.length > 0,
          "aria-activedescendant": showing && active >= 0 && active < found.length ? optionId(active) : undefined,
          "aria-describedby": hintId,
        }}
      />
      <span id={hintId} className="sr-only">
        Enter goes to the song or artist, Shift and Enter plays a song.
      </span>
      {showing && (
        <div className="soundmap__results">
          {found.length === 0 ? (
            <p className="soundmap__results-none" role="status">
              Not on the map yet
            </p>
          ) : null}
          <ul id={listId} role="listbox" aria-label="On the map" className="soundmap__results-list" hidden={found.length === 0}>
            {found.map((f, i) => (
              <li
                key={`${f.kind}:${f.id}`}
                id={optionId(i)}
                role="option"
                aria-selected={i === active}
                className="soundmap__result"
                // Keep focus in the search field.
                onMouseDown={(e) => e.preventDefault()}
                onClick={() => pick(f)}
              >
                <span className="soundmap__result-text">
                  <span className="soundmap__result-title">{f.title}</span>
                  <span className="soundmap__result-sub">
                    {f.kind === "artist" ? "Artist" : "Song"} · {f.sub}
                  </span>
                </span>
                {f.kind === "song" && (
                  // For the mouse; keyboards play with Shift and Enter.
                  <button
                    type="button"
                    className="soundmap__result-play"
                    tabIndex={-1}
                    aria-hidden="true"
                    title={`Play ${f.title}`}
                    data-testid="soundmap-result-play"
                    onClick={(e) => {
                      e.stopPropagation();
                      play(f);
                    }}
                  >
                    <Play size={14} aria-hidden="true" />
                  </button>
                )}
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}

function NameDialog({ count, onClose, onSave }: { count: number; onClose: () => void; onSave: (title: string) => Promise<void> }) {
  const [name, setName] = useState("From the sound map");
  const [busy, setBusy] = useState(false);
  const id = useId();
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => input.current?.select(), []);
  return (
    <Dialog open onClose={onClose} label="Make playlist" className="confirm soundmap-dialog" initialFocus={input}>
      <h2 className="confirm__title">Make a playlist of {songs(count)}</h2>
      <form
        className="soundmap-dialog__form"
        onSubmit={(e) => {
          e.preventDefault();
          if (!name.trim() || busy) return;
          setBusy(true);
          void onSave(name.trim()).finally(() => setBusy(false));
        }}
      >
        <label htmlFor={id} className="soundmap-dialog__label">
          Name
        </label>
        <input id={id} ref={input} className="soundmap-input" value={name} maxLength={100} onChange={(e) => setName(e.target.value)} />
        <div className="confirm__actions">
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="primary" type="submit" disabled={!name.trim() || busy}>
            Make playlist
          </Button>
        </div>
      </form>
    </Dialog>
  );
}
