/**
 * A small working picture of the app shell for a look: sidebar, Home or an
 * album page, the now playing panel and the player bar, drawn with the
 * look's tokens, style and layout on its own root (so nothing leaks into
 * the real app). Everything in it is made-up sample music, never the
 * listener's library, so a screenshot of it is safe to share.
 *
 * With `onReorder`, sidebar sections, Home shelves, player buttons and
 * track columns can be dragged into a new order right in the picture.
 */
import { Clock3, Heart, ListMusic, Maximize2, Mic2, MonitorSpeaker, PictureInPicture2, Play, Repeat, Search, Shuffle, SkipBack, SkipForward, Volume2 } from "lucide-react";
import { useEffect, useRef, useState, type CSSProperties, type DragEvent, type ReactNode } from "react";
import { lookProps, type LookParts } from "../../design/looks/apply";
import type { HomeShelfKey, PlayerButton, SidebarSection, ThemeBase, TrackColumn } from "../../design/themes";
import "./look-preview.css";

export type ReorderList = "sidebarSections" | "homeShelves" | "playerButtons" | "trackColumns";

export interface LookPreviewProps {
  base: ThemeBase;
  tokens: Record<string, string>;
  parts: LookParts;
  page?: "home" | "album";
  /** Drag-to-arrange: move `id` to where `before` is (or the end when null). */
  onReorder?: (list: ReorderList, id: string, before: string | null) => void;
  label?: string;
}

/** Made-up sample music for the picture. */
const ALBUMS = [
  { title: "Low Tide Hours", artist: "Mara Vale", hue: 18 },
  { title: "Paper Lanterns", artist: "The Quiet Rooms", hue: 205 },
  { title: "Northern Static", artist: "Ilse Brandt", hue: 160 },
  { title: "Glasshouse", artist: "Otto & the Pines", hue: 280 },
  { title: "Small Weather", artist: "June Halloway", hue: 42 },
  { title: "Copper Sun", artist: "Fen Marsh", hue: 340 },
  { title: "Slow Signals", artist: "Rue Calder", hue: 120 },
  { title: "Harbour Lights", artist: "Tamsin Rowe", hue: 230 },
];
const TRACKS = ["Morning Ferry", "Salt in the Air", "Everything Slow", "Lantern Song", "The Long Way Back"];

const SHELF_TITLES: Record<HomeShelfKey, string> = {
  madeForYou: "Made for you",
  recentlyPlayed: "Recently played",
  mixMoods: "Your mixes by mood",
  mixGenres: "Your genre mixes",
  jumpBackIn: "Jump back in",
  playlists: "Your playlists",
  recentlyAdded: "Recently added",
  topArtists: "Your top artists",
};

function Cover({ hue, round = false, className = "" }: { hue: number; round?: boolean; className?: string }) {
  return (
    <span
      className={`lp-art ${round ? "lp-art--round" : ""} ${className}`}
      style={{ "--lp-hue": hue } as CSSProperties}
      aria-hidden="true"
    />
  );
}

/** Drag handlers for one item of a reorderable list. */
function useDrag(onReorder: LookPreviewProps["onReorder"]) {
  const [dragging, setDragging] = useState<{ list: ReorderList; id: string } | null>(null);
  const [over, setOver] = useState<string | null>(null);
  return (list: ReorderList, id: string) => {
    if (!onReorder) return {};
    return {
      draggable: true,
      "data-drag-id": id,
      className: `lp-draggable ${dragging?.id === id ? "is-dragging" : ""} ${over === `${list}:${id}` && dragging?.list === list ? "is-over" : ""}`,
      onDragStart: (e: DragEvent) => {
        e.stopPropagation();
        e.dataTransfer.effectAllowed = "move";
        e.dataTransfer.setData("text/plain", id);
        setDragging({ list, id });
      },
      onDragOver: (e: DragEvent) => {
        if (dragging?.list !== list) return;
        e.preventDefault();
        e.stopPropagation();
        setOver(`${list}:${id}`);
      },
      onDrop: (e: DragEvent) => {
        if (dragging?.list !== list) return;
        e.preventDefault();
        e.stopPropagation();
        if (dragging.id !== id) onReorder(list, dragging.id, id);
        setDragging(null);
        setOver(null);
      },
      onDragEnd: () => {
        setDragging(null);
        setOver(null);
      },
    };
  };
}

function Sidebar({ sections, rail, drag }: { sections: SidebarSection[]; rail: boolean; drag: ReturnType<typeof useDrag> }) {
  const part = (id: SidebarSection): ReactNode => {
    const d = drag("sidebarSections", id);
    switch (id) {
      case "servers":
        return (
          <div key={id} {...d} className={`lp-side__servers ${d.className ?? ""}`}>
            <span className="lp-dot" /> Home server
          </div>
        );
      case "filters":
        return (
          <div key={id} {...d} className={`lp-side__chips ${d.className ?? ""}`}>
            <span className="lp-chip is-on">Playlists</span>
            <span className="lp-chip">Albums</span>
            <span className="lp-chip">Artists</span>
          </div>
        );
      case "tools":
        return (
          <div key={id} {...d} className={`lp-side__tools ${d.className ?? ""}`}>
            <Search size={12} aria-hidden="true" />
            <span className="lp-side__sort">Recents</span>
          </div>
        );
      default:
        return null;
    }
  };
  return (
    <div className={`lp-side ${rail ? "lp-side--rail" : ""}`}>
      <div className="lp-side__head">
        <ListMusic size={14} aria-hidden="true" /> Your Library
      </div>
      {!rail && sections.filter((s) => s !== "status").map(part)}
      <ul className="lp-side__list">
        {ALBUMS.slice(0, 6).map((a, i) => (
          <li key={a.title} className="lp-side__row">
            <Cover hue={a.hue} round={i === 3} />
            <span className="lp-side__text">
              <span className="lp-line lp-line--title">{a.title}</span>
              <span className="lp-line lp-line--sub">{i === 3 ? "Artist" : `Album · ${a.artist}`}</span>
            </span>
          </li>
        ))}
      </ul>
      {sections.includes("status") && (
        <div {...drag("sidebarSections", "status")} className={`lp-side__status ${drag("sidebarSections", "status").className ?? ""}`}>
          <span className="lp-dot" /> Connected
        </div>
      )}
    </div>
  );
}

function Home({ parts, drag }: { parts: LookParts; drag: ReturnType<typeof useDrag> }) {
  const { layout } = parts;
  return (
    <div className="lp-page">
      {layout.homeGreeting && <div className="lp-greeting">Good evening</div>}
      <div className="lp-chips">
        <span className="lp-chip is-on">All</span>
        <span className="lp-chip">Music</span>
        <span className="lp-chip">Playlists</span>
      </div>
      <div className="lp-grid" style={{ "--lp-cols": layout.quickAccess === 6 ? 3 : 4 } as CSSProperties}>
        {ALBUMS.slice(0, layout.quickAccess).map((a) => (
          <span key={a.title} className="lp-tile">
            <Cover hue={a.hue} />
            <span className="lp-line lp-line--title">{a.title}</span>
          </span>
        ))}
      </div>
      {layout.homeShelves.slice(0, 3).map((key, n) => (
        <div key={key} {...drag("homeShelves", key)} className={`lp-shelf ${drag("homeShelves", key).className ?? ""}`}>
          <div className="lp-shelf__title">{SHELF_TITLES[key]}</div>
          <div className="lp-shelf__row">
            {ALBUMS.slice(n, n + 5).map((a) => (
              <span key={a.title} className="lp-card">
                <Cover hue={a.hue + n * 30} round={key === "topArtists"} />
                <span className="lp-line lp-line--title">{a.title}</span>
                <span className="lp-line lp-line--sub">{a.artist}</span>
              </span>
            ))}
          </div>
        </div>
      ))}
      {layout.homeShelves.length > 3 && <p className="lp-more">and {layout.homeShelves.length - 3} more shelves</p>}
    </div>
  );
}

const COLUMN_LABEL: Record<TrackColumn, string> = { album: "Album", info: "Quality", duration: "Time" };

function Album({ parts, drag }: { parts: LookParts; drag: ReturnType<typeof useDrag> }) {
  const cols = parts.layout.trackColumns;
  const a = ALBUMS[0];
  const cell = (c: TrackColumn, i: number) =>
    c === "album" ? (
      <span key={c} className="lp-col lp-col--album">
        {a.title}
      </span>
    ) : c === "info" ? (
      <span key={c} className="lp-col lp-col--info">
        <span className="lp-badge">FLAC</span>
      </span>
    ) : (
      <span key={c} className="lp-col lp-col--duration">
        {3 + (i % 3)}:{String((i * 17) % 60).padStart(2, "0")}
      </span>
    );
  return (
    <div className="lp-page lp-page--album">
      <div className="lp-header" style={{ "--lp-hue": a.hue } as CSSProperties}>
        <Cover hue={a.hue} className="lp-header__art" />
        <div className="lp-header__text">
          <span className="lp-line lp-line--sub">Album</span>
          <div className="lp-header__title">{a.title}</div>
          <span className="lp-line lp-line--sub">
            {a.artist} · 2024 · 9 songs
          </span>
        </div>
      </div>
      <div className="lp-actions">
        <span className="lp-play">
          <Play size={16} fill="currentColor" aria-hidden="true" />
        </span>
        <Shuffle size={16} aria-hidden="true" />
      </div>
      <div className="lp-tracks">
        <div className="lp-track lp-track--head">
          <span className="lp-col lp-col--num">#</span>
          <span className="lp-col lp-col--title">Title</span>
          {cols.map((c) => (
            <span key={c} {...drag("trackColumns", c)} className={`lp-col lp-col--${c} lp-col--head ${drag("trackColumns", c).className ?? ""}`}>
              {c === "duration" ? <Clock3 size={12} aria-label="Time" /> : COLUMN_LABEL[c]}
            </span>
          ))}
        </div>
        {TRACKS.map((t, i) => (
          <div key={t} className={`lp-track ${i === 1 ? "is-playing" : ""}`}>
            <span className="lp-col lp-col--num">{i + 1}</span>
            <span className="lp-col lp-col--title">
              <span className="lp-line lp-line--title">{t}</span>
              <span className="lp-line lp-line--sub">{a.artist}</span>
            </span>
            {cols.map((c) => cell(c, i))}
          </div>
        ))}
      </div>
    </div>
  );
}

const BUTTON_ICON: Record<PlayerButton, ReactNode> = {
  like: <Heart size={14} aria-hidden="true" />,
  lyrics: <Mic2 size={14} aria-hidden="true" />,
  queue: <ListMusic size={14} aria-hidden="true" />,
  devices: <MonitorSpeaker size={14} aria-hidden="true" />,
  volume: (
    <>
      <Volume2 size={14} aria-hidden="true" />
      <span className="lp-volume" />
    </>
  ),
  miniplayer: <PictureInPicture2 size={14} aria-hidden="true" />,
  fullWindow: <Maximize2 size={14} aria-hidden="true" />,
};

function PlayerBar({ parts, drag }: { parts: LookParts; drag: ReturnType<typeof useDrag> }) {
  const buttons = parts.layout.playerButtons;
  const a = ALBUMS[0];
  return (
    <div className="lp-player">
      <div className="lp-player__now">
        <Cover hue={a.hue} className="lp-player__art" />
        <span className="lp-player__meta">
          <span className="lp-line lp-line--title">{TRACKS[1]}</span>
          <span className="lp-line lp-line--sub">{a.artist}</span>
        </span>
        {buttons.includes("like") && (
          <span {...drag("playerButtons", "like")} className={`lp-btn lp-btn--like ${drag("playerButtons", "like").className ?? ""}`}>
            {BUTTON_ICON.like}
          </span>
        )}
      </div>
      <div className="lp-player__center">
        <div className="lp-player__controls">
          <Shuffle size={13} aria-hidden="true" />
          <SkipBack size={14} aria-hidden="true" />
          <span className="lp-play lp-play--small">
            <Play size={12} fill="currentColor" aria-hidden="true" />
          </span>
          <SkipForward size={14} aria-hidden="true" />
          <Repeat size={13} aria-hidden="true" />
        </div>
        <div className="lp-seek">
          <span className="lp-seek__fill" />
        </div>
      </div>
      <div className="lp-player__extras">
        {buttons
          .filter((b) => b !== "like")
          .map((b) => (
            <span key={b} {...drag("playerButtons", b)} className={`lp-btn lp-btn--${b} ${drag("playerButtons", b).className ?? ""}`} title={b}>
              {BUTTON_ICON[b]}
            </span>
          ))}
      </div>
    </div>
  );
}

function NowPlaying() {
  const a = ALBUMS[0];
  return (
    <div className="lp-np">
      <div className="lp-np__head">{a.title}</div>
      <span className="lp-np__art is-spinning">
        <Cover hue={a.hue} />
      </span>
      <span className="lp-line lp-line--big">{TRACKS[1]}</span>
      <span className="lp-line lp-line--sub">{a.artist}</span>
      <div className="lp-np__card">
        <span className="lp-line lp-line--title">About the artist</span>
        <span className="lp-line lp-line--sub">Songs about harbours and slow mornings.</span>
      </div>
    </div>
  );
}

/** The picture itself. Its size comes from its parent (it scales to fit). */
export function LookPreview({ base, tokens, parts, page = "home", onReorder, label = "Preview" }: LookPreviewProps) {
  const drag = useDrag(onReorder);
  const { attrs, style } = lookProps(parts);
  const vars: Record<string, string> = { ...style };
  for (const [k, v] of Object.entries(tokens)) vars[`--${k}`] = v;
  const { layout } = parts;
  return (
    <div className="lp" role={onReorder ? "group" : "img"} aria-label={label} data-theme-base={base} {...attrs} style={vars as CSSProperties}>
      <div className="lp-top">
        <span className="lp-top__dots" />
        <span className="lp-top__search">
          <Search size={12} aria-hidden="true" /> What do you want to play?
        </span>
      </div>
      <div className="lp-body">
        <Sidebar sections={layout.sidebarSections} rail={layout.sidebarCollapsed} drag={drag} />
        <div className="lp-main">{page === "album" ? <Album parts={parts} drag={drag} /> : <Home parts={parts} drag={drag} />}</div>
        {layout.nowPlaying !== "off" && <NowPlaying />}
      </div>
      <PlayerBar parts={parts} drag={drag} />
    </div>
  );
}

/** The picture scaled to fill its frame's width (16:10). */
export function PreviewFrame(props: LookPreviewProps) {
  const [scale, setScale] = useState(0.25);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const fit = () => setScale(el.clientWidth / 960 || 0.25);
    fit();
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(fit);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  return (
    <div ref={ref} className="lp-frame">
      <div className="lp-frame__inner" style={{ transform: `scale(${scale})` }}>
        <LookPreview {...props} />
      </div>
    </div>
  );
}
