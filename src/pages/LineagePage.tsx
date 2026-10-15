/**
 * A song's lineage: every version on one timeline, oldest first (versions
 * with no date follow), each with its stage, status, quality and era, and
 * each playable. Pick two to Compare them: both files come onto this
 * computer first ("Preparing compare…"), then X switches between them at
 * the same moment, with an optional loop region.
 */
import { SoundsLikeHint } from "../components/SoundsLikeHint";
import { GitCompareArrows, MoreHorizontal, Pause, Play, Repeat, X as Close } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { Art } from "../components/Art";
import { Badge, QualityBadge } from "../components/Badge";
import { Button, IconButton } from "../components/Button";
import { openContextMenu } from "../components/ContextMenu";
import { versionLabel } from "../components/FileTagBadges";
import { Skeleton } from "../components/Skeleton";
import { trackMenuItems } from "../components/TrackRow";
import { backend } from "../lib/backend";
import { formatDuration, plural } from "../lib/format";
import type { AbLoop, CompareNow, CompareReady, CompareSide, Lineage, LineageNode, LineageStage } from "../lib/types";
import { useLoad } from "../lib/useLoad";
import { useArtTint } from "../lib/useTint";
import { player, usePlayer } from "../stores/player";
import { nav } from "../stores/router";
import { useSettings } from "../stores/settings";
import { usePageBar, useTitleStep } from "./detail";
import { formatArchiveDate, SONG_STATUS_LABEL } from "./era";
import "./lineage.css";

export const STAGE_LABEL: Record<LineageStage, string> = {
  demo: "Demo",
  session: "Session",
  leak: "Leak",
  alternate: "Alternate",
  unreleased: "Unreleased",
  release: "Release",
  remaster: "Remaster",
  live: "Live",
};

const QUALITY_WORD: Record<string, string> = { lossless: "Lossless", cdq: "CDQ", lq: "LQ" };

/** The date shown beside a node: ARCHIVE_DATE, else the album's year. */
export function nodeDate(n: LineageNode): string {
  if (n.date) return formatArchiveDate(n.date);
  if (n.year) return String(n.year);
  return "Date unknown";
}

/** The years a lineage spans, from its dated versions. */
export function lineageSpan(l: Lineage): string | null {
  const years = l.nodes.filter((n) => n.when !== null).map((n) => Math.floor(n.when! / 10000));
  if (!years.length) return null;
  const lo = Math.min(...years);
  const hi = Math.max(...years);
  return lo === hi ? String(lo) : `${lo} to ${hi}`;
}

/** The song menu for a node; unreleased versions never get a share link. */
function nodeMenu(n: LineageNode) {
  const items = trackMenuItems(n.track);
  return n.shareable ? items : items.filter((i) => i.label !== "Share");
}

export function LineagePage({ id }: { id: string }) {
  const { data, error } = useLoad(() => backend.trackLineage(id), [id]);
  const on = useSettings((s) => s.settings?.showLineage !== false);
  const nowId = usePlayer((s) => s.state?.track?.id);
  const status = usePlayer((s) => s.state?.status);
  const [picked, setPicked] = useState<string[]>([]);
  const ids = data ? data.nodes.map((n) => n.track.id) : [];
  const playingHere = !!nowId && ids.includes(nowId) && status === "playing";
  const play = () => (playingHere ? player.toggle() : void player.playTracks(ids));
  const face = data?.nodes.find((n) => n.stage === "release") ?? data?.nodes[0];
  useArtTint(null, face?.track.thumb ?? undefined);
  usePageBar(data ? `${data.title} lineage` : undefined, playingHere, play);
  const { header, column, step, compact } = useTitleStep(data?.title);

  if (!on) {
    return (
      <div className="page-pad lineage-off">
        <h1 className="lineage-off__title">Lineage is off</h1>
        <p>Turn on Show lineage in Settings to see every version of a song on one timeline.</p>
        <Button variant="secondary" onClick={() => nav.go({ name: "settings" })}>
          Open Settings
        </Button>
      </div>
    );
  }
  if (error && !data) return <p className="page-pad page-error">Couldn't load this song's lineage: {error.message}</p>;

  const pick = (trackId: string) =>
    setPicked((p) => (p.includes(trackId) ? p.filter((x) => x !== trackId) : p.length >= 2 ? [p[1], trackId] : [...p, trackId]));
  const pickedNodes = picked.map((pid) => data?.nodes.find((n) => n.track.id === pid)).filter((n): n is LineageNode => !!n);
  const span = data ? lineageSpan(data) : null;

  return (
    <div className="detail lineage-page">
      <header ref={header} className={`detail-header ${compact ? "detail-header--compact" : ""}`}>
        <div className="detail-header__art">{face ? <Art path={face.track.thumb} size={232} alt="" /> : <Skeleton width={232} height={232} />}</div>
        <div ref={column} className="detail-header__text">
          {data ? (
            <>
              <span className="detail-header__type">Lineage</span>
              <h1 className="detail-header__title" data-step={step}>
                {data.title}
              </h1>
              <div className="detail-header__meta">
                <button type="button" className="link detail-header__artist" onClick={() => nav.go({ name: "artist", id: data.artistId })}>
                  {data.artistTitle}
                </button>
                <span>• {plural(data.nodes.length, "version")}</span>
                {span && <span>• {span}</span>}
              </div>
              <SoundsLikeHint trackId={id} />
            </>
          ) : (
            <>
              <Skeleton width={60} height={12} />
              <Skeleton width="50%" height={48} style={{ margin: "12px 0" }} />
              <Skeleton width="30%" height={14} />
            </>
          )}
        </div>
      </header>

      <div className="detail-body">
        <div className="action-row">
          <IconButton
            label={playingHere ? "Pause" : "Play every version"}
            variant="accent"
            size="xl"
            disabled={!data}
            icon={playingHere ? <Pause size={24} fill="currentColor" /> : <Play size={24} fill="currentColor" className="nudge-right" />}
            onClick={play}
          />
          <p className="lineage-page__hint">Pick two versions to compare them.</p>
        </div>

        <ol className="lineage" aria-label={data ? `Versions of ${data.title}, oldest first` : "Versions"} aria-busy={!data || undefined}>
          {data
            ? data.nodes.map((n, i) => {
                const undatedStart = n.when === null && (i === 0 || data.nodes[i - 1].when !== null);
                return (
                  <NodeRow
                    key={n.track.id}
                    node={n}
                    divider={undatedStart && i > 0}
                    active={n.track.id === nowId}
                    playing={n.track.id === nowId && status === "playing"}
                    pickedAs={picked.indexOf(n.track.id)}
                    onPlay={() => (n.track.id === nowId ? player.toggle() : void player.playTracks(ids, n.track.id))}
                    onPick={() => pick(n.track.id)}
                  />
                );
              })
            : Array.from({ length: 4 }, (_, i) => (
                <li key={i} className="lineage__node" aria-hidden="true">
                  <span className="lineage__date">
                    <Skeleton width={70} height={12} />
                  </span>
                  <span className="lineage__dot" />
                  <div className="lineage__card">
                    <Skeleton width={48} height={48} />
                    <div className="lineage__text">
                      <Skeleton width="50%" height={14} />
                      <Skeleton width="70%" height={12} style={{ marginTop: 6 }} />
                    </div>
                  </div>
                </li>
              ))}
        </ol>
      </div>
      {pickedNodes.length > 0 && (
        <ComparePanel
          key={pickedNodes.map((n) => n.track.id).join(":")}
          nodes={pickedNodes}
          onClear={() => setPicked([])}
        />
      )}
    </div>
  );
}

function NodeRow({
  node: n,
  divider,
  active,
  playing,
  pickedAs,
  onPlay,
  onPick,
}: {
  node: LineageNode;
  divider: boolean;
  active: boolean;
  playing: boolean;
  pickedAs: number;
  onPlay: () => void;
  onPick: () => void;
}) {
  const version = versionLabel(n.version);
  const quality = n.quality ? QUALITY_WORD[n.quality] : null;
  const picked = pickedAs >= 0;
  const released = n.status === "released";
  return (
    <li
      className={`lineage__node lineage__node--${n.stage} ${n.current ? "is-current" : ""} ${active ? "is-active" : ""} ${picked ? "is-picked" : ""} ${divider ? "lineage__node--undated" : ""}`}
      aria-current={n.current || undefined}
      onContextMenu={(e) => {
        e.preventDefault();
        openContextMenu(`Options for ${n.track.title}`, nodeMenu(n), e);
      }}
    >
      <span className={`lineage__date ${n.when === null ? "is-unknown" : ""}`}>{nodeDate(n)}</span>
      <span className="lineage__dot" aria-hidden="true" />
      <div className="lineage__card">
        <IconButton
          label={playing ? `Pause ${n.track.title}` : `Play ${n.track.title}`}
          size="md"
          variant="filled"
          icon={playing ? <Pause size={16} fill="currentColor" /> : <Play size={16} fill="currentColor" className="nudge-right" />}
          onClick={onPlay}
        />
        <Art path={n.track.thumb} size={48} />
        <div className="lineage__text">
          <div className="lineage__name">
            <span className={`truncate ${active ? "is-playing" : ""}`}>{n.track.title}</span>
            {n.current && <span className="lineage__here">This one</span>}
          </div>
          <div className="lineage__meta">
            <button type="button" className="link truncate" onClick={() => nav.go({ name: "album", id: n.track.albumId })}>
              {[n.track.albumTitle, n.edition].filter(Boolean).join(" · ")}
            </button>
            {n.era && (
              <>
                <span aria-hidden="true">·</span>
                <button type="button" className="link lineage__era" onClick={() => nav.go({ name: "era", id: n.track.artistId, era: n.era! })}>
                  {n.era} era
                </button>
              </>
            )}
          </div>
          <div className="lineage__badges">
            <span className={`lineage__stage lineage__stage--${n.stage}`}>{STAGE_LABEL[n.stage]}</span>
            {!released && (
              <Badge tone="version" title={SONG_STATUS_LABEL[n.status]}>
                {n.status === "probablyAlternate" ? "Alternate" : "Unreleased"}
              </Badge>
            )}
            {version && version.toLowerCase() !== STAGE_LABEL[n.stage].toLowerCase() && <Badge tone="version">{version}</Badge>}
            {quality && <Badge tone="neutral">{quality}</Badge>}
            <QualityBadge track={n.track} />
          </div>
        </div>
        <span className="lineage__duration tabular">{formatDuration(n.track.durationMs)}</span>
        <button
          type="button"
          className={`lineage__pick ${picked ? "is-on" : ""}`}
          aria-pressed={picked}
          aria-label={picked ? `Remove ${n.track.title} from Compare` : `Compare ${n.track.title}`}
          onClick={onPick}
        >
          {picked ? (pickedAs === 0 ? "A" : "B") : <GitCompareArrows size={16} aria-hidden="true" />}
          <span className="lineage__pick-text">{picked ? "" : "Compare"}</span>
        </button>
        <IconButton
          label={`More options for ${n.track.title}`}
          size="sm"
          icon={<MoreHorizontal size={18} />}
          onClick={(e) => openContextMenu(`Options for ${n.track.title}`, nodeMenu(n), e.currentTarget)}
        />
      </div>
    </li>
  );
}

type Phase =
  | { kind: "picking" }
  | { kind: "preparing"; a: number; b: number }
  | { kind: "failed"; message: string }
  | { kind: "ready"; info: CompareReady; now: CompareNow | null };

/** A close still on its way: the next prepare waits for it, so it can't cancel the new one. */
let ending: Promise<void> = Promise.resolve();

function isTyping(el: EventTarget | null): boolean {
  return el instanceof HTMLElement && (el.isContentEditable || el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.tagName === "SELECT");
}

/** Compare two versions: prepare both files, then switch with X. */
export function ComparePanel({ nodes, onClear }: { nodes: LineageNode[]; onClear: () => void }) {
  const [a, b] = nodes;
  const [phase, setPhase] = useState<Phase>(b ? { kind: "preparing", a: 0, b: 0 } : { kind: "picking" });
  const [attempt, setAttempt] = useState(0);
  const [loopStart, setLoopStart] = useState<number | null>(null);
  const position = usePlayer((s) => s.positionMs);
  const status = usePlayer((s) => s.state?.status);
  const nowId = usePlayer((s) => s.state?.track?.id);
  const abLoop = usePlayer((s) => s.state?.abLoop ?? null);
  const ready = phase.kind === "ready" ? phase : null;

  useEffect(() => {
    if (!b) return;
    let alive = true;
    setPhase({ kind: "preparing", a: 0, b: 0 });
    const un = backend.listen("compare://progress", (p) => {
      if (!alive) return;
      setPhase((ph) => (ph.kind === "preparing" ? { ...ph, [p.side]: p.fraction } : ph));
    });
    ending.then(() => backend.comparePrepare(a.track.id, b.track.id)).then(
      (info) => alive && setPhase({ kind: "ready", info, now: null }),
      (e: { message?: string }) => alive && setPhase({ kind: "failed", message: e.message ?? "Couldn't prepare the files." }),
    );
    return () => {
      alive = false;
      void un.then((f) => f());
    };
  }, [a.track.id, b?.track.id, attempt]);

  // Closing Compare (or leaving the page) gives the sound settings back.
  const pair = !!b;
  useEffect(() => {
    if (!pair) return;
    return () => {
      ending = backend.compareEnd().catch(() => {});
    };
  }, [pair]);

  const side: CompareSide | null = ready?.now?.side ?? null;
  const sideOf = (s: CompareSide) => (s === "a" ? a : b);
  const listening = !!side && nowId === sideOf(side)?.track.id;

  const go = useCallback(
    async (to: CompareSide, play?: boolean) => {
      if (phase.kind !== "ready") return;
      try {
        const now = await backend.compareSwitch(to, play);
        setPhase((ph) => (ph.kind === "ready" ? { ...ph, now } : ph));
      } catch (e) {
        setPhase({ kind: "failed", message: (e as { message?: string }).message ?? "Couldn't switch." });
      }
    },
    [phase.kind],
  );
  const flip = useCallback(() => void go(side === "a" ? "b" : "a"), [go, side]);

  useEffect(() => {
    if (phase.kind !== "ready") return;
    const onKey = (e: KeyboardEvent) => {
      if (e.ctrlKey || e.metaKey || e.altKey || isTyping(e.target)) return;
      if (e.key === "x" || e.key === "X") {
        e.preventDefault();
        flip();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [phase.kind, flip]);

  const setLoop = async (ab: AbLoop | null) => {
    try {
      const now = await backend.compareLoop(ab);
      setPhase((ph) => (ph.kind === "ready" && ph.now ? { ...ph, now: { ...ph.now, abLoop: now } } : ph));
    } catch (e) {
      setPhase({ kind: "failed", message: (e as { message?: string }).message ?? "Couldn't set the loop." });
    }
  };
  const loop = listening ? abLoop : null;

  return (
    <section className="compare" aria-label="Compare" data-testid="compare">
      <div className="compare__head">
        <GitCompareArrows size={18} aria-hidden="true" />
        <h2 className="compare__title">Compare</h2>
        {ready?.info.approximate && (
          <span className="compare__note" title="These versions differ in length by more than 5%, so switching keeps the same point in the song by proportion.">
            Approximate
          </span>
        )}
        {ready && (
          <span className="compare__note" title={ready.info.levelAvailable ? undefined : "The server has no loudness data for one of these versions."}>
            {ready.info.levelMatched ? "Level matched" : "Not level matched"}
          </span>
        )}
        <IconButton label="Close Compare" size="sm" icon={<Close size={18} />} onClick={onClear} className="compare__close" />
      </div>

      {phase.kind === "picking" && <p className="compare__status">Pick one more version to compare with {a.track.title}.</p>}

      {phase.kind === "preparing" && (
        <div className="compare__status" role="status">
          <p>Preparing compare…</p>
          <p className="compare__sub">Both files come onto this computer first, one after the other, so switching is instant.</p>
          <div className="compare__progress">
            {(["a", "b"] as const).map((s) => (
              <div key={s} className="compare__bar">
                <span className="compare__letter">{s.toUpperCase()}</span>
                <span className="truncate">{sideOf(s).track.title}</span>
                <progress max={1} value={phase[s]} aria-label={`${sideOf(s).track.title}: ${Math.round(phase[s] * 100)}%`} />
              </div>
            ))}
          </div>
        </div>
      )}

      {phase.kind === "failed" && (
        <div className="compare__status" role="alert">
          <p>Couldn't prepare Compare. {phase.message}</p>
          <Button variant="secondary" onClick={() => setAttempt((n) => n + 1)}>
            Try again
          </Button>
        </div>
      )}

      {ready && (
        <div className="compare__body">
          <div className="compare__sides" role="group" aria-label="Which version plays">
            {(["a", "b"] as const).map((s) => (
              <button
                key={s}
                type="button"
                className={`compare__side ${side === s ? "is-on" : ""}`}
                aria-pressed={side === s}
                onClick={() => void go(s)}
              >
                <span className="compare__letter">{s.toUpperCase()}</span>
                <span className="compare__side-text">
                  <span className="truncate">{sideOf(s).track.title}</span>
                  <span className="compare__side-meta truncate">
                    {STAGE_LABEL[sideOf(s).stage]} · {nodeDate(sideOf(s))} · {formatDuration(sideOf(s).track.durationMs)}
                  </span>
                </span>
              </button>
            ))}
          </div>
          <div className="compare__controls">
            <IconButton
              label={listening && status === "playing" ? "Pause" : "Play"}
              variant="accent"
              size="lg"
              icon={listening && status === "playing" ? <Pause size={20} fill="currentColor" /> : <Play size={20} fill="currentColor" className="nudge-right" />}
              onClick={() => (listening ? player.toggle() : void go(side ?? "a", true))}
            />
            <Button variant="secondary" onClick={flip}>
              Switch <kbd className="compare__kbd">X</kbd>
            </Button>
            <span className="compare__time tabular">{listening ? formatDuration(position) : "0:00"}</span>
            <span className="compare__spacer" />
            {loop ? (
              <>
                <span className="compare__loop tabular">
                  <Repeat size={14} aria-hidden="true" /> {formatDuration(loop.aMs)} to {formatDuration(loop.bMs)}
                </span>
                <Button variant="ghost" onClick={() => void setLoop(null)}>
                  Clear loop
                </Button>
              </>
            ) : loopStart === null ? (
              <Button variant="ghost" disabled={!listening} onClick={() => setLoopStart(position)}>
                Loop from here
              </Button>
            ) : (
              <>
                <span className="compare__loop tabular">From {formatDuration(loopStart)}</span>
                <Button
                  variant="ghost"
                  disabled={!listening || position <= loopStart + 1000}
                  onClick={() => {
                    void setLoop({ aMs: loopStart, bMs: position });
                    setLoopStart(null);
                  }}
                >
                  End loop here
                </Button>
                <Button variant="ghost" onClick={() => setLoopStart(null)}>
                  Cancel
                </Button>
              </>
            )}
          </div>
          <p className="compare__sub">Press X to switch at the same moment.{ready.info.approximate ? " The lengths differ, so the moment is approximate." : ""}</p>
        </div>
      )}
    </section>
  );
}
