/**
 * The sound map on the phone: the map as a square the width of the screen
 * (drag to move, pinch to zoom, tap a dot to pick it), the song you picked
 * in a card under it with Play, colour by genre or era, Lasso to circle a
 * patch and play it or make it a playlist, and find a song or an artist.
 * The same points, colours and play order as on a computer.
 */
import { Crosshair, Lasso, Move, Play, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Art } from "../../components/Art";
import { backend } from "../../lib/backend";
import type { SoundMapInfo, SoundMapSong } from "../../lib/types";
import { FindBox, Legend, loadAll, MapLead, useColours, walkOrder } from "../../pages/SoundMapPage";
import { MapCanvas, type MapApi, type MapMode } from "../../pages/soundmap/MapCanvas";
import { emptyData, legend as makeLegend, type ColourBy, type MapData } from "../../pages/soundmap/geometry";
import { player } from "../../stores/player";
import { createPlaylist } from "../../stores/playlists";
import { useSoundMap, useSoundMapOn } from "../../stores/soundmap";
import { BackBar, PageFailed } from "./common";
import { Empty, GhostRows, GoPill, Pills } from "./kit";
import "../../pages/soundmap.css";

const songs = (n: number) => (n === 1 ? "1 song" : `${n.toLocaleString()} songs`);

function Picked({ id, onPlay, onClose }: { id: string; onPlay: () => void; onClose: () => void }) {
  const [song, setSong] = useState<SoundMapSong | null>(null);
  useEffect(() => {
    let alive = true;
    backend.soundmapSong(id).then((s) => alive && setSong(s), () => {});
    return () => {
      alive = false;
    };
  }, [id]);
  if (!song) return null;
  return (
    <div className="ph-map__card" role="region" aria-label={`${song.title} by ${song.artist}`}>
      <Art path={song.thumb} size={48} />
      <span className="ph-row__text">
        <span className="ph-row__title">{song.title}</span>
        <span className="ph-row__sub">{song.artist}</span>
      </span>
      <button type="button" className="ph-play ph-play--row" aria-label={`Play ${song.title}`} onClick={onPlay}>
        <Play size={18} fill="currentColor" aria-hidden="true" />
      </button>
      <button type="button" className="ph-icon-btn" aria-label="Close" onClick={onClose}>
        <X size={20} aria-hidden="true" />
      </button>
    </div>
  );
}

export function PhoneSoundMap({ album }: { album?: string }) {
  const on = useSoundMapOn();
  const [info, setInfo] = useState<SoundMapInfo | null>(null);
  const [data, setData] = useState<MapData | null>(null);
  const [error, setError] = useState(false);
  const [by, setBy] = useState<ColourBy>("genre");
  const [mode, setMode] = useState<MapMode>("move");
  const [picked, setPicked] = useState<number[]>([]);
  const [focus, setFocus] = useState(-1);
  const [root, setRoot] = useState<HTMLElement | null>(null);
  const api = useRef<MapApi | null>(null);
  const refresh = useCallback(async () => {
    try {
      const i = await backend.soundmapInfo();
      setInfo(i);
      useSoundMap.setState({ info: i });
      setData(i.placed > 0 ? await loadAll() : emptyData());
      setError(false);
    } catch {
      setError(true);
    }
  }, []);
  useEffect(() => {
    if (on) void refresh();
  }, [on, refresh]);
  // Opened on a record: its songs picked and in view.
  const aimed = useRef(false);
  useEffect(() => {
    if (!album || !data?.ids.length || aimed.current) return;
    aimed.current = true;
    backend.albumPage(album).then((page) => {
      const ids = page.tracks.map((t) => data.index.get(t.id) ?? -1).filter((i) => i >= 0);
      if (!ids.length) return;
      setPicked(ids);
      api.current?.flyTo(ids.reduce((s, i) => s + data.x[i], 0) / ids.length, ids.reduce((s, i) => s + data.y[i], 0) / ids.length, 6);
    }, () => {});
  }, [album, data]);
  const legend = useMemo(() => makeLegend(data ?? emptyData(), by), [data, by]);
  const { colours, ring, highlight } = useColours(root, by, legend.groups);
  const selected = useMemo(() => {
    if (!data || !picked.length) return null;
    const m = new Uint8Array(data.ids.length);
    for (const i of picked) m[i] = 1;
    return m;
  }, [data, picked]);
  const play = (ids: number[]) => {
    if (!data || !ids.length) return;
    const order = walkOrder(data, ids).map((i) => data.ids[i]);
    void player.playTracks(order, order[0]);
  };
  const placed = info?.placed ?? 0;
  return (
    <div ref={setRoot} className="ph-page ph-x ph-map soundmap" data-phone-page="soundMap">
      {/* Its own frame: the map reads its colours from this root element. */}
      <BackBar title="Sound map" after={72} />
      <h1 className="ph-title ph-title--page ph-x__title">Sound map</h1>
      {placed > 0 && data ? <MapLead className="ph-x__lede" placed={placed} groups={legend.groups} by={by} /> : <p className="ph-x__lede">Songs that sound alike sit together.</p>}
      {!on ? (
        <Empty title="The sound map is off" text="Turn it on in Settings to see your songs placed by how they sound." action={<GoPill route={{ name: "settings", row: "showSoundMap" }}>Open Settings</GoPill>} />
      ) : error && !data ? (
        <PageFailed what="the sound map" retry={() => void refresh()} />
      ) : !data || !info ? (
        <GhostRows n={3} />
      ) : placed === 0 ? (
        <Empty
          title="The map fills in as songs are analysed"
          text={`Automix works out how each song sounds while Platter is idle, and each song takes its place here once it's done. ${info.analysed.toLocaleString()} of ${info.total.toLocaleString()} analysed so far.`}
          action={<GoPill route={{ name: "settings", row: "automix" }}>Automix settings</GoPill>}
        />
      ) : (
        <>
          <div className="ph-map__find">
            <FindBox
              onFound={(f) => {
                if (f.kind === "song") {
                  setPicked([]);
                  setFocus(data.index.get(f.id) ?? -1);
                  api.current?.flyTo(f.x, f.y, 12);
                } else api.current?.flyTo(f.x, f.y, 4);
              }}
              onPlay={(f) => void player.playTracks([f.id], f.id)}
            />
          </div>
          <div className="ph-map__tools">
            <Pills label="Colour by" items={[{ id: "genre", label: "Genre" }, { id: "era", label: "Era" }] as const} value={by} onChange={setBy} />
            <div className="ph-map__modes" role="group" aria-label="Dragging">
              <button type="button" className={`ph-icon-btn ${mode === "move" ? "is-on" : ""}`} aria-pressed={mode === "move"} aria-label="Move" onClick={() => setMode("move")}>
                <Move size={20} aria-hidden="true" />
              </button>
              <button type="button" className={`ph-icon-btn ${mode === "lasso" ? "is-on" : ""}`} aria-pressed={mode === "lasso"} aria-label="Lasso" onClick={() => setMode("lasso")}>
                <Lasso size={20} aria-hidden="true" />
              </button>
              <button type="button" className="ph-icon-btn" aria-label="Show the whole map" onClick={() => api.current?.reset()}>
                <Crosshair size={20} aria-hidden="true" />
              </button>
            </div>
          </div>
          <div className="ph-map__stage soundmap__stage">
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
              label={`Sound map of ${songs(placed)}. Drag to move, pinch to zoom, tap a dot to pick its song.`}
              onHover={() => {}}
              onPick={(i) => {
                setPicked([]);
                setFocus(i);
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
          </div>
          <p className="ph-x__note">{mode === "lasso" ? "Draw round a patch of songs to pick them." : "Switch to Lasso to circle a patch of songs."} Ringed dots are unreleased.</p>
          {focus >= 0 && picked.length === 0 && <Picked id={data.ids[focus]} onPlay={() => play([focus])} onClose={() => setFocus(-1)} />}
          {picked.length > 0 && (
            <div className="ph-map__card" role="region" aria-label="Selection">
              <span className="ph-row__text">
                <span className="ph-row__title">{songs(picked.length)} picked</span>
              </span>
              <button type="button" className="ph-pill is-primary" onClick={() => play(picked)}>
                Play
              </button>
              <button
                type="button"
                className="ph-pill"
                onClick={() => void createPlaylist({ trackIds: walkOrder(data, picked).map((i) => data.ids[i]), title: "From the sound map", open: false }).then((p) => p && setPicked([]))}
              >
                Make playlist
              </button>
              <button type="button" className="ph-icon-btn" aria-label="Clear the selection" onClick={() => setPicked([])}>
                <X size={20} aria-hidden="true" />
              </button>
            </div>
          )}
          <div className="ph-map__legend">
            <Legend groups={legend.groups} colours={colours} by={by} />
          </div>
        </>
      )}
    </div>
  );
}
