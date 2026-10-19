/**
 * Turntable: the player is a fixed column on the left, a record player on a
 * shelf. The platter carries the cover as its label and the tonearm lowers
 * on play; speed and format plates sit under it, then the song, the
 * tonearm line, the transport and a short stack of the sleeves up next.
 * Browsing on the right has its own section tabs and search. In a narrow
 * window the column shrinks to a small platter with the transport.
 *
 * The controls are the player bar's own, so names, keys and tab stops
 * don't change. Reduced motion turns the spin and the arm into fades
 * (Turntable.tsx).
 */
import { useMemo } from "react";
import { Art } from "../components/Art";
import { ArtistLinks, TextLink } from "../components/ArtistLinks";
import { ExplicitMark } from "../components/FileTagBadges";
import { openContextMenu } from "../components/ContextMenu";
import { Plate } from "../components/Plate";
import { SaveButton } from "../components/SaveButton";
import { Skeleton } from "../components/Skeleton";
import { trackMenuItems } from "../components/TrackRow";
import { trackArtistLine } from "../lib/format";
import { chrome } from "../lib/platform";
import type { QueueItem, QueueView } from "../lib/types";
import { endSongDrag, startSongDrag, tracksDrag } from "../stores/drag";
import { fullWindow, useFullWindow } from "../stores/fullWindow";
import { usePlayer } from "../stores/player";
import { nav, useCurrentEntry } from "../stores/router";
import { useSettings } from "../stores/settings";
import { useLayout } from "../stores/theme";
import { useUi } from "../stores/ui";
import { HifiMark } from "./HifiMark";
import { Logo } from "./Logo";
import { NowPlayingAnnouncer, PlayControls, SeekBar } from "./PlayerBar";
import { SignalBadge, SignalPanel } from "./SignalPath";
import { useSourceKeys } from "./sourceKeys";
import { StripExtras } from "./TransportStrip";
import { Turntable } from "./Turntable";
import "./turntable-frame.css";

/** How many sleeves the up-next stack shows. */
export const SLEEVES_SHOWN = 3;

/** What plays next: your queue first, then the rest of what's playing. */
export function upNextItems(q: QueueView | null | undefined, n = SLEEVES_SHOWN): QueueItem[] {
  if (!q) return [];
  return [...q.userQueue, ...q.nextUp].slice(0, n);
}

/** One sleeve in the up-next stack: a run of songs from the same record. */
export interface UpNextSleeve {
  item: QueueItem;
  /** Songs in a row from this record (1 or more). */
  count: number;
}

/** How far ahead the stack looks for different records. */
const SLEEVE_LOOKAHEAD = 40;

/**
 * The up-next stack as sleeves: songs in a row from one record share a
 * sleeve with a count, so the stack shows different records, not the same
 * cover three times.
 */
export function upNextSleeves(q: QueueView | null | undefined, n = SLEEVES_SHOWN): UpNextSleeve[] {
  const out: UpNextSleeve[] = [];
  for (const item of upNextItems(q, SLEEVE_LOOKAHEAD)) {
    const last = out[out.length - 1];
    if (last && last.item.track.albumId === item.track.albumId) last.count += 1;
    else if (out.length < n) out.push({ item, count: 1 });
    else break;
  }
  return out;
}

/** The record on its platter, with speed and format plates on the plinth. */
function Platter({ compact }: { compact: boolean }) {
  const track = usePlayer((s) => s.state?.track);
  const playing = usePlayer((s) => s.state?.status === "playing");
  const expanded = useFullWindow((s) => s.open);
  const nowPlayingOpen = useUi((s) => s.nowPlayingOpen) && !expanded;
  // The record player setting off: the record holds still (the arm still follows the music).
  const spin = useSettings((s) => (s.settings?.recordMotion ?? "off") !== "off");
  return (
    <div className="tt-plinth">
      <div className="tt-plinth__deck">
        <Turntable
          thumb={track?.thumb}
          sleeve={track ? { title: track.albumTitle, artist: track.artistTitle } : null}
          track={track}
          playing={!!track && playing}
          spin={spin}
          requestSize={compact ? 240 : 480}
        />
        {track && (
          <button
            type="button"
            className="tt-plinth__open"
            aria-label="Now playing view"
            aria-pressed={nowPlayingOpen}
            onClick={(e) => e.detail < 2 && useUi.getState().toggleNowPlaying()}
            onDoubleClick={() => {
              useUi.getState().toggleNowPlaying();
              fullWindow.open();
            }}
          />
        )}
      </div>
      <div className="tt-plates">
        {/* The motor lamp is lit while the record turns. */}
        <Plate lamp lampOn={!!track && playing} className={`tt-plate--speed ${track && playing ? "is-on" : ""}`} title="Record speed">
          33⅓
        </Plate>
        <SignalBadge />
        <HifiMark />
      </div>
    </div>
  );
}

/** Title, artist and the like key, under the platter. */
function NowLines() {
  const state = usePlayer((s) => s.state);
  const like = useLayout().playerButtons.includes("like");
  const songDrags = useSettings((s) => s.settings?.dragAndDrop !== false);
  const track = state?.track;
  const nowText = track ? `Now playing: ${track.title} by ${trackArtistLine(track)}` : null;
  const dragNow =
    track && songDrags ? { draggable: true, onDragStart: (e: React.DragEvent) => void startSongDrag(e, tracksDrag([track])), onDragEnd: endSongDrag } : null;
  return (
    <div className="tt-now" role="region" aria-label={nowText ?? "Nothing playing"} {...dragNow}>
      {track ? (
        <>
          <div className="tt-now__text">
            <div
              className="tt-now__title truncate"
              onContextMenu={(e) => {
                e.preventDefault();
                openContextMenu(`Options for ${track.title}`, trackMenuItems(track), e);
              }}
            >
              <TextLink onActivate={() => nav.go({ name: "album", id: track.albumId })}>{track.title}</TextLink>
              <ExplicitMark trackId={track.id} />
            </div>
            <ArtistLinks track={track} className="tt-now__artist truncate" />
          </div>
          {like && <SaveButton track={track} place="bar" />}
        </>
      ) : state === null ? (
        <div className="tt-now__text">
          <Skeleton width="70%" height={15} />
          <Skeleton width="45%" height={12} style={{ marginTop: 6 }} />
        </div>
      ) : (
        <p className="tt-now__empty">Nothing on the platter. Pick a record.</p>
      )}
    </div>
  );
}

/** The next few songs as a short stack of sleeves. Opens the queue. */
function UpNext() {
  const queue = usePlayer((s) => s.state?.queue);
  const sleeves = useMemo(() => upNextSleeves(queue), [queue]);
  const queueOpen = useUi((s) => s.queueOpen);
  if (!sleeves.length) return <p className="tt-next__empty">Nothing up next. Add songs to the queue and they stack here.</p>;
  const first = sleeves[0].item.track;
  const more = sleeves[0].count - 1;
  return (
    <button
      type="button"
      className="tt-next"
      aria-pressed={queueOpen}
      aria-label={`Up next: ${first.title} by ${trackArtistLine(first)}${more > 0 ? `, then ${more} more from ${first.albumTitle}` : ""}. Open the queue`}
      onClick={() => useUi.getState().setQueueOpen(!queueOpen)}
    >
      <span className="tt-next__stack" aria-hidden="true">
        {sleeves
          .slice()
          .reverse()
          .map((sl, i, all) => (
            <span key={sl.item.itemId} className="tt-next__sleeve" style={{ "--i": all.length - 1 - i } as React.CSSProperties}>
              <Art path={sl.item.track.thumb} size={56} sleeve={{ title: sl.item.track.albumTitle, artist: sl.item.track.artistTitle }} />
              {sl.count > 1 && (
                <span className="plate plate--sm tt-next__count" data-testid="tt-next-count">
                  {sl.count}
                </span>
              )}
            </span>
          ))}
      </span>
      <span className="tt-next__text" aria-hidden="true">
        <span className="tt-next__label">Up next</span>
        <span className="tt-next__title truncate">{first.title}</span>
        <span className="tt-next__artist truncate">{trackArtistLine(first)}</span>
      </span>
    </button>
  );
}

/** The player column. Its label and controls match the player bar's. */
export function TurntableColumn({ compact }: { compact: boolean }) {
  const track = usePlayer((s) => s.state?.track);
  const nowText = track ? `Now playing: ${track.title} by ${trackArtistLine(track)}` : null;
  return (
    <aside className={`tt-column ${compact ? "is-compact" : ""}`} aria-label="Player" data-testid="turntable-column">
      <NowPlayingAnnouncer text={nowText} />
      <div className="tt-column__head" data-tauri-drag-region>
        {chrome !== "mac" && (
          <span className="tt-column__logo" data-tauri-drag-region>
            <Logo size={22} />
          </span>
        )}
      </div>
      <div className="tt-column__body">
        <Platter compact={compact} />
        <NowLines />
        <div className="tt-line">
          <SeekBar strip />
        </div>
        <div className="tt-transport">
          <PlayControls />
        </div>
        <StripExtras />
        {!compact && (
          <section className="tt-next-wrap" aria-label="Up next">
            <UpNext />
          </section>
        )}
      </div>
      <SignalPanel />
    </aside>
  );
}

/** The browsing side's section tabs (its search sits in the window row above them). */
export function TurntableTabs() {
  const current = useCurrentEntry().route;
  const keys = useSourceKeys({ search: false });
  return (
    <div className="tt-browse-head">
      <nav className="tt-tabs" aria-label="Main">
        <ul className="tt-tabs__list">
          {keys.map((k) => {
            const on = k.on(current);
            return (
              <li key={k.key}>
                <button type="button" className={`tt-tab ${on ? "is-on" : ""}`} aria-current={on ? "page" : undefined} onClick={() => nav.go(k.route)}>
                  {k.label}
                </button>
              </li>
            );
          })}
        </ul>
      </nav>
    </div>
  );
}
