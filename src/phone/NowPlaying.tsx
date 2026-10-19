/**
 * Now playing, full screen: where the music is playing from, the cover,
 * the song, the tonearm seek line, the transport within thumb reach, and
 * output, format and queue along the bottom. Under the player: the lyrics
 * card, sleeve notes and credits. A sideways swipe on the cover skips; a
 * pull down closes, straight down to the tab underneath.
 */
import { ChevronDown, ListMusic, MicVocal, MoreHorizontal, Pause, Play, Repeat, Repeat1, Shuffle, SkipBack, SkipForward } from "lucide-react";
import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties } from "react";
import { Art } from "../components/Art";
import { Slider } from "../components/Slider";
import { formatDuration, trackArtistLine } from "../lib/format";
import type { Track } from "../lib/types";
import { useArtColours } from "../lib/useTint";
import { useSleeveNotes } from "../lib/useSleeveNotes";
import { useCreditRows } from "../shell/creditRows";
import { SEEK_KEY_STEP, SEEK_PAGE_STEP } from "../shell/PlayerBar";
import { player, usePlayer } from "../stores/player";
import { FormatPlate, LikeControl, Marquee, remaining, RowGlyphs, tap } from "./bits";

import { OutputButton } from "./DeviceSheet";
import { useArtGestures } from "./gestures";
import { LyricsCard } from "./LyricsScreen";
import { NP_OPEN_MS, flyCover } from "./motion";
import { phone, usePhoneUi } from "./phoneUi";
import type { HapticKind } from "../lib/backend";

/** The close slides straight down in this long. */
export const CLOSE_MS = 300;

const CONTEXT_NOUN: Record<string, string> = {
  album: "album",
  playlist: "playlist",
  artist: "artist",
  liked: "Liked Songs",
  mix: "mix",
  radio: "radio",
  search: "search",
  tracks: "your songs",
};

/** "Playing from" and its source. */
export function playingFrom(ctx: { kind: string; title?: string | null } | null | undefined, source?: string): string {
  // Past the end of the record or list: what plays is chosen to follow it, or queued by hand.
  if (source === "autoplay") return ctx?.title ? `Autoplay after ${ctx.title}` : "Autoplay";
  if (source === "user") return "Your queue";
  if (!ctx) return "Your queue";
  if (ctx.title) return ctx.title;
  return ctx.kind === "liked" ? "Liked Songs" : `This ${CONTEXT_NOUN[ctx.kind] ?? "queue"}`;
}

export function NowPlaying() {
  const open = usePhoneUi((s) => s.nowPlaying);
  const track = usePlayer((s) => s.state?.track ?? null);
  // Kept on screen while it slides away.
  const [shown, setShown] = useState(open);
  useEffect(() => {
    if (open) return setShown(true);
    const t = setTimeout(() => setShown(false), CLOSE_MS);
    return () => clearTimeout(t);
  }, [open]);
  if (!shown || !track) return null;
  return <Player track={track} closing={!open} />;
}

function Player({ track, closing }: { track: Track; closing: boolean }) {
  const ctx = usePlayer((s) => s.state?.queue.context ?? null);
  const source = usePlayer((s) => s.state?.queue.current?.source);
  const status = usePlayer((s) => s.state?.status ?? "idle");
  const colours = useArtColours(null, track.thumb);
  const playing = status === "playing" || status === "loading" || status === "buffering";
  const screen = useRef<HTMLElement>(null);
  const art = useRef<HTMLDivElement>(null);
  const { offset, handlers } = useArtGestures({
    onNext: () => {
      tap();
      void player.next();
    },
    onPrevious: () => {
      tap();
      void player.previous();
    },
    onClose: phone.closePlayer,
    size: () => ({ width: art.current?.clientWidth || 360, height: screen.current?.clientHeight || 800 }),
  });
  useCoverFlight(screen, closing);
  const style = {
    "--ph-tint": colours?.vivid ?? "var(--color-bg-overlay)",
    transform: offset.y ? `translateY(${offset.y}px)` : undefined,
  } as CSSProperties;
  return (
    <section
      ref={screen}
      className={`ph-np ${closing ? "is-closing" : ""} ${offset.dragging ? "is-dragging" : ""}`}
      role="dialog"
      aria-modal="true"
      aria-label="Now playing"
      style={style}
    >
      <div className="ph-np__scroll">
        <div className="ph-np__player">
          <header className="ph-np__top">
            <button type="button" className="ph-icon-btn" aria-label="Close now playing" onClick={phone.closePlayer}>
              <ChevronDown size={28} aria-hidden="true" />
            </button>
            <div className="ph-np__from">
              <span className="ph-np__from-label">Playing from</span>
              <span className="ph-np__from-name">{playingFrom(ctx, source)}</span>
            </div>
            <button type="button" className="ph-icon-btn" aria-label="More for this song" onClick={() => phone.push({ kind: "track", track })}>
              <MoreHorizontal size={24} aria-hidden="true" />
            </button>
          </header>
          <div
            ref={art}
            className="ph-np__art"
            data-testid="np-art"
            {...handlers}
            style={{ transform: offset.x ? `translateX(${offset.x}px)` : undefined }}
          >
            <div className={`ph-np__cover ${playing ? "is-playing" : ""}`}>
              <Art path={track.thumb} fluid requestSize={900} sleeve={{ title: track.albumTitle, artist: track.artistTitle }} />
            </div>
          </div>
          <div className="ph-np__deck">
          <div className="ph-np__meta">
            <div className="ph-np__names">
              <button type="button" className="ph-np__title" onClick={() => phone.go({ name: "album", id: track.albumId })}>
                {track.title}
              </button>
              <span className="ph-np__artist-line">
                <RowGlyphs trackId={track.id} />
                <button type="button" className="ph-np__artist" aria-label={trackArtistLine(track)} onClick={() => phone.go({ name: "artist", id: track.artistId })}>
                  <Marquee text={trackArtistLine(track)} moving={playing} />
                </button>
              </span>
            </div>
            <LikeControl track={track} />
          </div>
          <PhoneSeek />
          <Transport playing={playing} />
          <div className="ph-np__bottom">
            <div className="ph-np__out">
              <OutputButton />
              <FormatPlate track={track} />
            </div>
            <div className="ph-np__right">
            <button
              type="button"
              className="ph-icon-btn"
              aria-label="Lyrics"
              onClick={() => {
                tap();
                phone.openLyrics();
              }}
            >
              <MicVocal size={22} aria-hidden="true" />
            </button>
            <button
              type="button"
              className="ph-icon-btn"
              aria-label="Queue"
              onClick={() => {
                tap();
                phone.push({ kind: "queue" });
              }}
            >
              <ListMusic size={24} aria-hidden="true" />
            </button>
            </div>
          </div>
          </div>
        </div>
        <LyricsCard track={track} />
        <SleeveCard track={track} />
        <CreditsCard track={track} />
      </div>
    </section>
  );
}

/**
 * The small cover now playing grows from: the mini player's, or on Home
 * the now-playing card's. Only one that is on screen counts.
 */
function smallCover(root: HTMLElement): HTMLElement | null {
  const el = root.querySelector<HTMLElement>(".ph-dock:not(.is-mini-away) .ph-mini__art") ?? root.querySelector<HTMLElement>(".ph-layer:not(.is-under) .ph-now__art");
  if (!el) return null;
  const r = el.getBoundingClientRect();
  return r.width > 0 && r.bottom > 0 && r.top < (root.clientHeight || window.innerHeight) ? el : null;
}

/**
 * The cover's flight: opening, it grows out of the small cover into its
 * place while the screen rises; closing, it shrinks back into the small
 * cover while the screen falls away. The mini player hides meanwhile.
 */
function useCoverFlight(screen: React.RefObject<HTMLElement | null>, closing: boolean) {
  const fly = (direction: "open" | "close") => {
    const sec = screen.current;
    const root = sec?.closest<HTMLElement>(".phone");
    const big = sec?.querySelector<HTMLElement>(".ph-np__cover");
    if (!sec || !root || !big || typeof big.animate !== "function") return;
    const small = smallCover(root);
    if (!small) return;
    const b = big.getBoundingClientRect();
    if (b.bottom < 0) return;
    let bigRect;
    if (direction === "open") {
      // Where the cover lands once the screen has risen (it starts a screen lower).
      const s = sec.getBoundingClientRect();
      const r = root.getBoundingClientRect();
      bigRect = { left: b.left - s.left + r.left, top: b.top - s.top + r.top, width: b.width, height: b.height };
    }
    root.classList.add("is-cover-flying");
    void flyCover({ root, small, big, bigRect, direction, ms: direction === "open" ? NP_OPEN_MS : CLOSE_MS }).finally(() => root.classList.remove("is-cover-flying"));
  };
  useLayoutEffect(() => {
    if (!closing) fly("open");
  }, []); // eslint-disable-line react-hooks/exhaustive-deps
  useLayoutEffect(() => {
    if (closing) fly("close");
  }, [closing]); // eslint-disable-line react-hooks/exhaustive-deps
}

/** The tonearm seek line with the time gone left and the time left right. */
export function PhoneSeek() {
  const positionMs = usePlayer((s) => s.positionMs);
  const durationMs = usePlayer((s) => s.durationMs);
  const trackId = usePlayer((s) => s.state?.track?.id ?? null);
  const [drag, setDrag] = useState<number | null>(null);
  const shown = drag ?? positionMs;
  return (
    <div className="ph-seek">
      <Slider
        label="Seek"
        className="ph-scrub"
        value={Math.min(shown, durationMs)}
        max={Math.max(durationMs, 1)}
        step={SEEK_KEY_STEP}
        pageStep={SEEK_PAGE_STEP}
        disabled={!trackId}
        resetKey={trackId}
        valueText={(v) => `${formatDuration(v)} of ${formatDuration(durationMs)}`}
        onChange={(v) => setDrag(v)}
        onCommit={(v) => {
          setDrag(null);
          void player.seek(Math.round(v));
        }}
      />
      <div className="ph-seek__times tabular" aria-hidden="true">
        <span>{formatDuration(shown)}</span>
        <span>{remaining(shown, durationMs)}</span>
      </div>
    </div>
  );
}

function Transport({ playing }: { playing: boolean }) {
  const shuffle = usePlayer((s) => !!s.state?.shuffle);
  const repeat = usePlayer((s) => s.state?.repeat ?? "off");
  const RepeatIcon = repeat === "one" ? Repeat1 : Repeat;
  const press = (f: () => unknown, kind: HapticKind = "light") => () => {
    tap(kind);
    void f();
  };
  return (
    <div className="ph-transport">
      <button type="button" className={`ph-icon-btn ${shuffle ? "is-on" : ""}`} aria-label="Shuffle" aria-pressed={shuffle} onClick={press(() => player.toggleShuffle(), "selection")}>
        <Shuffle size={22} aria-hidden="true" />
      </button>
      <button type="button" className="ph-icon-btn ph-transport__skip" aria-label="Previous" onClick={press(player.previous)}>
        <SkipBack size={30} fill="currentColor" aria-hidden="true" />
      </button>
      <button type="button" className="ph-play" aria-label={playing ? "Pause" : "Play"} onClick={press(player.toggle)}>
        {playing ? <Pause size={30} fill="currentColor" aria-hidden="true" /> : <Play size={30} fill="currentColor" aria-hidden="true" />}
      </button>
      <button type="button" className="ph-icon-btn ph-transport__skip" aria-label="Next" onClick={press(player.next)}>
        <SkipForward size={30} fill="currentColor" aria-hidden="true" />
      </button>
      <button
        type="button"
        className={`ph-icon-btn ${repeat !== "off" ? "is-on" : ""}`}
        aria-label={repeat === "one" ? "Repeat this song" : repeat === "all" ? "Repeat" : "Repeat off"}
        aria-pressed={repeat !== "off"}
        onClick={press(player.cycleRepeatNow, "selection")}
      >
        <RepeatIcon size={22} aria-hidden="true" />
      </button>
    </div>
  );
}

const dayFmt = new Intl.DateTimeFormat(undefined, { day: "numeric", month: "long", year: "numeric", timeZone: "UTC" });

/**
 * A date from the sleeve notes as people write it: "3 March 2007". A 1 January
 * date is how a year alone often arrives, so it shows as the year.
 */
export function shownDate(v: string): string {
  const m = /^(\d{4})-(\d{2})-(\d{2})/.exec(v);
  if (!m) return v;
  if (m[2] === "01" && m[3] === "01") return m[1];
  const d = new Date(Date.UTC(Number(m[1]), Number(m[2]) - 1, Number(m[3])));
  return Number.isNaN(d.getTime()) ? v : dayFmt.format(d);
}

/** An album's sleeve notes as a card: the facts, then the release's own notes. */
export function SleeveCard({ track, link = true }: { track: Pick<Track, "albumId" | "albumTitle" | "artistTitle" | "thumb">; link?: boolean }) {
  const { data } = useSleeveNotes(track.albumId);
  if (!data) return null;
  const facts = [
    ["Released", data.released?.value ? shownDate(data.released.value) : undefined],
    ["Recorded", data.recorded?.value ? shownDate(data.recorded.value) : undefined],
    ["Label", data.label?.value],
    ["Studio", data.studios[0]?.name],
  ].filter((f): f is [string, string] => !!f[1]);
  const notes = data.notes?.value;
  if (!facts.length && !notes) return null;
  return (
    <section className="ph-card ph-sleeve-card" aria-label="Sleeve notes">
      <div className="ph-card__head">
        <h3 className="ph-card__title">Sleeve notes</h3>
        {link && (
          <button type="button" className="ph-card__link" onClick={() => phone.go({ name: "album", id: track.albumId, tab: "sleeve" })}>
            Open the album
          </button>
        )}
      </div>
      {facts.length > 0 && (
        <dl className="ph-facts">
          {facts.map(([k, v]) => (
            <div key={k} className="ph-facts__row">
              <dt>{k}</dt>
              <dd className="tabular">{v}</dd>
            </div>
          ))}
        </dl>
      )}
      {notes && <p className="ph-sleeve-card__notes">{notes}</p>}
    </section>
  );
}

function CreditsCard({ track }: { track: Track }) {
  const { rows } = useCreditRows(track);
  if (!rows.length) return null;
  return (
    <section className="ph-card ph-credits-card" aria-label="Credits">
      <div className="ph-card__head">
        <h3 className="ph-card__title">Credits</h3>
      </div>
      <ul className="ph-credits">
        {rows.slice(0, 6).map((r) => (
          <li key={r.name} className="ph-credits__row">
            {r.id ? (
              <button type="button" className="ph-credits__name" onClick={() => phone.go({ name: "artist", id: r.id! })}>
                {r.name}
              </button>
            ) : (
              <span className="ph-credits__name">{r.name}</span>
            )}
            <span className="ph-credits__roles">{r.roles.join(", ")}</span>
          </li>
        ))}
      </ul>
    </section>
  );
}
