/**
 * Deck: the whole app reads as one instrument. A faceplate runs across the
 * top: the cover in a recessed window, a two-line display (the only place
 * with a mono face) over the tonearm line and its engraved plates, transport
 * keys that press in, a volume knob and the output. Under it a rail of
 * labelled source keys with search, then the page at full width. The queue
 * opens as a drawer and lyrics as a page.
 *
 * The controls are the player bar's own (same names, keys and tab stops);
 * the knob is a real slider for keyboard, wheel and screen readers.
 */
import { X } from "lucide-react";
import { useEffect, useRef, useState, type KeyboardEvent, type PointerEvent } from "react";
import { Art } from "../components/Art";
import { ArtistLinks, TextLink } from "../components/ArtistLinks";
import { IconButton } from "../components/Button";
import { ExplicitMark } from "../components/FileTagBadges";
import { openContextMenu } from "../components/ContextMenu";
import { SaveButton } from "../components/SaveButton";
import { Skeleton } from "../components/Skeleton";
import { Tooltip } from "../components/Tooltip";
import { trackMenuItems } from "../components/TrackRow";
import { trackArtistLine } from "../lib/format";
import { endSongDrag, startSongDrag, tracksDrag } from "../stores/drag";
import { fullWindow, useFullWindow } from "../stores/fullWindow";
import { player, usePlayer } from "../stores/player";
import { nav, useCurrentEntry } from "../stores/router";
import { useSettings } from "../stores/settings";
import { useLayout } from "../stores/theme";
import { useUi } from "../stores/ui";
import { useLyricsBackground, LyricsPane } from "./LyricsView";
import { NowPlayingAnnouncer, PlayControls, SeekBar, VOLUME_SLIDER_STEP, wheelVolumeDelta } from "./PlayerBar";
import { VolumeReadout } from "./PlayerExtras";
import { SearchBox } from "./SearchBox";
import { SignalPanel } from "./SignalPath";
import { useSourceKeys } from "./sourceKeys";
import { StripExtras } from "./TransportStrip";
import { Volume1, Volume2, VolumeX } from "lucide-react";
import "./deck.css";

// ---------------------------------------------------------------- the knob

/** The knob turns through 270 degrees, from seven o'clock to five. */
export const KNOB_SWEEP = 270;
/** Pixels of drag (up or right) for the whole range. */
const KNOB_DRAG_PX = 160;

const clamp01 = (v: number) => Math.min(1, Math.max(0, v));
const round2 = (v: number) => Math.round(v * 100) / 100;

/** The level a key gives from `v`, or null for a key the knob doesn't use. */
export function knobKey(key: string, v: number): number | null {
  switch (key) {
    case "ArrowUp":
    case "ArrowRight":
    case "PageUp":
      return round2(clamp01(v + VOLUME_SLIDER_STEP));
    case "ArrowDown":
    case "ArrowLeft":
    case "PageDown":
      return round2(clamp01(v - VOLUME_SLIDER_STEP));
    case "Home":
      return 0;
    case "End":
      return 1;
    default:
      return null;
  }
}

/**
 * Volume as a knob, with the mute key beside it. It is a slider to
 * assistive tech ("Volume", 0 to 100 percent): arrows and Page keys step 10,
 * Home and End go to the ends, the wheel nudges it, and dragging up or right
 * turns it up. It works the same as the player bar's volume.
 */
export function VolumeKnob() {
  const volume = usePlayer((s) => s.state?.volume ?? 0.7);
  const muted = usePlayer((s) => s.state?.muted ?? false);
  const [local, setLocal] = useState<number | null>(null);
  const v = local ?? (muted ? 0 : volume);
  const silent = muted || v === 0;
  const Icon = silent ? VolumeX : v < 0.5 ? Volume1 : Volume2;
  const knob = useRef<HTMLDivElement>(null);
  const drag = useRef<{ x: number; y: number; from: number } | null>(null);

  useEffect(() => {
    const el = knob.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      const delta = wheelVolumeDelta(e);
      if (!delta) return;
      e.preventDefault();
      void player.nudgeVolume(delta);
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, []);

  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    const next = knobKey(e.key, v);
    if (next === null) return;
    e.preventDefault();
    e.stopPropagation();
    void player.volumeInput(next);
    void player.volumeCommit(next);
  };
  const at = (e: PointerEvent<HTMLDivElement>) => {
    const d = drag.current!;
    return round2(clamp01(d.from + (d.y - e.clientY + (e.clientX - d.x)) / KNOB_DRAG_PX));
  };
  const onDown = (e: PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    e.preventDefault();
    knob.current?.focus();
    e.currentTarget.setPointerCapture?.(e.pointerId);
    drag.current = { x: e.clientX, y: e.clientY, from: v };
    setLocal(v);
  };
  const onMove = (e: PointerEvent<HTMLDivElement>) => {
    if (!drag.current) return;
    const next = at(e);
    setLocal(next);
    void player.volumeInput(next);
  };
  const onUp = (e: PointerEvent<HTMLDivElement>) => {
    if (!drag.current) return;
    const next = at(e);
    drag.current = null;
    setLocal(null);
    void player.volumeCommit(next);
  };
  const pct = Math.round(v * 100);
  return (
    <div className="deck-knob">
      <IconButton label={silent ? "Unmute" : "Mute"} tooltip={silent ? "Unmute" : "Mute"} icon={<Icon size={16} />} onClick={() => void player.toggleMute()} />
      <VolumeReadout value={v} silent={silent} dragging={local !== null}>
        <div
          ref={knob}
          className={`deck-knob__dial ${local !== null ? "is-turning" : ""}`}
          role="slider"
          tabIndex={0}
          aria-label="Volume"
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={pct}
          aria-valuetext={`${pct}%`}
          aria-orientation="vertical"
          style={{ "--knob-turn": `${-KNOB_SWEEP / 2 + v * KNOB_SWEEP}deg`, "--knob-level": `${v * KNOB_SWEEP}deg` } as React.CSSProperties}
          onKeyDown={onKey}
          onPointerDown={onDown}
          onPointerMove={onMove}
          onPointerUp={onUp}
          onPointerCancel={onUp}
        >
          <span className="deck-knob__scale" aria-hidden="true" />
          <span className="deck-knob__cap" aria-hidden="true">
            <span className="deck-knob__mark" />
          </span>
        </div>
      </VolumeReadout>
    </div>
  );
}

// ---------------------------------------------------------------- the faceplate

/** The recessed window: the cover, or an empty well. Opens now playing. */
function DeckWindow() {
  const track = usePlayer((s) => s.state?.track);
  const loading = usePlayer((s) => s.state === null);
  const expanded = useFullWindow((s) => s.open);
  const nowPlayingOpen = useUi((s) => s.nowPlayingOpen) && !expanded;
  const songDrags = useSettings((s) => s.settings?.dragAndDrop !== false);
  const dragNow =
    track && songDrags ? { draggable: true, onDragStart: (e: React.DragEvent) => void startSongDrag(e, tracksDrag([track])), onDragEnd: endSongDrag } : null;
  return (
    <div className="deck-window" {...dragNow}>
      {track ? (
        <button
          type="button"
          className="deck-window__art"
          aria-label="Now playing view"
          aria-pressed={nowPlayingOpen}
          onClick={(e) => e.detail < 2 && useUi.getState().toggleNowPlaying()}
          onDoubleClick={() => {
            useUi.getState().toggleNowPlaying();
            fullWindow.open();
          }}
        >
          <Art path={track.thumb} size={80} />
        </button>
      ) : loading ? (
        <Skeleton width={80} height={80} />
      ) : (
        <span className="deck-window__empty" aria-hidden="true" />
      )}
    </div>
  );
}

/** The display: track on the first line, artist and album on the second, then the tonearm line. */
function DeckDisplay() {
  const state = usePlayer((s) => s.state);
  const like = useLayout().playerButtons.includes("like");
  const track = state?.track;
  const nowText = track ? `Now playing: ${track.title} by ${trackArtistLine(track)}` : null;
  return (
    <div className="deck-display" role="region" aria-label={nowText ?? "Nothing playing"}>
      <div className="deck-display__screen">
        {track ? (
          <>
            <div className="deck-display__row">
              <div
                className="deck-display__line deck-display__line--title truncate"
                onContextMenu={(e) => {
                  e.preventDefault();
                  openContextMenu(`Options for ${track.title}`, trackMenuItems(track), e);
                }}
              >
                <TextLink onActivate={() => nav.go({ name: "album", id: track.albumId })}>{track.title}</TextLink>
                <ExplicitMark trackId={track.id} />
              </div>
              {like && <SaveButton track={track} place="bar" />}
            </div>
            <div className="deck-display__line deck-display__line--sub truncate">
              <ArtistLinks track={track} className="deck-display__artist" />
              {track.albumTitle && (
                <>
                  <span className="deck-display__sep" aria-hidden="true">
                    /
                  </span>
                  <TextLink className="deck-display__album" onActivate={() => nav.go({ name: "album", id: track.albumId })}>
                    {track.albumTitle}
                  </TextLink>
                </>
              )}
            </div>
          </>
        ) : state === null ? (
          <>
            <Skeleton width="60%" height={14} />
            <Skeleton width="40%" height={12} style={{ marginTop: 8 }} />
          </>
        ) : (
          <>
            <div className="deck-display__line deck-display__line--title deck-display__line--idle">No disc</div>
            <div className="deck-display__line deck-display__line--sub deck-display__line--idle">Pick a record to play</div>
          </>
        )}
      </div>
      <div className="deck-display__line-seek">
        <SeekBar strip />
      </div>
    </div>
  );
}

/** The faceplate across the top. Its label and controls match the player bar's. */
export function DeckFaceplate() {
  const track = usePlayer((s) => s.state?.track);
  const volume = useLayout().playerButtons.includes("volume");
  const nowText = track ? `Now playing: ${track.title} by ${trackArtistLine(track)}` : null;
  return (
    <aside className="deck-face" aria-label="Player" data-testid="deck-faceplate">
      <NowPlayingAnnouncer text={nowText} />
      <div className="deck-face__body">
        <DeckWindow />
        <DeckDisplay />
        <div className="deck-keys">
          <PlayControls />
        </div>
        <div className="deck-face__end">
          {volume && <VolumeKnob />}
          <StripExtras skip={["volume"]} />
        </div>
      </div>
      <SignalPanel />
    </aside>
  );
}

// ---------------------------------------------------------------- the source rail

/** Labelled source keys, then search. Labels can be turned off (the keys keep their names). */
export function DeckRail() {
  const current = useCurrentEntry().route;
  const labels = useLayout().deckLabels !== false;
  const keys = useSourceKeys({ search: true });
  return (
    <nav className={`deck-rail ${labels ? "is-labelled" : "is-bare"}`} aria-label="Main">
      <ul className="deck-rail__keys">
        {keys.map((k) => {
          const on = k.on(current);
          const key = (
            <button
              type="button"
              className={`deck-key ${on ? "is-on" : ""}`}
              aria-current={on ? "page" : undefined}
              aria-label={labels ? undefined : k.label}
              onClick={() => nav.go(k.route)}
            >
              <span className="deck-key__lamp" aria-hidden="true" />
              <span className="deck-key__icon" aria-hidden="true">
                {k.icon}
              </span>
              {labels && <span className="deck-key__label">{k.label}</span>}
            </button>
          );
          return <li key={k.key}>{labels ? key : <Tooltip label={k.label}>{key}</Tooltip>}</li>;
        })}
      </ul>
      <div className="deck-rail__search">
        <SearchBox />
      </div>
    </nav>
  );
}

// ---------------------------------------------------------------- lyrics as a page

/** Deck shows lyrics as a page under the faceplate, not over the window. */
export function DeckLyricsPage() {
  const track = usePlayer((s) => s.state?.track);
  const tint = useLyricsBackground(track?.thumb);
  return (
    <main id="main" className="main deck-lyrics" aria-label="Lyrics" style={{ "--fw-tint": tint } as React.CSSProperties}>
      <header className="deck-lyrics__head">
        <h1 className="deck-lyrics__title">Lyrics</h1>
        {track && (
          <span className="deck-lyrics__song truncate">
            {track.title} · {trackArtistLine(track)}
          </span>
        )}
        <IconButton label="Close lyrics" tooltip="Close lyrics" icon={<X size={18} />} onClick={() => void fullWindow.close()} />
      </header>
      <div className="fw__lyrics deck-lyrics__body">
        <LyricsPane />
      </div>
    </main>
  );
}
