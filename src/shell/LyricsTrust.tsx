/**
 * Lyrics you can trust, in the lyrics view: whether the lyrics were checked
 * against the song, the timing nudge (−/+ 0.5 s, the - and + keys, "Tap to
 * sync"), "Wrong lyrics?" and its picker, what the view says when nothing is
 * shown (released lyrics held back, checking, nothing matched, none
 * chosen), and the offer to download the speech model.
 */
import { AlertTriangle, AudioLines, Check, CircleHelp, Clock, Globe, Hand, Loader2, Minus, MoreHorizontal, PenLine, Plus, X } from "lucide-react";
import { useCallback, useEffect, useRef, useState, type MouseEvent } from "react";
import { openContextMenu } from "../components/ContextMenu";
import { Button } from "../components/Button";
import { Dialog } from "../components/Dialog";
import { backend } from "../lib/backend";
import { formatDuration, trackArtistLine } from "../lib/format";
import type { LyricLine, Lyrics, LyricsCandidate, LyricsCandidates, LyricsCheck, LyricsCheckStatus, LyricsVerdict, Track } from "../lib/types";
import { openLyricsEditor } from "../stores/lyricsEditor";
import { usePlayer } from "../stores/player";
import { useSettings, updateSettings } from "../stores/settings";
import { toast } from "../stores/toasts";
import type { MenuItem } from "../stores/ui";
import { modelSize, transcribe, useTranscribe } from "../stores/transcribe";
import {
  chooseLyrics,
  closeWrongLyrics,
  NUDGE_MS,
  openWrongLyrics,
  setLyricsOffset,
  setTapping,
  useWrongLyrics,
} from "../stores/wrongLyrics";
import { loadCheckStatus, lyricsChanged, retryCheck, useLyricsTrust } from "./lyrics";
import { ShareTranscription } from "./ShareTranscription";
import "./lyrics-trust.css";

/** The small note on what the check said. */
export const CHECK_TEXT = {
  checked: "Checked",
  notChecked: "Not checked",
  checking: "Checking against the song…",
  waiting: "Waiting to check",
  stuck: "Couldn't check yet",
} as const;

export const CHECK_TITLE = {
  checked: "These lyrics were compared with the song on this computer and match it.",
  notChecked: "These lyrics weren't compared with the song, so they may be for another version or another song.",
  checking: "Comparing these lyrics with the song on this computer.",
  waiting: "The check hasn't finished yet. It runs when the computer gets to it.",
  stuck: "The check hasn't finished. Try again to ask for it again.",
} as const;

/** How long the spinner shows before the note says the check is waiting. */
export const CHECK_WAIT_MS = 30_000;
/** How long without a result before the note offers Try again. */
export const CHECK_STUCK_MS = 120_000;

export type CheckPhase = LyricsCheck | "waiting" | "stuck";

/** When each song's check was first seen going (kept across views and reloads of its lyrics). */
const checkSince = new Map<string, number>();

/** Forget when checks started (tests). */
export function resetCheckClocks() {
  checkSince.clear();
}

/**
 * How long a check has gone on: still going after CHECK_WAIT_MS is
 * "waiting" (the spinner stops), after CHECK_STUCK_MS "stuck" (Try again,
 * when the core hasn't said where the check is). The clock starts when the
 * song's check is first seen going (or `retry` asks again) and stops with
 * any other status.
 */
export function useCheckPhase(trackId: string, check: LyricsCheck | undefined): { phase: CheckPhase | undefined; retry: () => void } {
  const [, setTick] = useState(0);
  const going = check === "checking";
  if (going && !checkSince.has(trackId)) checkSince.set(trackId, Date.now());
  const at = going ? checkSince.get(trackId)! : 0;
  useEffect(() => {
    if (!going) {
      checkSince.delete(trackId);
      return;
    }
    const timers = [CHECK_WAIT_MS, CHECK_STUCK_MS].map((ms) => {
      const left = at + ms - Date.now();
      return left > 0 ? setTimeout(() => setTick((n) => n + 1), left) : null;
    });
    return () => timers.forEach((t) => t && clearTimeout(t));
  }, [trackId, going, at]);
  const retry = useCallback(() => {
    checkSince.set(trackId, Date.now());
    setTick((n) => n + 1);
    retryCheck(trackId);
  }, [trackId]);
  if (!going) return { phase: check, retry };
  const took = Date.now() - at;
  return { phase: took >= CHECK_STUCK_MS ? "stuck" : took >= CHECK_WAIT_MS ? "waiting" : "checking", retry };
}

export const NOTICE_TEXT = {
  versionHeld: "Lyrics from the released version, may not match",
  checking: "Checking lyrics against the song…",
  noneMatched: "None of the lyrics found match this song",
  chosenNone: "You turned lyrics off for this song",
} as const;

/** What a check in progress is doing, in words; `stuck` when it couldn't run and Try again helps. */
export function checkProgress(s: LyricsCheckStatus | undefined): { text: string; spin: boolean; retry: boolean } | null {
  if (!s || s.state === "done") return null;
  switch (s.state) {
    case "queued":
      return { text: "Waiting to check against the song…", spin: true, retry: false };
    case "reading":
      return { text: "Reading the song to check the lyrics…", spin: true, retry: false };
    case "listening":
      return { text: "Checking against the song…", spin: true, retry: false };
    case "waiting":
      return { text: "Waiting: your computer is busy", spin: false, retry: false };
    case "failed":
      return s.reason === "noModel"
        ? { text: "Can't check without the speech model", spin: false, retry: false }
        : { text: "Couldn't check yet", spin: false, retry: true };
  }
}

/** The song's check status while its lyrics say checking. */
function useCheckProgress(trackId: string, checking: boolean) {
  const s = useLyricsTrust((st) => st.checks[trackId]);
  useEffect(() => {
    if (checking) void loadCheckStatus(trackId);
  }, [checking, trackId]);
  return checking ? checkProgress(s) : null;
}

/** "+1.5 s", "−0.5 s". */
export function offsetText(ms: number): string {
  const s = (Math.abs(ms) / 1000).toFixed(1);
  return `${ms < 0 ? "−" : "+"}${s} s`;
}

/** The offset that makes `line` start where playback is now. */
export function tapOffset(line: Pick<LyricLine, "startMs">, positionMs: number, currentOffsetMs: number): number {
  return Math.round((currentOffsetMs + positionMs - line.startMs) / 100) * 100;
}

// ---------------------------------------------------------------- the toolbar

/** The speech model can be downloaded so lyrics can be checked. */
function useModelOffer(): { show: boolean; size: string; download: () => void } {
  const check = useSettings((s) => s.settings?.lyricsCheckAudio ?? true);
  const status = useTranscribe((s) => s.status);
  const model = status?.models.find((m) => m.kind === status.model) ?? status?.models[0];
  const installed = !!status?.models.some((m) => m.installed);
  const show = check && !!status?.available && !installed && !!model && !model.downloading;
  return { show, size: model ? modelSize(model.sizeBytes) : "", download: () => model && void transcribe.download(model.kind) };
}

const OFFER_KEY = "platter.lyricsModelOfferHidden";

function readHidden(): boolean {
  try {
    return localStorage.getItem(OFFER_KEY) === "1";
  } catch {
    return false;
  }
}

/** "Download the speech model (about N MB) so Platter can check lyrics". */
export function CheckModelOffer({ compact = false }: { compact?: boolean }) {
  const offer = useModelOffer();
  const [hidden, setHidden] = useState(readHidden);
  if (!offer.show || (hidden && compact)) return null;
  const hide = () => {
    setHidden(true);
    try {
      localStorage.setItem(OFFER_KEY, "1");
    } catch {
      // Hidden for now only.
    }
  };
  return (
    <div className="lyrics-trust__offer" role="note">
      <p>
        Download the speech model (about {offer.size}) so Platter can check lyrics against the song. It runs on this computer;
        the song never leaves it.
      </p>
      <div className="lyrics-trust__offer-actions">
        <Button size="sm" variant="primary" icon={<AudioLines size={14} />} onClick={offer.download}>
          Download the speech model
        </Button>
        {compact && (
          <Button size="sm" variant="ghost" onClick={hide}>
            Not now
          </Button>
        )}
      </div>
    </div>
  );
}

/** The quiet note on what the check said, with a small icon. */
function CheckStatus({ lyrics, track }: { lyrics: Lyrics; track: Track }) {
  const { phase: c, retry } = useCheckPhase(track.id, lyrics.check);
  const p = useCheckProgress(track.id, lyrics.check === "checking");
  if (p) {
    // Where the core says the check is; the spinner stops after CHECK_WAIT_MS.
    const spin = p.spin && c === "checking";
    return (
      <span className="lyrics-trust__status">
        <span className={`lyrics-trust__check is-${p.retry ? "stuck" : spin ? "checking" : "waiting"}`} role="status">
          {spin ? (
            <Loader2 size={14} aria-hidden="true" className="lyrics-trust__spin" />
          ) : p.retry ? (
            <CircleHelp size={14} aria-hidden="true" />
          ) : (
            <Clock size={14} aria-hidden="true" />
          )}
          <span className="lyrics-trust__check-text">{p.text}</span>
        </span>
        {p.retry && (
          <button type="button" className="lyrics-trust__retry" onClick={retry}>
            Try again
          </button>
        )}
        {lyrics.chosen && <span className="lyrics-trust__pick">Your pick</span>}
      </span>
    );
  }
  if (!c && !lyrics.chosen) return null;
  const icon =
    c === "checked" ? (
      <Check size={14} aria-hidden="true" />
    ) : c === "checking" ? (
      <Loader2 size={14} aria-hidden="true" className="lyrics-trust__spin" />
    ) : c === "waiting" ? (
      <Clock size={14} aria-hidden="true" />
    ) : (
      <CircleHelp size={14} aria-hidden="true" />
    );
  const extra = c === "checked" && (lyrics.retimed || lyrics.autoOffsetMs) ? ", timing fixed" : "";
  return (
    <span className="lyrics-trust__status">
      {c && (
        <span className={`lyrics-trust__check is-${c}`} title={CHECK_TITLE[c]} role="status">
          {icon}
          <span className="lyrics-trust__check-text">
            {CHECK_TEXT[c]}
            {extra}
          </span>
        </span>
      )}
      {c === "stuck" && (
        <button type="button" className="lyrics-trust__retry" onClick={retry}>
          Try again
        </button>
      )}
      {lyrics.chosen && <span className="lyrics-trust__pick">Your pick</span>}
    </span>
  );
}

/** −, the offset (click to reset) and +, in one control. The - and + keys nudge too. */
function TimingStepper({ lyrics, track }: { lyrics: Lyrics; track: Track }) {
  const offset = lyrics.offsetMs ?? 0;
  const pickerOpen = useWrongLyrics((s) => !!s.track);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || e.ctrlKey || e.metaKey || e.altKey || pickerOpen) return;
      const t = e.target as HTMLElement | null;
      if (t?.closest?.("input, textarea, select, [contenteditable=true], .dialog")) return;
      const step = e.key === "-" || e.key === "_" ? -NUDGE_MS : e.key === "+" || e.key === "=" ? NUDGE_MS : 0;
      if (!step) return;
      e.preventDefault();
      void setLyricsOffset(track.id, offset + step);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [track.id, offset, pickerOpen]);
  // A new song ends tapping.
  useEffect(() => () => setTapping(null), [track.id]);
  return (
    <span className="lyrics-trust__stepper" role="group" aria-label="Lyrics timing">
      <button
        type="button"
        className="lyrics-trust__step"
        aria-label="Show lyrics 0.5 seconds earlier"
        title="Earlier (−)"
        onClick={() => void setLyricsOffset(track.id, offset - NUDGE_MS)}
      >
        <Minus size={14} aria-hidden="true" />
      </button>
      {offset !== 0 ? (
        <button
          type="button"
          className="lyrics-trust__offset is-moved"
          aria-label={`Timing moved ${offsetText(offset)}. Reset`}
          title="Reset the timing"
          onClick={() => void setLyricsOffset(track.id, 0)}
        >
          {offsetText(offset)}
        </button>
      ) : (
        <span className="lyrics-trust__offset" title="Timing as found">
          0 s
        </span>
      )}
      <button
        type="button"
        className="lyrics-trust__step"
        aria-label="Show lyrics 0.5 seconds later"
        title="Later (+)"
        onClick={() => void setLyricsOffset(track.id, offset + NUDGE_MS)}
      >
        <Plus size={14} aria-hidden="true" />
      </button>
    </span>
  );
}

/** Hidden by the toolbar's width (collapsed into "More lyrics tools"). */
function collapsed(el: HTMLElement | null): boolean {
  return !!el && el.offsetParent === null;
}

/**
 * Under the lyrics, in its own row: the check note, the timing stepper
 * (synced lyrics), "Tap to sync", "Wrong lyrics?" (made plain when other
 * lyrics found disagree) and sharing a transcription. When the row gets
 * narrow, sharing, then "Tap to sync", then "Wrong lyrics?" move into a
 * "More lyrics tools" menu instead of wrapping.
 */
export function LyricsTools({ lyrics, track, synced }: { lyrics: Lyrics; track: Track; synced: boolean }) {
  const tapping = useWrongLyrics((s) => s.tapping === track.id);
  const tapRef = useRef<HTMLButtonElement>(null);
  const wrongRef = useRef<HTMLButtonElement>(null);
  const shareRef = useRef<HTMLSpanElement>(null);
  const toggleTap = () => setTapping(tapping ? null : track.id);
  const more = (e: MouseEvent<HTMLButtonElement>) => {
    const items: MenuItem[] = [];
    const share = shareRef.current?.querySelector("button") ?? null;
    if (collapsed(share)) items.push({ label: share!.textContent ?? "", icon: Globe, onSelect: () => share!.click() });
    if (collapsed(tapRef.current)) items.push({ label: "Tap to sync", icon: Hand, checked: tapping, onSelect: toggleTap });
    if (collapsed(wrongRef.current)) items.push({ label: "Wrong lyrics?", icon: AlertTriangle, onSelect: () => openWrongLyrics(track) });
    if (items.length) openContextMenu("More lyrics tools", items, e.currentTarget);
  };
  return (
    <div className="lyrics-trust" role="toolbar" aria-label="Lyrics tools">
      {lyrics.check === "notChecked" && <CheckModelOffer compact />}
      {lyrics.disagree && <p className="lyrics-trust__hint">Other lyrics found for this song say something different.</p>}
      {tapping && (
        <p className="lyrics-trust__hint" role="status">
          Click a line as it's sung.{" "}
          <button type="button" className="lyrics__source-edit" onClick={() => setTapping(null)}>
            Done
          </button>
        </p>
      )}
      <div className="lyrics-trust__bar">
        <CheckStatus lyrics={lyrics} track={track} />
        <span className="lyrics-trust__actions">
          {synced && <TimingStepper lyrics={lyrics} track={track} />}
          {synced && (
            <button
              ref={tapRef}
              type="button"
              className={`lyrics-trust__btn lyrics-trust__tap ${tapping ? "is-on" : ""}`}
              aria-pressed={tapping}
              title="Click a line as it's sung to line the lyrics up there"
              onClick={toggleTap}
            >
              Tap to sync
            </button>
          )}
          <button
            ref={wrongRef}
            type="button"
            className={`lyrics-trust__btn lyrics-trust__wrong ${lyrics.disagree ? "is-prominent" : ""}`}
            onClick={() => openWrongLyrics(track)}
          >
            {lyrics.disagree && <AlertTriangle size={14} aria-hidden="true" />}
            Wrong lyrics?
          </button>
          <span ref={shareRef} className="lyrics-trust__share">
            <ShareTranscription lyrics={lyrics} track={track} />
          </span>
          <button
            type="button"
            className={`lyrics-trust__btn lyrics-trust__more ${synced ? "has-tap" : ""} ${lyrics.disagree ? "" : "has-wrong"}`}
            aria-label="More lyrics tools"
            title="More"
            onClick={more}
          >
            <MoreHorizontal size={16} aria-hidden="true" />
          </button>
        </span>
      </div>
    </div>
  );
}

/** A line clicked while tapping: set the offset so it starts now. True when it was used. */
export function tapLine(trackId: string, line: LyricLine, lyrics: Lyrics): boolean {
  if (useWrongLyrics.getState().tapping !== trackId) return false;
  const pos = usePlayer.getState().positionMs;
  void setLyricsOffset(trackId, tapOffset(line, pos, lyrics.offsetMs ?? 0));
  setTapping(null);
  return true;
}

// ---------------------------------------------------------------- nothing shown

/** What the lyrics view says instead of lyrics, and what can be done. */
export function LyricsNotice({ info, track }: { info: Lyrics; track: Track }) {
  const n = info.notice;
  const p = useCheckProgress(track.id, n === "checking");
  if (!n) return null;
  return (
    <div className="lyrics-page__empty-wrap lyrics-trust__notice">
      <p className="lyrics-page__empty">{NOTICE_TEXT[n]}</p>
      {n === "versionHeld" && (
        <p className="lyrics-trust__notice-note">
          This is another version of the song (unreleased, alternate, live or tagged with a version), so its words may differ.
        </p>
      )}
      {n === "checking" &&
        (p ? (
          <p className="lyrics-trust__notice-note" role="status">
            {p.spin && <Loader2 size={14} aria-hidden="true" className="lyrics-trust__spin" />} {p.text}
          </p>
        ) : (
          <p className="lyrics-trust__notice-note">
            <Loader2 size={14} aria-hidden="true" className="lyrics-trust__spin" /> The lyrics found are being compared with the song on
            this computer.
          </p>
        ))}
      <div className="lyrics-trust__notice-actions">
        {n === "versionHeld" && info.pick && (
          <Button variant="primary" onClick={() => void chooseLyrics(track.id, info.pick!)}>
            Use anyway
          </Button>
        )}
        {n === "chosenNone" && (
          <Button variant="primary" onClick={() => void chooseLyrics(track.id, null)}>
            Show lyrics again
          </Button>
        )}
        {p?.retry && (
          <Button variant="primary" onClick={() => retryCheck(track.id)}>
            Try again
          </Button>
        )}
        <Button variant={n === "versionHeld" || n === "chosenNone" ? "ghost" : "primary"} onClick={() => openWrongLyrics(track)}>
          Wrong lyrics?
        </Button>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------- the picker

const SOURCE_TEXT: Record<LyricsCandidate["origin"], string> = {
  user: "Your lyrics",
  server: "From your server",
  lrclibExact: "LRCLIB",
  lrclibSearch: "LRCLIB, found by title",
  auto: "Transcribed on this computer",
};

export const VERDICT_TEXT: Record<LyricsVerdict, string> = {
  match: "Matches the song",
  matchOffset: "Matches the song, timing moved",
  noMatch: "Doesn't match the song",
  cantTell: "Couldn't tell",
};

function sourceText(c: LyricsCandidate): string {
  if (c.origin === "server" && c.source === "file") return "Lyrics file next to the song";
  return SOURCE_TEXT[c.origin];
}

function CandidateRow({ c, selected, onPick }: { c: LyricsCandidate; selected: boolean; onPick: () => void }) {
  const meta = [sourceText(c), c.album, c.durationMs ? formatDuration(c.durationMs) : null, c.synced ? "Synced" : "Not synced"].filter(
    Boolean,
  );
  const verdict = c.check ? VERDICT_TEXT[c.check] : c.origin === "user" || c.origin === "auto" ? null : "Not checked";
  return (
    <li>
      <button
        type="button"
        role="radio"
        aria-checked={selected}
        className={`wrong-lyrics__row ${selected ? "is-selected" : ""}`}
        onClick={onPick}
      >
        <span className="wrong-lyrics__radio" aria-hidden="true" />
        <span className="wrong-lyrics__body">
          <span className="wrong-lyrics__preview">{c.preview[0] ?? "(no words)"}</span>
          {c.preview[1] && <span className="wrong-lyrics__preview2">{c.preview[1]}</span>}
          <span className="wrong-lyrics__meta">{meta.join(" · ")}</span>
          <span className="wrong-lyrics__tags">
            {verdict && <span className={`wrong-lyrics__tag is-${c.check ?? "unchecked"}`}>{verdict}</span>}
            {c.otherVersion && <span className="wrong-lyrics__tag">Released version</span>}
          </span>
        </span>
      </button>
    </li>
  );
}

/** "Wrong lyrics?": every set found, transcribe, write or paste, or none. */
export function WrongLyricsDialog() {
  const track = useWrongLyrics((s) => s.track);
  const trust = useLyricsTrust((s) => s.version);
  const transcribeOn = useSettings((s) => s.settings?.transcribeLyrics ?? false);
  const [list, setList] = useState<{ id: string; c: LyricsCandidates } | null>(null);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    if (!track) return;
    let alive = true;
    setFailed(false);
    backend.lyricsCandidates(track.id).then(
      (c) => alive && setList({ id: track.id, c }),
      () => alive && setFailed(true),
    );
    return () => {
      alive = false;
    };
  }, [track, trust]);
  if (!track) return null;
  const c = list?.id === track.id ? list.c : null;
  const shown = c?.items.find((i) => i.shown)?.key;
  const selected = c?.pick ?? shown;
  const pick = async (key: string | null, msg?: string) => {
    if (await chooseLyrics(track.id, key)) {
      closeWrongLyrics();
      if (msg) toast.info(msg);
    }
  };
  const transcribeNow = async () => {
    if (!transcribeOn) await updateSettings({ transcribeLyrics: true });
    await transcribe.track(track.id);
    await chooseLyrics(track.id, "auto");
    closeWrongLyrics();
  };
  return (
    <Dialog open onClose={closeWrongLyrics} label="Wrong lyrics?" className="wrong-lyrics">
      <header className="wrong-lyrics__head">
        <h2 className="wrong-lyrics__title">Wrong lyrics?</h2>
        <p className="wrong-lyrics__song truncate">
          {track.title} · {trackArtistLine(track)}
        </p>
      </header>
      {failed && <p className="wrong-lyrics__empty">The lyrics found couldn't be listed. Try again.</p>}
      {!c && !failed && <p className="wrong-lyrics__empty">Looking for lyrics…</p>}
      {c && (
        <>
          {c.items.length ? (
            <ul className="wrong-lyrics__list" role="radiogroup" aria-label="Lyrics found for this song">
              {c.items.map((i) => (
                <CandidateRow key={i.key} c={i} selected={selected === i.key} onPick={() => void pick(i.key)} />
              ))}
            </ul>
          ) : (
            <p className="wrong-lyrics__empty">No lyrics were found for this song.</p>
          )}
          {c.pick && (
            <button type="button" className="lyrics__source-edit wrong-lyrics__auto" onClick={() => void pick(null, "Platter picks the lyrics again.")}>
              Let Platter pick
            </button>
          )}
          {!c.canCheck && <CheckModelOffer />}
        </>
      )}
      <div className="wrong-lyrics__actions">
        <Button variant="ghost" icon={<AudioLines size={16} />} onClick={() => void transcribeNow()}>
          Transcribe this song
        </Button>
        <Button
          variant="ghost"
          icon={<PenLine size={16} />}
          onClick={() => {
            closeWrongLyrics();
            openLyricsEditor(track);
          }}
        >
          Write or paste lyrics
        </Button>
        <Button
          variant="ghost"
          icon={<X size={16} />}
          disabled={c?.pick === "none"}
          onClick={() => void pick("none", "Lyrics are off for this song.")}
        >
          No lyrics for this song
        </Button>
      </div>
      <div className="wrong-lyrics__foot">
        <Button variant="secondary" onClick={closeWrongLyrics}>
          Close
        </Button>
      </div>
    </Dialog>
  );
}

/** Settings: how many songs have a lyrics choice, and Reset. */
export function LyricsChoicesControl({ labelledBy }: { labelledBy?: string }) {
  const [n, setN] = useState<number | null>(null);
  useEffect(() => {
    let alive = true;
    backend.lyricsChoicesCount().then(
      (v) => alive && setN(v),
      () => alive && setN(0),
    );
    return () => {
      alive = false;
    };
  }, []);
  const reset = async () => {
    try {
      const was = await backend.lyricsChoicesClear();
      setN(0);
      lyricsChanged();
      toast.info(was === 1 ? "Lyrics choices reset for 1 song." : `Lyrics choices reset for ${was} songs.`);
    } catch {
      toast.error("Couldn't reset the lyrics choices.");
    }
  };
  return (
    <span className="lyrics-trust__choices" aria-labelledby={labelledBy}>
      <span className="lyrics-trust__count">{n === null ? "" : n === 1 ? "1 song" : `${n} songs`}</span>
      <Button size="sm" disabled={!n} aria-label="Reset lyrics choices" onClick={() => void reset()}>
        Reset
      </Button>
    </span>
  );
}
