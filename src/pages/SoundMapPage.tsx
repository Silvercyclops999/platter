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
import { useSettings } from "../stores/settings";
import { useSoundMap, useSoundMapOn } from "../stores/soundmap";
import { MapCanvas, type MapApi, type MapMode } from "./soundmap/MapCanvas";
import { emptyData, joinChunks, legend as makeLegend, type ColourBy, type MapData } from "./soundmap/geometry";
import "./soundmap.css";

/** Points per request. */
export const CHUNK = 20_000;
/** How long a dot is hovered before its card shows. */
const CARD_DELAY_MS = 120;
/** Selections up to this size play in a walk from one song to the most alike next. */
const WALK_MAX = 1500;

const fmt = (n: number) => n.toLocaleString();
const songs = (n: number) => (n === 1 ? "1 song" : `${fmt(n)} songs`);

/** Opens Settings at one row. */
export function openSetting(key: string) {
  nav.go({ name: "settings" });
  let tries = 0;
  const look = () => {
    const row = document.querySelector<HTMLElement>(`[data-setting="${key}"]`);
    if (row) {
      row.scrollIntoView?.({ block: "center" });
      row.querySelector<HTMLElement>("button, [role=switch], input")?.focus({ preventScroll: true });
    } else if (tries++ < 60) requestAnimationFrame(look);
  };
  requestAnimationFrame(look);
}

/** The whole map, chunk by chunk. */
async function loadAll(): Promise<MapData> {
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
function useColours(el: HTMLElement | null, by: ColourBy, groups: { slot: number }[]) {
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

export function SoundMapPage() {
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
  const api = useRef<MapApi | null>(null);

  const refresh = useCallback(async () => {
    try {
      const i = await backend.soundmapInfo();
      setInfo(i);
      // The app menu's entry follows what the page knows.
      useSoundMap.setState({ info: i });
      if (i.placed > 0) setData(await loadAll());
      else setData(emptyData());
      setError(null);
    } catch (e) {
      setError(e && typeof e === "object" && "message" in e ? String((e as { message: unknown }).message) : "Couldn't load the map.");
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
      void a.then((f) => f());
      void b.then((f) => f());
    };
  }, [on, refresh]);

  // A new set of points drops a selection made on the old one.
  useEffect(() => {
    setPicked([]);
    setFocus(-1);
  }, [data]);

  const legend = useMemo(() => makeLegend(data ?? emptyData(), by), [data, by]);
  const { colours, ring, highlight } = useColours(root, by, legend.groups);
  const selected = useMemo(() => {
    if (!data || picked.length === 0) return null;
    const m = new Uint8Array(data.ids.length);
    for (const i of picked) m[i] = 1;
    return m;
  }, [data, picked]);

  const hover = useHoverCard(data);

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
  const partial = info && data && placed > 0 && info.analysing && info.analysed < info.total ? info : null;

  const play = (ids: number[]) => {
    if (!data || ids.length === 0) return;
    const order = walkOrder(data, ids).map((i) => data.ids[i]);
    void player.playTracks(order, order[0]);
  };

  return (
    <div className="page-pad soundmap" data-testid="soundmap-page" ref={setRoot}>
      <header className="soundmap__head">
        <div className="soundmap__titles">
          <h1 className="page-title">Sound map</h1>
          <p className="soundmap__sub">
            Songs that sound alike sit together.
            {ready && placed > 0 && <> {songs(placed)} on the map.</>}
          </p>
        </div>
        {ready && placed > 0 && (
          <div className="soundmap__tools">
            <FindBox
              onFound={(f) => {
                if (!data) return;
                if (f.kind === "song") {
                  const i = data.index.get(f.id) ?? -1;
                  setFocus(i);
                  api.current?.flyTo(f.x, f.y, 12);
                } else {
                  setFocus(-1);
                  api.current?.flyTo(f.x, f.y, 4);
                }
              }}
            />
            <div className="soundmap__seg" role="radiogroup" aria-label="Colour by">
              {(["genre", "era"] as const).map((k) => (
                <button key={k} type="button" role="radio" aria-checked={by === k} className="soundmap__seg-btn" onClick={() => setBy(k)}>
                  {k === "genre" ? "Genre" : "Era"}
                </button>
              ))}
            </div>
            <div className="soundmap__seg" role="radiogroup" aria-label="Dragging">
              <button type="button" role="radio" aria-checked={mode === "move"} className="soundmap__seg-btn" onClick={() => setMode("move")}>
                <Move size={14} aria-hidden="true" /> Move
              </button>
              <button type="button" role="radio" aria-checked={mode === "lasso"} className="soundmap__seg-btn" onClick={() => setMode("lasso")}>
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

      {error && !data && (
        <p className="soundmap__note" role="alert">
          {error}
        </p>
      )}
      {!ready && !error && <SkeletonLines lines={3} widths={["60%", "80%", "40%"]} gap={20} />}

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
              label={`Sound map of ${songs(placed)}. Drag to move, scroll to zoom, arrow keys and plus or minus with the map focused.`}
              onHover={hover.onHover}
              onPick={(i) => {
                setFocus(i);
                void player.playTracks([data.ids[i]], data.ids[i]);
              }}
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
              <div className="soundmap__card" style={{ left: hover.card.sx, top: hover.card.sy }} aria-hidden="true" data-testid="soundmap-card">
                <Art path={hover.card.song.thumb} size={40} />
                <div className="soundmap__card-text">
                  <span className="soundmap__card-title">{hover.card.song.title}</span>
                  <span className="soundmap__card-sub">{hover.card.song.artist}</span>
                  {hover.card.unreleased && <span className="soundmap__card-tag">Unreleased</span>}
                </div>
              </div>
            )}
            {focus >= 0 && picked.length === 0 && <FocusCard id={data.ids[focus]} onPlay={() => play([focus])} onClose={() => setFocus(-1)} />}
            {picked.length > 0 && (
              <div className="soundmap__selection" role="region" aria-label="Selection">
                <span className="soundmap__selection-count" aria-live="polite">
                  {songs(picked.length)} selected
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

function Legend({ groups, colours, by }: { groups: { label: string; count: number }[]; colours: string[]; by: ColourBy }) {
  return (
    <ul className="soundmap__legend" aria-label={by === "genre" ? "Genres" : "Eras"}>
      {groups.map((g, i) =>
        g.count > 0 ? (
          <li key={g.label} className="soundmap__legend-item">
            <span className="soundmap__swatch" style={{ background: colours[i] }} aria-hidden="true" />
            <span>{g.label}</span>
            <span className="soundmap__legend-count">{fmt(g.count)}</span>
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

/** The hover card: shows after a short hover, follows the dot. */
function useHoverCard(data: MapData | null) {
  const [card, setCard] = useState<{ song: SoundMapSong; sx: number; sy: number; unreleased: boolean } | null>(null);
  const at = useRef(-1);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => () => void (timer.current && clearTimeout(timer.current)), []);
  const onHover = useCallback(
    (i: number, sx: number, sy: number) => {
      if (i === at.current) {
        if (i >= 0) setCard((c) => (c ? { ...c, sx, sy } : c));
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
          if (song && at.current === i) setCard({ song, sx, sy, unreleased });
        });
      }, CARD_DELAY_MS);
    },
    [data],
  );
  return { card, onHover };
}

/** A song found by search or clicked: its card with Play. */
function FocusCard({ id, onPlay, onClose }: { id: string; onPlay: () => void; onClose: () => void }) {
  const [song, setSong] = useState<SoundMapSong | null>(null);
  useEffect(() => {
    let alive = true;
    void songOf(id).then((s) => alive && setSong(s));
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

/** Search to fly to a song or an artist. */
function FindBox({ onFound }: { onFound: (f: SoundMapFound) => void }) {
  const [q, setQ] = useState("");
  const [found, setFound] = useState<SoundMapFound[]>([]);
  const [open, setOpen] = useState(false);
  const listId = useId();
  useEffect(() => {
    const query = q.trim();
    if (!query) {
      setFound([]);
      return;
    }
    let alive = true;
    const t = setTimeout(() => void backend.soundmapFind(query).then((f) => alive && setFound(f), () => {}), 150);
    return () => {
      alive = false;
      clearTimeout(t);
    };
  }, [q]);
  const pick = (f: SoundMapFound) => {
    onFound(f);
    setOpen(false);
  };
  return (
    <div className="soundmap__find" onBlur={(e) => !e.currentTarget.contains(e.relatedTarget as Node) && setOpen(false)}>
      <SearchField
        value={q}
        onChange={(v) => {
          setQ(v);
          setOpen(true);
        }}
        onSubmit={() => found[0] && pick(found[0])}
        label="Find a song or artist on the map"
        placeholder="Find a song or artist"
        inputProps={{ "aria-controls": listId, "aria-expanded": open && found.length > 0 }}
      />
      {open && q.trim() !== "" && (
        <ul id={listId} className="soundmap__results" aria-label="On the map">
          {found.length === 0 && <li className="soundmap__results-none">Not on the map yet</li>}
          {found.map((f) => (
            <li key={`${f.kind}:${f.id}`}>
              <button type="button" className="soundmap__result" onClick={() => pick(f)}>
                <span className="soundmap__result-title">{f.title}</span>
                <span className="soundmap__result-sub">
                  {f.kind === "artist" ? "Artist" : "Song"} · {f.sub}
                </span>
              </button>
            </li>
          ))}
        </ul>
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
