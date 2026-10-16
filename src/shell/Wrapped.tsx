/**
 * Wrapped: a year in seven cards that fill the window. Each card leads with
 * one sentence made from the year's numbers, uses real artwork and one
 * accent (the year's colour).
 *
 * Cards move on by themselves; arrow keys or a click move between them
 * (the left third goes back), holding the pointer down (or Space) pauses,
 * Esc closes. Under reduced motion nothing moves: the cards are static and
 * wait for you. The last card is the summary, which saves or copies as a
 * 1080 by 1920 image; unreleased songs stay off it unless turned on there.
 */
import { ChevronLeft, ChevronRight, Copy, Download, Pause, Play, RotateCcw, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState, type CSSProperties, type PointerEvent as ReactPointerEvent, type ReactNode } from "react";
import { Art } from "../components/Art";
import { Button, IconButton } from "../components/Button";
import { CARD_FAILED, loadImage } from "../components/ShareCardDialog";
import { Switch } from "../components/Switch";
import { prefersReducedMotion } from "../design/motion";
import { artUrl, backend } from "../lib/backend";
import { canvasMeasure, CARD_SIZES } from "../lib/shareCard";
import type { TopSong, Wrapped as WrappedData } from "../lib/statsTypes";
import { useLoad } from "../lib/useLoad";
import { albumWords, artistWords, dayWords, drawSummaryCard, layoutSummaryCard, minutesWords, personalityWords, songWords, summaryContent, yearTone } from "../lib/wrapped";
import { YearBanner } from "../pages/TopSongsTile";
import { player } from "../stores/player";
import { nav } from "../stores/router";
import { toast } from "../stores/toasts";
import { useWrapped, wrapped } from "../stores/wrapped";
import "./wrapped.css";

/** How long a card stays before the next one. */
export const CARD_MS = 9000;
/** A press longer than this is a hold (pause), not a click. */
export const HOLD_MS = 220;

export type CardKind = "minutes" | "artists" | "songs" | "albums" | "personality" | "day" | "summary";

const CARD_NAMES: Record<CardKind, string> = {
  minutes: "Minutes listened",
  artists: "Top artists",
  songs: "Top songs",
  albums: "Top albums and genres",
  personality: "Listening personality",
  day: "Biggest day",
  summary: "Your year",
};

/** The cards a year has, in order; a card without numbers behind it is left out. */
export function wrappedCards(w: WrappedData): CardKind[] {
  if (w.totals.plays === 0) return [];
  const out: CardKind[] = ["minutes"];
  if (w.topArtists.length) out.push("artists");
  if (w.topSongs.length) out.push("songs");
  if (w.topAlbums.length) out.push("albums");
  if (w.personality) out.push("personality");
  if (w.biggestDay) out.push("day");
  out.push("summary");
  return out;
}

function Kicker({ children }: { children: ReactNode }) {
  return <p className="wcard__kicker">{children}</p>;
}

function UnreleasedMark() {
  return <span className="wcard__mark">Unreleased</span>;
}

function MinutesCard({ w }: { w: WrappedData }) {
  const m = minutesWords(w);
  return (
    <div className="wcard wcard--minutes">
      <div className="wcard__art">
        <YearBanner year={w.year} w={1440} h={900} />
      </div>
      <div className="wcard__body">
        <Kicker>Your {w.year} Wrapped</Kicker>
        <h2 className="wcard__lead">{m.lead}</h2>
        <p className="wcard__big">
          <span className="wcard__accent">{m.big}</span>
          <span className="wcard__unit">minutes</span>
        </p>
        <p className="wcard__sub">{m.change}</p>
      </div>
    </div>
  );
}

function ArtistsCard({ w }: { w: WrappedData }) {
  const words = artistWords(w)!;
  const top = w.topArtists[0];
  return (
    <div className="wcard wcard--artists">
      {top.thumb && (
        <div className="wcard__photo" aria-hidden="true">
          <img src={artUrl(top.thumb, 1600, 1600)} alt="" draggable={false} />
        </div>
      )}
      <div className="wcard__body">
        <Kicker>Top artists</Kicker>
        <h2 className="wcard__lead">{words.lead}</h2>
        <p className="wcard__sub">{words.sub}</p>
        <ol className="wcard__ranks">
          {w.topArtists.map((a, i) => (
            <li key={a.key} className={i === 0 ? "is-first" : ""}>
              <span className="wcard__rank">{i + 1}</span>
              <span className="wcard__name">{a.name}</span>
            </li>
          ))}
        </ol>
      </div>
    </div>
  );
}

function SongsCard({ w }: { w: WrappedData }) {
  const words = songWords(w)!;
  const playable = w.topSongs.filter((s) => s.track).map((s) => s.track!.id);
  const play = (s: TopSong) => s.track && void player.playTracks(playable, s.track.id);
  return (
    <div className="wcard wcard--songs">
      <div className="wcard__body wcard__body--split">
        <div className="wcard__words">
          <Kicker>Top songs</Kicker>
          <h2 className="wcard__lead">{words.lead}</h2>
          <p className="wcard__sub">{words.sub}</p>
        </div>
        <ol className="wcard__songs">
          {w.topSongs.map((s, i) => {
            const inner = (
              <>
                <span className="wcard__rank">{i + 1}</span>
                <span className="wcard__cover">
                  <Art path={s.track?.thumb ?? null} size={64} />
                  {s.track && (
                    <span className="wcard__play" aria-hidden="true">
                      <Play size={18} fill="currentColor" />
                    </span>
                  )}
                </span>
                <span className="wcard__song">
                  <span className="wcard__name truncate">
                    {s.title}
                    {s.unreleased && <UnreleasedMark />}
                  </span>
                  <span className="wcard__by truncate">{s.artist}</span>
                </span>
                <span className="wcard__plays tabular">{s.plays.toLocaleString()} plays</span>
              </>
            );
            return (
              <li key={s.key} className={i === 0 ? "is-first" : ""}>
                {s.track ? (
                  <button type="button" className="wcard__row" aria-label={`Play ${s.title} by ${s.artist}`} onClick={() => play(s)}>
                    {inner}
                  </button>
                ) : (
                  <div className="wcard__row">{inner}</div>
                )}
              </li>
            );
          })}
        </ol>
      </div>
    </div>
  );
}

function AlbumsCard({ w }: { w: WrappedData }) {
  const words = albumWords(w)!;
  const [first, ...rest] = w.topAlbums;
  return (
    <div className="wcard wcard--albums">
      <div className="wcard__body wcard__body--split">
        <div className="wcard__words">
          <Kicker>Top albums and genres</Kicker>
          <h2 className="wcard__lead">{words.lead}</h2>
          <p className="wcard__sub">{words.sub}</p>
          <ol className="wcard__albums">
            {rest.map((a, i) => (
              <li key={a.key}>
                <Art path={a.thumb} size={88} />
                <span className="wcard__album-text">
                  <span className="wcard__rank-sm">{i + 2}</span>
                  <span className="wcard__name truncate">{a.title}</span>
                  <span className="wcard__by truncate">{a.artist}</span>
                </span>
              </li>
            ))}
          </ol>
          {words.genres && <p className="wcard__genres">{words.genres}</p>}
        </div>
        <div className="wcard__hero-cover">
          <Art path={first.thumb} fluid requestSize={640} />
          {first.unreleased && <UnreleasedMark />}
        </div>
      </div>
    </div>
  );
}

function PersonalityCard({ w }: { w: WrappedData }) {
  const p = personalityWords(w.personality!, w.hours);
  return (
    <div className="wcard wcard--personality">
      <div className="wcard__art wcard__art--quiet">
        <YearBanner year={w.year} w={1440} h={900} />
      </div>
      <div className="wcard__body">
        <Kicker>Listening personality</Kicker>
        <h2 className="wcard__lead">{p.lead}</h2>
        <ul className="wcard__traits">
          {p.traits.map((t) => (
            <li key={t.label}>
              <span className="wcard__value wcard__accent">{t.value}</span>
              <span className="wcard__trait">{t.label}</span>
              <span className="wcard__line">{t.line}</span>
            </li>
          ))}
        </ul>
      </div>
    </div>
  );
}

function DayCard({ w }: { w: WrappedData }) {
  const d = dayWords(w)!;
  const r = w.replayed;
  return (
    <div className="wcard wcard--day">
      <div className={`wcard__body ${r ? "wcard__body--split" : ""}`}>
        <div className="wcard__words">
          <Kicker>Biggest day</Kicker>
          <h2 className="wcard__lead">{d.lead}</h2>
          <p className="wcard__sub">{d.sub}</p>
        </div>
        {r && d.replay && (
          <figure className="wcard__replay">
            <span className="wcard__replay-cover">
              <Art path={r.song.track?.thumb ?? null} fluid requestSize={640} />
            </span>
            <figcaption>
              <p className="wcard__kicker">Most replayed in a day</p>
              <p className="wcard__replay-line">
                {d.replay}
                {r.song.unreleased && <UnreleasedMark />}
              </p>
              {r.song.track && (
                <Button size="sm" icon={<Play size={14} fill="currentColor" />} onClick={() => void player.playTracks([r.song.track!.id], r.song.track!.id)}>
                  Play it
                </Button>
              )}
            </figcaption>
          </figure>
        )}
      </div>
    </div>
  );
}

function SummaryCard({ w, onRestart }: { w: WrappedData; onRestart: () => void }) {
  const [withUnreleased, setWithUnreleased] = useState(false);
  const hasUnreleased = w.topSongs.some((s) => s.unreleased);
  const content = useMemo(() => summaryContent(w, withUnreleased), [w, withUnreleased]);
  const canvas = useRef<HTMLCanvasElement>(null);
  const [img, setImg] = useState<HTMLImageElement | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let live = true;
    if (content.artistArt) void loadImage(backend.artUrl(content.artistArt, 440, 440)).then((i) => live && setImg(i));
    return () => {
      live = false;
    };
  }, [content.artistArt]);

  useEffect(() => {
    const el = canvas.current;
    if (!el) return;
    el.width = CARD_SIZES.story.width;
    el.height = CARD_SIZES.story.height;
    let ctx: CanvasRenderingContext2D | null = null;
    try {
      ctx = el.getContext("2d");
    } catch {
      ctx = null;
    }
    if (!ctx) return;
    drawSummaryCard(ctx, layoutSummaryCard(content, canvasMeasure(ctx)), content, img);
  }, [content, img]);

  const save = async () => {
    const el = canvas.current;
    if (!el) return;
    setBusy(true);
    try {
      const blob = await new Promise<Blob | null>((resolve) => el.toBlob(resolve, "image/png"));
      if (!blob) throw new Error("no image");
      const saved = await backend.shareCardSave(new Uint8Array(await blob.arrayBuffer()), `${w.year} Wrapped`);
      if (saved) toast.info(`Saved ${saved}`);
    } catch {
      toast.error("Couldn't save the image");
    } finally {
      setBusy(false);
    }
  };

  const copy = async () => {
    const el = canvas.current;
    const ctx = el?.getContext("2d");
    if (!el || !ctx) return void toast.error(CARD_FAILED);
    setBusy(true);
    try {
      const data = ctx.getImageData(0, 0, el.width, el.height).data;
      await backend.shareCardCopy(new Uint8Array(data.buffer, data.byteOffset, data.byteLength), el.width, el.height);
      toast.info("Image copied");
    } catch {
      toast.error("Couldn't copy the image");
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="wcard wcard--summary">
      <div className="wcard__body wcard__body--split">
        <div className="wcard__words">
          <Kicker>Your year</Kicker>
          <h2 className="wcard__lead">That was your {w.year}.</h2>
          <p className="wcard__sub">Keep it as a picture, or copy it to share. It's made on this computer and nothing is uploaded.</p>
          <div className="wcard__actions">
            <Button variant="primary" icon={<Download size={16} />} disabled={busy} onClick={() => void save()}>
              Save image
            </Button>
            <Button icon={<Copy size={16} />} disabled={busy} onClick={() => void copy()}>
              Copy image
            </Button>
            <Button variant="ghost" icon={<RotateCcw size={16} />} onClick={onRestart}>
              Watch again
            </Button>
          </div>
          {hasUnreleased && (
            <div className="wcard__option">
              <Switch checked={withUnreleased} onChange={setWithUnreleased} label="Include unreleased songs" />
              <span>Include unreleased songs</span>
            </div>
          )}
          <p className="wcard__fine">{hasUnreleased && !withUnreleased ? "Unreleased songs are left off the image." : "1080 by 1920, the size of a story."}</p>
        </div>
        <div className="wcard__preview">
          <canvas ref={canvas} className="wcard__canvas" role="img" aria-label={`Your ${w.year} Wrapped as an image`} data-unreleased={withUnreleased ? "on" : "off"} />
        </div>
      </div>
    </div>
  );
}

function CardBody({ kind, w, onRestart }: { kind: CardKind; w: WrappedData; onRestart: () => void }) {
  switch (kind) {
    case "minutes":
      return <MinutesCard w={w} />;
    case "artists":
      return <ArtistsCard w={w} />;
    case "songs":
      return <SongsCard w={w} />;
    case "albums":
      return <AlbumsCard w={w} />;
    case "personality":
      return <PersonalityCard w={w} />;
    case "day":
      return <DayCard w={w} />;
    case "summary":
      return <SummaryCard w={w} onRestart={onRestart} />;
  }
}

const isControl = (t: EventTarget | null) => t instanceof Element && !!t.closest("button, a, input, label, [role='switch'], canvas");

/** The cards for one year. */
export function WrappedView({ year, onClose }: { year: number; onClose: () => void }) {
  const { data: w, error } = useLoad(() => backend.statsWrapped(year), [year]);
  const cards = useMemo(() => (w ? wrappedCards(w) : []), [w]);
  const [i, setI] = useState(0);
  const [held, setHeld] = useState(false);
  const [stopped, setStopped] = useState(false);
  const reduced = useMemo(() => prefersReducedMotion(), []);
  const last = i >= cards.length - 1;
  const paused = held || stopped || reduced || last || !cards.length;
  const remaining = useRef(CARD_MS);
  const root = useRef<HTMLDivElement>(null);
  const press = useRef<{ timer: number; hold: boolean; x: number } | null>(null);

  const go = useCallback((step: number) => setI((n) => Math.max(0, Math.min(cards.length - 1, n + step))), [cards.length]);

  // A new card starts its full time; leaving one keeps what's left of a paused card.
  useEffect(() => {
    remaining.current = CARD_MS;
  }, [i]);
  useEffect(() => {
    if (paused) return;
    const started = Date.now();
    const t = window.setTimeout(() => go(1), remaining.current);
    return () => {
      window.clearTimeout(t);
      remaining.current = Math.max(0, remaining.current - (Date.now() - started));
    };
  }, [i, paused, go]);

  // Keys: captured before the app's own shortcuts (Space, arrows) see them.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const k = e.key;
      if (k === "Escape") onClose();
      else if (k === "ArrowRight" || k === "PageDown") go(1);
      else if (k === "ArrowLeft" || k === "PageUp") go(-1);
      else if (k === " " && !isControl(e.target)) setStopped((s) => !s);
      else return;
      e.preventDefault();
      e.stopPropagation();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [go, onClose]);

  useEffect(() => {
    const before = document.activeElement as HTMLElement | null;
    root.current?.focus();
    return () => before?.focus?.();
  }, []);

  const onDown = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (e.button !== 0 || isControl(e.target)) return;
    const timer = window.setTimeout(() => {
      if (press.current) press.current.hold = true;
      setHeld(true);
    }, HOLD_MS);
    press.current = { timer, hold: false, x: e.clientX };
  };
  const onUp = (e: ReactPointerEvent<HTMLDivElement>) => {
    const p = press.current;
    press.current = null;
    if (!p) return;
    window.clearTimeout(p.timer);
    if (p.hold) return setHeld(false);
    const box = e.currentTarget.getBoundingClientRect();
    go(p.x - box.left < box.width / 3 ? -1 : 1);
  };
  const onCancel = () => {
    const p = press.current;
    press.current = null;
    if (p) window.clearTimeout(p.timer);
    setHeld(false);
  };

  const tone = yearTone(year);
  const kind = cards[i];
  const style = { "--year-deep": tone.deep, "--year-light": tone.light, "--card-ms": `${CARD_MS}ms` } as CSSProperties;
  return (
    <div
      ref={root}
      className="wrapped"
      role="dialog"
      aria-modal="true"
      aria-label={`${year} Wrapped`}
      tabIndex={-1}
      style={style}
      data-testid="wrapped"
      data-card={kind ?? "none"}
      data-paused={paused ? "true" : "false"}
      data-held={held ? "true" : "false"}
    >
      <div className="wrapped__bar">
        <div className="wrapped__progress" aria-hidden="true">
          {cards.map((c, n) => (
            <span key={c} className={`wrapped__seg ${n < i ? "is-done" : n === i ? "is-current" : ""}`}>
              <i key={n === i ? `on-${i}` : "off"} style={{ animationPlayState: paused ? "paused" : "running" }} />
            </span>
          ))}
        </div>
        <div className="wrapped__top">
          <span className="wrapped__title">{year} Wrapped</span>
          {kind && (
            <span className="wrapped__count" aria-live="polite">
              {i + 1} of {cards.length}
              <span className="sr-only">: {CARD_NAMES[kind]}</span>
            </span>
          )}
          <span className="wrapped__tools">
            {!reduced && !last && cards.length > 0 && (
              <IconButton label={stopped ? "Resume" : "Pause"} size="sm" icon={stopped ? <Play size={16} fill="currentColor" /> : <Pause size={16} fill="currentColor" />} onClick={() => setStopped((s) => !s)} />
            )}
            <IconButton label="Close Wrapped" size="sm" icon={<X size={18} />} onClick={onClose} />
          </span>
        </div>
      </div>
      <div className="wrapped__stage" onPointerDown={onDown} onPointerUp={onUp} onPointerCancel={onCancel} onPointerLeave={onCancel}>
        {error ? (
          <div className="wcard wcard--state">
            <div className="wcard__body">
              <h2 className="wcard__lead">Couldn't put your {year} together.</h2>
              <p className="wcard__sub">{error.message}</p>
            </div>
          </div>
        ) : !w ? (
          <div className="wcard wcard--state" aria-busy="true">
            <div className="wcard__art">
              <YearBanner year={year} w={1440} h={900} />
            </div>
            <div className="wcard__body">
              <p className="wcard__big">
                <span className="wcard__accent">{year}</span>
              </p>
              <p className="wcard__sub">Counting your year…</p>
            </div>
          </div>
        ) : !kind ? (
          <div className="wcard wcard--state">
            <div className="wcard__body">
              <h2 className="wcard__lead">Nothing to look back on in {year} yet.</h2>
              <p className="wcard__sub">Wrapped is made from what you play here and the history you import.</p>
              <div className="wcard__actions">
                <Button
                  onClick={() => {
                    onClose();
                    nav.go({ name: "settings" });
                  }}
                >
                  Import past plays
                </Button>
              </div>
            </div>
          </div>
        ) : (
          <section key={kind} className="wrapped__card" aria-label={`${i + 1} of ${cards.length}: ${CARD_NAMES[kind]}`}>
            <CardBody kind={kind} w={w} onRestart={() => setI(0)} />
          </section>
        )}
      </div>
      {cards.length > 1 && (
        <>
          <button type="button" className="wrapped__nav wrapped__nav--prev" aria-label="Previous card" disabled={i === 0} onClick={() => go(-1)}>
            <ChevronLeft size={20} aria-hidden="true" />
          </button>
          <button type="button" className="wrapped__nav wrapped__nav--next" aria-label="Next card" disabled={last} onClick={() => go(1)}>
            <ChevronRight size={20} aria-hidden="true" />
          </button>
        </>
      )}
    </div>
  );
}

/** Mounted in the shell; shows while a year is open. */
export function WrappedHost() {
  const year = useWrapped((s) => s.year);
  return year == null ? null : <WrappedView key={year} year={year} onClose={wrapped.close} />;
}
