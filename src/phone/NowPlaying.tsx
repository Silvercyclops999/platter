/**
 * Now playing, full screen: where the music is playing from, the record,
 * the song, the tonearm seek line, the transport within thumb reach, and
 * output, format and queue along the bottom. Under the player: the lyrics
 * card, sleeve notes and credits. A sideways swipe on the record skips; a
 * pull down closes, straight down to the tab underneath.
 */
import { ChevronDown, ListMusic, MoreHorizontal, Pause, Play, Repeat, Repeat1, Shuffle, SkipBack, SkipForward } from "lucide-react";
import { useEffect, useRef, useState, type CSSProperties } from "react";
import { Art } from "../components/Art";
import { Slider } from "../components/Slider";
import { formatDuration, trackArtistLine } from "../lib/format";
import type { Track } from "../lib/types";
import { useArtColours } from "../lib/useTint";
import { useSleeveNotes } from "../lib/useSleeveNotes";
import { useCreditRows } from "../shell/creditRows";
import { SEEK_KEY_STEP, SEEK_PAGE_STEP } from "../shell/PlayerBar";
import { Turntable } from "../shell/Turntable";
import { player, usePlayer } from "../stores/player";
import { useSettings } from "../stores/settings";
import { FormatPlate, LikeControl, remaining, tap } from "./bits";
import { OutputButton } from "./DeviceSheet";
import { useArtGestures } from "./gestures";
import { LyricsCard } from "./LyricsScreen";
import { phone, usePhoneUi } from "./phoneUi";

/** The close slides straight down in this long. */
export const CLOSE_MS = 350;

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
export function playingFrom(ctx: { kind: string; title?: string | null } | null | undefined): string {
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
  const status = usePlayer((s) => s.state?.status ?? "idle");
  const spin = useSettings((s) => (s.settings?.recordMotion ?? "off") !== "off");
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
              <span className="ph-np__from-name">{playingFrom(ctx)}</span>
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
            <Turntable thumb={track.thumb} playing={playing && spin} sleeve={{ title: track.albumTitle, artist: track.artistTitle }} requestSize={600} />
          </div>
          <div className="ph-np__meta">
            <div className="ph-np__names">
              <button type="button" className="ph-np__title" onClick={() => phone.go({ name: "album", id: track.albumId })}>
                {track.title}
              </button>
              <button type="button" className="ph-np__artist" onClick={() => phone.go({ name: "artist", id: track.artistId })}>
                {trackArtistLine(track)}
              </button>
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
        <LyricsCard track={track} />
        <SleeveCard track={track} />
        <CreditsCard track={track} />
      </div>
    </section>
  );
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
        className="slider--tonearm"
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
  const press = (f: () => unknown) => () => {
    tap();
    void f();
  };
  return (
    <div className="ph-transport">
      <button type="button" className={`ph-icon-btn ${shuffle ? "is-on" : ""}`} aria-label="Shuffle" aria-pressed={shuffle} onClick={press(() => player.toggleShuffle())}>
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
        onClick={press(player.cycleRepeatNow)}
      >
        <RepeatIcon size={22} aria-hidden="true" />
      </button>
    </div>
  );
}

/** An album's sleeve notes as a card: the facts, then the release's own notes. */
export function SleeveCard({ track, link = true }: { track: Pick<Track, "albumId" | "albumTitle" | "artistTitle" | "thumb">; link?: boolean }) {
  const { data } = useSleeveNotes(track.albumId);
  if (!data) return null;
  const facts = [
    ["Released", data.released?.value],
    ["Recorded", data.recorded?.value],
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
      <div className="ph-sleeve-card__record">
        <Art path={track.thumb} size={56} sleeve={{ title: track.albumTitle, artist: track.artistTitle }} />
        <span className="ph-sleeve-card__album">{track.albumTitle}</span>
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
