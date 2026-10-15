/**
 * The lyrics editor, mounted once in the shell and opened for one song
 * (stores/lyricsEditor). Step 1: paste or type the words, one lyric per
 * line. Step 2 (optional): sync them to the song. It plays from the start,
 * Space stamps the current line and moves on, Backspace steps back a line
 * and rewinds 3 s, Enter plays from the selected line, Alt+Left/Right nudges
 * it by 0.1 s, and the timeline's marks can be dragged. Saved in the core,
 * keyed by the song; never written next to the music file.
 */
import { Download, Pause, Play, RotateCcw } from "lucide-react";
import { useCallback, useEffect, useId, useMemo, useRef, useState, type KeyboardEvent, type PointerEvent } from "react";
import { Button, IconButton } from "../components/Button";
import { Dialog } from "../components/Dialog";
import { Switch } from "../components/Switch";
import { backend } from "../lib/backend";
import { trackArtistLine } from "../lib/format";
import { clockTenths, isSynced, linesFromText, parseLrc, stampable, textFromLines, toLrc, unsyncedCount, type EditLine } from "../lib/lrc";
import type { Track } from "../lib/types";
import { closeLyricsEditor, ownLyricsChanged, useLyricsEditor } from "../stores/lyricsEditor";
import { player, usePlayer } from "../stores/player";
import { usePageVisible } from "../design/pageVisibility";
import { toast } from "../stores/toasts";
import { interpolate } from "./lyrics";
import {
  clearTimes,
  firstStampable,
  moveTo,
  nextStampable,
  NUDGE_MS,
  nudge,
  prevStampable,
  reconcile,
  stamp,
  STEP_BACK_REWIND_MS,
  timelineFraction,
  timelineWindow,
  type TimelineWindow,
} from "./lyricsSync";
import "./lyrics-editor.css";

type Step = "write" | "sync";

/** The core's own words when it refused the lyrics (`invalid`), otherwise a plain fallback. */
function errorText(e: unknown, fallback: string): string {
  const err = e as { kind?: unknown; message?: unknown } | null;
  return err && err.kind === "invalid" && typeof err.message === "string" && err.message ? err.message : fallback;
}

/** The song's position between the core's updates, read on demand (stamps) and shown about 10 times a second. */
function useSongClock(trackId: string) {
  const isThis = usePlayer((s) => s.state?.track?.id === trackId);
  const playing = usePlayer((s) => s.state?.track?.id === trackId && s.state?.status === "playing");
  const base = useRef({ positionMs: usePlayer.getState().positionMs, at: performance.now() });
  const now = useCallback(() => {
    const st = usePlayer.getState();
    if (st.state?.track?.id !== trackId) return 0;
    return Math.round(interpolate(base.current, performance.now(), st.state?.status === "playing", st.durationMs));
  }, [trackId]);
  const [shown, setShown] = useState(0);
  const visible = usePageVisible();
  useEffect(() => {
    const unsub = usePlayer.subscribe((s, p) => {
      if (s.positionMs !== p.positionMs || s.state?.status !== p.state?.status) {
        base.current = { positionMs: s.positionMs, at: performance.now() };
        setShown(now());
      }
    });
    setShown(now());
    return unsub;
  }, [now]);
  // Ticks only while the song plays and the window can be seen.
  useEffect(() => {
    if (!playing || !visible) return;
    const t = setInterval(() => setShown(now()), 100);
    return () => clearInterval(t);
  }, [now, playing, visible]);
  return { isThis, playing, now, shown };
}

/** Play the song from `ms`: seek when it's the one playing, otherwise start it. */
function playFrom(track: Track, ms: number) {
  const st = usePlayer.getState().state;
  if (st?.track?.id === track.id) {
    void player.seek(ms);
    if (st.status !== "playing") void player.toggle();
  } else {
    void player.playTracks([track.id], track.id);
  }
}

export function LyricsEditorHost() {
  const track = useLyricsEditor((s) => s.track);
  return track ? <LyricsEditor key={track.id} track={track} /> : null;
}

function LyricsEditor({ track }: { track: Track }) {
  const [loaded, setLoaded] = useState(false);
  const [had, setHad] = useState(false);
  const [text, setText] = useState("");
  const [lines, setLines] = useState<EditLine[]>([]);
  const [prefer, setPrefer] = useState(false);
  const [step, setStep] = useState<Step>("write");
  const [confirm, setConfirm] = useState<"discard" | "remove" | null>(null);
  const [busy, setBusy] = useState(false);
  const baseline = useRef("|false");
  const textArea = useRef<HTMLTextAreaElement>(null);
  const preferId = useId();

  useEffect(() => {
    let alive = true;
    backend.ownLyrics(track.id).then(
      (own) => {
        if (!alive) return;
        if (own) {
          const ls = parseLrc(own.text);
          setLines(ls);
          setText(textFromLines(ls));
          setPrefer(own.prefer);
          setHad(true);
          baseline.current = `${toLrc(ls)}|${own.prefer}`;
        }
        setLoaded(true);
      },
      () => alive && setLoaded(true),
    );
    return () => {
      alive = false;
    };
  }, [track.id]);

  const lrc = useMemo(() => toLrc(lines), [lines]);
  const dirty = loaded && `${lrc}|${prefer}` !== baseline.current;
  const hasWords = lines.some(stampable);
  const missing = unsyncedCount(lines);
  const canSave = loaded && hasWords && missing === 0 && !busy;

  const onText = (v: string) => {
    setText(v);
    setLines((old) => reconcile(old, linesFromText(v)));
    setConfirm(null);
  };

  const cancel = () => {
    if (busy) return;
    if (dirty && confirm !== "discard") setConfirm("discard");
    else closeLyricsEditor();
  };

  const save = async () => {
    if (!canSave) return;
    setBusy(true);
    try {
      await backend.ownLyricsSave(track.id, lrc, prefer);
      ownLyricsChanged(track.id, true);
      toast.info("Lyrics saved");
      closeLyricsEditor();
    } catch (e) {
      toast.error(errorText(e, "Couldn't save the lyrics"));
      setBusy(false);
    }
  };

  const remove = async () => {
    setBusy(true);
    try {
      await backend.ownLyricsDelete(track.id);
      ownLyricsChanged(track.id, false);
      toast.info("Your lyrics were removed");
      closeLyricsEditor();
    } catch (e) {
      toast.error(errorText(e, "Couldn't remove the lyrics"));
      setBusy(false);
    }
  };

  const exportLrc = async () => {
    try {
      const name = await backend.lyricsExport(lrc, `${trackArtistLine(track)} - ${track.title}`);
      if (name) toast.info(`Saved ${name}`);
    } catch (e) {
      toast.error(errorText(e, "Couldn't export the lyrics"));
    }
  };

  const toSync = () => {
    if (!hasWords) return;
    setConfirm(null);
    setStep("sync");
    // The song plays from the start, ready for the first stamp.
    playFrom(track, 0);
  };

  const title = had ? "Edit lyrics" : "Add lyrics";

  return (
    <Dialog open onClose={cancel} label={`${title}: ${track.title}`} className="lyr-ed" initialFocus={textArea}>
      <header className="lyr-ed__head">
        <div className="lyr-ed__heading">
          <h2 className="lyr-ed__title">{title}</h2>
          <p className="lyr-ed__song truncate">
            {track.title} · {trackArtistLine(track)}
          </p>
        </div>
        <ol className="lyr-ed__steps" aria-label="Steps">
          <li aria-current={step === "write" ? "step" : undefined} className={step === "write" ? "is-on" : ""}>
            <span className="lyr-ed__step-n">1</span> Write
          </li>
          <li aria-current={step === "sync" ? "step" : undefined} className={step === "sync" ? "is-on" : ""}>
            <span className="lyr-ed__step-n">2</span> Sync <span className="lyr-ed__optional">(optional)</span>
          </li>
        </ol>
      </header>

      {step === "write" ? (
        <div className="lyr-ed__body">
          <textarea
            ref={textArea}
            className="lyr-ed__text"
            aria-label="Lyrics"
            placeholder={"Paste or type the lyrics, one line per lyric line.\nLeave a blank line between verses."}
            value={text}
            disabled={!loaded}
            spellCheck
            onChange={(e) => onText(e.target.value)}
          />
          <p className="lyr-ed__hint">
            {!hasWords
              ? "Plain lyrics are fine to save as they are. Sync them to make them follow the song."
              : isSynced(lines)
                ? `${lines.filter(stampable).length} lines, synced. Lines you change lose their time.`
                : `${lines.filter(stampable).length} lines${missing ? `, ${missing} still without a time` : ""}.`}
          </p>
        </div>
      ) : (
        <SyncStep track={track} lines={lines} setLines={setLines} />
      )}

      <div className="lyr-ed__prefer">
        <div>
          <div id={preferId} className="lyr-ed__prefer-title">
            Show these instead of Plex's lyrics
          </div>
          <div className="lyr-ed__prefer-desc">For this song. Your lyrics always show when Plex has none.</div>
        </div>
        <Switch checked={prefer} onChange={setPrefer} labelledBy={preferId} />
      </div>

      {confirm ? (
        <div className="lyr-ed__confirm" role="alert">
          <span>{confirm === "discard" ? "Discard your changes?" : "Remove your lyrics for this song?"}</span>
          <div className="lyr-ed__actions">
            <Button variant="ghost" onClick={() => setConfirm(null)}>
              Keep editing
            </Button>
            {confirm === "discard" ? (
              <Button variant="primary" onClick={closeLyricsEditor}>
                Discard
              </Button>
            ) : (
              <Button variant="primary" disabled={busy} onClick={() => void remove()}>
                Remove
              </Button>
            )}
          </div>
        </div>
      ) : (
        <footer className="lyr-ed__foot">
          <div className="lyr-ed__actions">
            <Button variant="ghost" icon={<Download size={16} />} disabled={!hasWords || missing > 0} onClick={() => void exportLrc()}>
              Export .lrc
            </Button>
            {had && (
              <Button variant="ghost" onClick={() => setConfirm("remove")}>
                Remove
              </Button>
            )}
          </div>
          <div className="lyr-ed__actions">
            {missing > 0 && step === "sync" && (
              <span className="lyr-ed__missing">
                {missing === 1 ? "1 line" : `${missing} lines`} still need a time
              </span>
            )}
            <Button variant="ghost" onClick={cancel}>
              Cancel
            </Button>
            {step === "write" ? (
              <Button variant="secondary" disabled={!hasWords} onClick={toSync}>
                Sync to the song
              </Button>
            ) : (
              <Button variant="secondary" onClick={() => setStep("write")}>
                Back to the words
              </Button>
            )}
            <Button variant="primary" disabled={!canSave} onClick={() => void save()}>
              Save
            </Button>
          </div>
        </footer>
      )}
    </Dialog>
  );
}

// ---------------------------------------------------------------- sync step

const KEYS = "Space: this line starts now · Backspace: back a line · Enter: play from the selected line · Alt+Left/Right: nudge 0.1 s";

function SyncStep({ track, lines, setLines }: { track: Track; lines: EditLine[]; setLines: (f: (l: EditLine[]) => EditLine[]) => void }) {
  const { isThis, playing, now, shown } = useSongClock(track.id);
  const durationMs = track.durationMs;
  // The line Space stamps next (-1: every line done) and the one nudges and Enter act on.
  const [cursor, setCursor] = useState(() => {
    const firstOpen = lines.findIndex((l) => stampable(l) && l.startMs === null);
    return firstOpen >= 0 ? firstOpen : firstStampable(lines);
  });
  const [selected, setSelected] = useState(-1);
  const list = useRef<HTMLDivElement>(null);
  const rowEls = useRef<(HTMLElement | null)[]>([]);
  const listId = useId();

  useEffect(() => {
    list.current?.focus();
  }, []);

  // Keep the line being worked on in view.
  const active = cursor >= 0 ? cursor : selected;
  useEffect(() => {
    const el = rowEls.current[active];
    if (el && typeof el.scrollIntoView === "function") el.scrollIntoView({ block: "nearest" });
  }, [active]);

  const select = (i: number) => {
    setSelected(i);
    setCursor(i);
  };

  const onKey = (e: KeyboardEvent) => {
    const k = e.key;
    let handled = true;
    if (k === " " && !e.repeat) {
      if (cursor >= 0 && isThis) {
        const at = now();
        setLines((ls) => stamp(ls, cursor, at));
        setSelected(cursor);
        setCursor(nextStampable(lines, cursor));
      } else if (!isThis) {
        playFrom(track, 0);
      }
    } else if (k === "Backspace") {
      const from = cursor >= 0 ? cursor : lines.length;
      const back = prevStampable(lines, from);
      if (back >= 0) select(back);
      if (isThis) void player.seek(Math.max(0, now() - STEP_BACK_REWIND_MS));
    } else if (k === "Enter") {
      const i = selected >= 0 ? selected : cursor;
      const t = lines[i]?.startMs;
      if (t !== null && t !== undefined) {
        select(i);
        playFrom(track, t);
      }
    } else if ((k === "ArrowLeft" || k === "ArrowRight") && e.altKey) {
      if (selected >= 0) setLines((ls) => nudge(ls, selected, k === "ArrowRight" ? NUDGE_MS : -NUDGE_MS, durationMs));
    } else if ((k === "ArrowUp" || k === "ArrowDown") && !e.altKey) {
      const from = active >= 0 ? active : k === "ArrowDown" ? -1 : lines.length;
      const to = k === "ArrowDown" ? nextStampable(lines, from) : prevStampable(lines, from);
      if (to >= 0) select(to);
    } else {
      handled = false;
    }
    if (handled) {
      e.preventDefault();
      e.stopPropagation();
    }
  };

  const optionId = (i: number) => `${listId}-l${i}`;

  return (
    <div className="lyr-ed__body lyr-ed__body--sync">
      <div className="lyr-ed__transport">
        <IconButton
          label={playing ? "Pause" : "Play"}
          icon={playing ? <Pause size={18} /> : <Play size={18} />}
          variant="filled"
          onClick={() => (isThis ? void player.toggle() : playFrom(track, 0))}
        />
        <IconButton label="Play from the start" icon={<RotateCcw size={16} />} onClick={() => playFrom(track, 0)} />
        <span className="lyr-ed__clock" aria-label="Song position">
          {isThis ? clockTenths(shown) : "Not playing"}
        </span>
        <Button variant="ghost" size="sm" onClick={() => setLines(clearTimes)}>
          Clear times
        </Button>
      </div>
      <p className="lyr-ed__keys">{KEYS}</p>

      <div
        ref={list}
        className="lyr-ed__lines"
        role="listbox"
        aria-label="Lines"
        tabIndex={0}
        aria-activedescendant={active >= 0 ? optionId(active) : undefined}
        onKeyDown={onKey}
      >
        {lines.map((l, i) =>
          stampable(l) ? (
            <div
              key={i}
              id={optionId(i)}
              ref={(el) => {
                rowEls.current[i] = el;
              }}
              role="option"
              aria-selected={i === selected}
              className={`lyr-ed__line ${i === cursor ? "is-next" : ""} ${i === selected ? "is-selected" : ""}`}
              onClick={() => {
                select(i);
                list.current?.focus();
              }}
            >
              <span className="lyr-ed__time">{l.startMs === null ? "--:--.-" : clockTenths(l.startMs)}</span>
              <span className="lyr-ed__words">{l.text}</span>
            </div>
          ) : (
            <div key={i} className="lyr-ed__break" aria-hidden="true" />
          ),
        )}
      </div>

      <Timeline
        lines={lines}
        durationMs={durationMs}
        positionMs={isThis ? shown : null}
        selected={selected}
        onSelect={select}
        onMove={(i, ms) => setLines((ls) => moveTo(ls, i, ms, durationMs))}
        onSeek={(ms) => isThis && void player.seek(ms)}
      />
    </div>
  );
}

// ---------------------------------------------------------------- timeline

/**
 * Line starts as marks along a 20 s stretch of the song around the selected
 * line (or the playhead); drag a mark to move its line, click the bar to seek.
 */
function Timeline({
  lines,
  durationMs,
  positionMs,
  selected,
  onSelect,
  onMove,
  onSeek,
}: {
  lines: EditLine[];
  durationMs: number;
  positionMs: number | null;
  selected: number;
  onSelect: (i: number) => void;
  onMove: (i: number, ms: number) => void;
  onSeek: (ms: number) => void;
}) {
  const bar = useRef<HTMLDivElement>(null);
  const drag = useRef<number | null>(null);
  // While a mark is dragged the window holds still under the pointer.
  const [frozen, setFrozen] = useState<TimelineWindow | null>(null);
  const centre = (selected >= 0 ? lines[selected]?.startMs : null) ?? positionMs ?? 0;
  const win = frozen ?? timelineWindow(centre, durationMs);
  const at = (ms: number) => `${timelineFraction(ms - win.start, win.span) * 100}%`;
  const inView = (ms: number) => ms >= win.start && ms <= win.start + win.span;

  const msAt = (clientX: number) => {
    const r = bar.current?.getBoundingClientRect();
    if (!r || r.width <= 0) return win.start;
    return Math.round(win.start + Math.min(1, Math.max(0, (clientX - r.left) / r.width)) * win.span);
  };

  const down = (i: number) => (e: PointerEvent) => {
    e.stopPropagation();
    e.preventDefault();
    drag.current = i;
    setFrozen(win);
    onSelect(i);
    (e.currentTarget as Element).setPointerCapture?.(e.pointerId);
  };
  const move = (e: PointerEvent) => {
    if (drag.current !== null) onMove(drag.current, msAt(e.clientX));
  };
  const up = (e: PointerEvent) => {
    if (drag.current === null) return;
    (e.currentTarget as Element).releasePointerCapture?.(e.pointerId);
    drag.current = null;
    setFrozen(null);
  };

  return (
    <div className="lyr-tl">
      <div
        ref={bar}
        className="lyr-tl__bar"
        onPointerDown={(e) => onSeek(msAt(e.clientX))}
        data-testid="lyrics-timeline"
        data-start={win.start}
        data-span={win.span}
      >
        {positionMs !== null && inView(positionMs) && <span className="lyr-tl__head" style={{ left: at(positionMs) }} aria-hidden="true" />}
        {lines.map((l, i) =>
          l.startMs === null || !inView(l.startMs) ? null : (
            <span
              key={i}
              role="slider"
              tabIndex={-1}
              aria-label={`Start of "${l.text}"`}
              aria-valuemin={0}
              aria-valuemax={durationMs}
              aria-valuenow={l.startMs}
              aria-valuetext={clockTenths(l.startMs)}
              className={`lyr-tl__mark ${i === selected ? "is-selected" : ""}`}
              style={{ left: at(l.startMs) }}
              onPointerDown={down(i)}
              onPointerMove={move}
              onPointerUp={up}
              onPointerCancel={up}
            />
          ),
        )}
      </div>
      <div className="lyr-tl__ends" aria-hidden="true">
        <span>{clockTenths(win.start)}</span>
        <span>{clockTenths(win.start + win.span)}</span>
      </div>
    </div>
  );
}
