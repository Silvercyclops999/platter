/**
 * Now playing, full screen: where the music is playing from, the cover,
 * the song, the tonearm seek line, the transport within thumb reach, and
 * output, format and queue along the bottom. Under the player: the lyrics
 * card, sleeve notes and credits. A sideways swipe on the cover skips; a
 * pull down closes, straight down to the tab underneath.
 */
import { GettingLine } from "../components/GettingLine";
import { getting, useTaps } from "../stores/taps";
import { ChevronDown, ListMusic, MicVocal, MoreHorizontal, Pause, Play, Repeat, Repeat1, Shuffle, SkipBack, SkipForward } from "lucide-react";
import { memo, useDeferredValue, useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type RefObject } from "react";
import { Art, preloadArt, sleeveHue } from "../components/Art";
import { contextRoute, nextInQueue } from "../shell/nowPlaying";
import { holdLightStatusBar, mixIsDark } from "./statusBar";
import { Slider } from "../components/Slider";
import { formatDuration, trackArtistLine } from "../lib/format";
import type { QueueContextInfo, Track } from "../lib/types";
import { useArtColours } from "../lib/useTint";
import { useSleeveNotes } from "../lib/useSleeveNotes";
import { linerHasFacts, SleeveLiner } from "../pages/SleeveNotes";
import { useCreditRows } from "../shell/creditRows";
import { SEEK_KEY_STEP, SEEK_PAGE_STEP } from "../shell/PlayerBar";
import { player, usePlayer } from "../stores/player";
import { LikeControl, Marquee, remaining, RowGlyphs, tap } from "./bits";
import { SignalPlate } from "./SignalSheet";

import { OutputButton } from "./DeviceSheet";
import { CoverDeck, NP_ART_CSS } from "./CoverDeck";
import { LyricsCard } from "./LyricsScreen";
import { NP_OPEN_MS, flyCover } from "./motion";
import { phone, usePhoneUi } from "./phoneUi";
import type { HapticKind } from "../lib/backend";
import { setPhoneVis, usePhoneVis } from "../visualizer/phoneVis";
import { Stage } from "../visualizer/Stage";
import { stepVisStyle, useCoverTap, useVisStyle } from "../visualizer/useVis";
import type { VisualizerStyle } from "../lib/types";

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

export { NP_ART_CSS };
/** How much of the cover's colour tints the top of now playing (phone.css .ph-np). */
export const NP_TOP_TINT = 0.46;

/**
 * How big the title is set: a short one large, a long one smaller so it
 * still reads in two lines at most. By length, so it never jumps once drawn.
 */
export function titleFit(title: string): "short" | "medium" | "long" {
  const n = Array.from(title).length;
  return n <= 16 ? "short" : n <= 30 ? "medium" : "long";
}

/** The printed sleeve's tone as a colour the screen can be lit with, for a record with no cover. */
export function sleeveTint(title: string): string {
  // hsl(h 18% 30%), as hex so the status bar can tell how dark it is.
  const h = sleeveHue(title) / 360;
  const s = 0.18;
  const l = 0.3;
  const q = l < 0.5 ? l * (1 + s) : l + s - l * s;
  const p = 2 * l - q;
  const ch = (t: number) => {
    const k = ((t % 1) + 1) % 1;
    const v = k < 1 / 6 ? p + (q - p) * 6 * k : k < 1 / 2 ? q : k < 2 / 3 ? p + (q - p) * (2 / 3 - k) * 6 : p;
    return Math.round(v * 255)
      .toString(16)
      .padStart(2, "0");
  };
  return `#${ch(h + 1 / 3)}${ch(h)}${ch(h - 1 / 3)}`;
}


export function NowPlaying() {

  const open = usePhoneUi((s) => s.nowPlaying);
  const track = usePlayer((s) => s.state?.track ?? null);
  // The next song's cover, at now playing's size, so it is ready when the song changes.
  const nextThumb = usePlayer((s) => nextInQueue(s.state?.queue)?.track.thumb ?? null);
  useEffect(() => preloadArt(nextThumb, NP_ART_CSS), [nextThumb]);
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

/**
 * The screen itself: its tint, the cover's flight and the status bar. Closing
 * changes only this; everything on it (PlayerBody) stays as it is drawn, so
 * the first frame of the close is one class.
 */
/** The visualizer style now playing shows full screen, or null while it is closed. */
export function usePhoneArt(): VisualizerStyle | null {
  const coverTap = useCoverTap();
  const open = usePhoneVis((s) => s.open);
  const style = useVisStyle();
  return open && coverTap ? style : null;
}

/** A tap on the cover: the visualizer opens, or closes. */
function toggleVisualizer() {
  tap("selection");
  setPhoneVis(!usePhoneVis.getState().open);
}

/** The sharp cover the art gathers round: the deck's current one. */
const currentCover = (screen: RefObject<HTMLElement | null>) => () => screen.current?.querySelector<HTMLElement>(".ph-np__slide--cur .ph-np__cover") ?? null;

function Player({ track, closing }: { track: Track; closing: boolean }) {
  const colours = useArtColours(null, track.thumb);
  const screen = useRef<HTMLElement>(null);
  useCoverFlight(screen, closing);
  const art = usePhoneArt();
  // A tap on the art hides the controls, another brings them back; leaving the art brings them back too.
  const [bare, setBare] = useState(false);
  if (bare && !art) setBare(false);
  // Over a dark cover colour (or the art) the top of the screen is dark even on the light look: light status bar text then.
  // With no cover the screen takes the printed sleeve's tone.
  const vivid = track.thumb ? (colours?.vivid ?? null) : sleeveTint(track.albumTitle || track.title);
  const darkTop = !!art || mixIsDark(vivid, NP_TOP_TINT);
  useEffect(() => {
    if (closing || document.documentElement.dataset.themeBase !== "light" || !darkTop) return;
    const release = holdLightStatusBar();
    // Closing, the status bar changes once the screen has gone, not under the slide.
    return () => void (usePhoneUi.getState().nowPlaying ? release() : setTimeout(release, CLOSE_MS));
  }, [darkTop, closing]);
  // The pull down writes its transform and is-dragging straight onto the screen (CoverDeck).
  const style = { "--ph-tint": vivid ?? "var(--color-bg-overlay)" } as CSSProperties;
  return (
    <section
      ref={screen}
      className={`ph-np ${closing ? "is-closing" : ""} ${art ? `is-art is-art--${art}` : ""} ${bare ? "is-bare" : ""}`}
      role="dialog"
      aria-modal="true"
      aria-label="Now playing"
      style={style}
    >
      {art && <PhoneArtStage art={art} screen={screen} />}
      <PlayerBody track={track} screen={screen} art={art} bare={bare} onBare={setBare} />
      {art && bare && <ShowControls screen={screen} onShow={() => setBare(false)} />}
    </section>
  );
}

function PhoneArtStage({ art, screen }: { art: VisualizerStyle; screen: RefObject<HTMLElement | null> }) {
  const [cover] = useState(() => currentCover(screen));
  return <Stage style={art} phone cover={cover} />;
}

/**
 * With the controls hidden over the art, the way back for everyone: a quiet
 * button outside the hidden part, which takes focus so it never lands on
 * nothing; Escape, or focus moving anywhere else on the screen, brings the
 * controls back too.
 */
function ShowControls({ screen, onShow }: { screen: RefObject<HTMLElement | null>; onShow: () => void }) {
  const button = useRef<HTMLButtonElement>(null);
  const show = useRef(onShow);
  show.current = onShow;
  useEffect(() => {
    // Focus that sat on a control now hidden moves here, so it never rests on nothing.
    button.current?.focus({ preventScroll: true });
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.preventDefault();
      e.stopPropagation();
      show.current();
    };
    const onFocus = (e: FocusEvent) => {
      if (e.target !== button.current) show.current();
    };
    const sec = screen.current;
    window.addEventListener("keydown", onKey, true);
    sec?.addEventListener("focusin", onFocus);
    return () => {
      window.removeEventListener("keydown", onKey, true);
      sec?.removeEventListener("focusin", onFocus);
    };
  }, [screen]);
  return (
    <button
      ref={button}
      type="button"
      className="ph-np__show"
      onClick={() => {
        show.current();
        // Back on the controls, at the top of the screen.
        requestAnimationFrame?.(() => screen.current?.querySelector<HTMLElement>(".ph-np__top button")?.focus({ preventScroll: true }));
      }}
    >
      Show controls
    </button>
  );
}

/** Presses that are the controls' own (or the cover's), not a tap on the art. */
const ON_A_CONTROL = "button, a, input, select, textarea, [role=slider], [role=button], [role=switch], .ph-seek, .ph-np__art";

interface BodyProps {
  track: Track;
  screen: RefObject<HTMLElement | null>;
  art?: VisualizerStyle | null;
  bare?: boolean;
  onBare?: (bare: boolean) => void;
}

const PlayerBody = memo(function PlayerBody({ track, screen, art = null, bare = false, onBare }: BodyProps) {
  const ctx = usePlayer((s) => s.state?.queue.context ?? null);
  const source = usePlayer((s) => s.state?.queue.current?.source);
  const status = usePlayer((s) => s.state?.status ?? "idle");
  const playing = status === "playing" || status === "loading" || status === "buffering";
  // The cards under the player follow a new song a moment later, in pieces that give way to the screen.
  const below = useDeferredValue(track);
  // A tap on the cover opens the visualizer (and closes it); a sideways swipe over it changes the style.
  const coverTap = useCoverTap();
  const scroller = useRef<HTMLDivElement>(null);
  useScrolled(scroller);
  return (
    <div ref={scroller} className="ph-np__scroll">
      <div
        className="ph-np__player"
        data-testid="np-player"
        onClick={(e) => {
          if (!art || !onBare) return;
          if (e.target instanceof Element && e.target.closest(ON_A_CONTROL)) return;
          onBare(!bare);
        }}
      >
        <header className="ph-np__top" inert={bare || undefined}>
          <button type="button" className="ph-icon-btn" aria-label="Close now playing" onClick={phone.closePlayer}>
            <ChevronDown size={28} aria-hidden="true" />
          </button>
          <PlayingFrom ctx={ctx} source={source} />
          <button type="button" className="ph-icon-btn" aria-label="More for this song" onClick={() => phone.push({ kind: "track", track })}>
            <MoreHorizontal size={24} aria-hidden="true" />
          </button>
        </header>
        <CoverDeck
          screen={screen}
          onClose={phone.closePlayer}
          art={art}
          onTap={coverTap ? toggleVisualizer : undefined}
          onStyleSwipe={art ? stepVisStyle : undefined}
        />
        <div className="ph-np__deck" inert={bare || undefined}>
        <div className="ph-np__meta">
          <div className="ph-np__names">
            <button type="button" className="ph-np__title" data-fit={titleFit(track.title)} onClick={() => phone.go({ name: "album", id: track.albumId })}>
              <span className="ph-np__title-text">{track.title}</span>
            </button>
            <span className="ph-np__artist-line">
              <RowGlyphs trackId={track.id} />
              <button type="button" className="ph-np__artist" aria-label={trackArtistLine(track)} onClick={() => phone.go({ name: "artist", id: track.artistId })}>
                <Marquee text={trackArtistLine(track)} moving={playing} />
              </button>
            </span>
            {/* The album, unless it is what's playing from (the top bar says so already). */}
            {track.albumTitle && !(ctx?.kind === "album" && ctx.id === track.albumId) && (
              <button type="button" className="ph-np__album" onClick={() => phone.go({ name: "album", id: track.albumId })}>
                <span>{track.albumTitle}</span>
              </button>
            )}
          </div>
          <LikeControl track={track} />
        </div>
        <NowGetting />
        <PhoneSeek />
        <Transport playing={playing} />
        <div className="ph-np__bottom">
          <div className="ph-np__out">
            <OutputButton />
            <SignalPlate track={track} />
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
      <UpNext />
      <LyricsCard track={below} />
      <SleeveCard track={below} />
      <CreditsCard track={below} />
    </div>
  );
});

/**
 * "Playing from" and where: a link back to it when it is a page.
 */
function PlayingFrom({ ctx, source }: { ctx: QueueContextInfo | null; source?: string }) {
  const name = playingFrom(ctx, source);
  const route = source === "autoplay" || source === "user" ? null : contextRoute(ctx);
  const inner = (
    <>
      <span className="ph-np__from-label">Playing from</span>
      <span className="ph-np__from-name">{name}</span>
    </>
  );
  if (!route) return <div className="ph-np__from">{inner}</div>;
  return (
    <button type="button" className="ph-np__from" onClick={() => phone.go(route)}>
      {inner}
    </button>
  );
}

/** The top bar takes a backing only once the screen has scrolled, so at rest it is the screen itself. */
function useScrolled(ref: RefObject<HTMLElement | null>) {
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const sec = el.closest(".ph-np");
    let on = false;
    const check = () => {
      const now = el.scrollTop > 8;
      if (now === on) return;
      on = now;
      sec?.classList.toggle("is-scrolled", now);
    };
    el.addEventListener("scroll", check, { passive: true });
    return () => el.removeEventListener("scroll", check);
  }, [ref]);
}

/**
 * The next song, just under the player so the foot of the screen is never
 * empty: its cover, title and artist. A tap opens the queue.
 */
function UpNext() {
  const next = usePlayer((s) => nextInQueue(s.state?.queue)?.track ?? null);
  if (!next) return null;
  return (
    <section className="ph-upnext" aria-label="Up next">
      <button
        type="button"
        className="ph-upnext__row"
        aria-label={`Up next: ${next.title}, ${trackArtistLine(next)}. Open the queue`}
        onClick={() => {
          tap();
          phone.push({ kind: "queue" });
        }}
      >
        <span className="ph-upnext__art">
          <Art path={next.thumb} size={40} sleeve={{ title: next.albumTitle, artist: next.artistTitle }} />
        </span>
        <span className="ph-upnext__text">
          <span className="ph-upnext__label">Up next</span>
          <span className="ph-upnext__title">{next.title}</span>
          <span className="ph-upnext__artist">{trackArtistLine(next)}</span>
        </span>
        <ListMusic className="ph-upnext__icon" size={20} aria-hidden="true" />
      </button>
    </section>
  );
}

/**
 * The small cover now playing grows from: the mini player's, or on Home
 * the now-playing card's. Only one that is on screen counts.
 */
function smallCover(root: HTMLElement): HTMLElement | null {
  const el = root.querySelector<HTMLElement>(".ph-dock:not(.is-mini-away) .ph-mini__art") ?? root.querySelector<HTMLElement>(".ph-layer:not(.is-under):not(.is-parked) .ph-now__art");
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
    const big = sec?.querySelector<HTMLElement>(".ph-np__slide--cur .ph-np__cover");
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

/**
 * An album's sleeve notes: the computer's printed release side, compact.
 * Under now playing its heading is an h3 like the cards beside it; on the
 * album page, under the h1, an h2.
 */
export function SleeveCard({ track, link = true, level = 3 }: { track: Pick<Track, "albumId" | "albumTitle" | "artistTitle" | "thumb">; link?: boolean; level?: 2 | 3 }) {
  const { data } = useSleeveNotes(track.albumId);
  if (!data || !linerHasFacts(data)) return null;
  // The computer's printed sleeve, set for the phone's width.
  return (
    <div className="ph-sleeve-card">
      <SleeveLiner
        n={data}
        compact
        level={level}
        heading="Sleeve notes"
        action={
          link ? (
            <button type="button" className="ph-card__link" onClick={() => phone.go({ name: "album", id: track.albumId, tab: "sleeve" })}>
              Open the album
            </button>
          ) : undefined
        }
      />
    </div>
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

/** A tapped song on its way, over the seek bar of what's playing meanwhile. */
function NowGetting() {
  const t = useTaps((s) => s.current);
  if (!getting(t)) return null;
  return (
    <div className="ph-np__getting">
      <span className="ph-np__getting-title">Getting {t.title}</span>
      <GettingLine tap={t} />
    </div>
  );
}
