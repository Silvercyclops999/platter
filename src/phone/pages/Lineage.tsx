/**
 * A song's lineage on the phone: every version down one timeline, oldest
 * first, each with its date, stage and record, playable; pick two to
 * Compare them (the same compare as on a computer, in a panel at the foot).
 */
import { GitCompareArrows, Pause, Play } from "lucide-react";
import { useState } from "react";
import { Art } from "../../components/Art";
import { versionLabel } from "../../components/FileTagBadges";
import { backend } from "../../lib/backend";
import { formatDuration, plural } from "../../lib/format";
import type { LineageNode } from "../../lib/types";
import { useLoad } from "../../lib/useLoad";
import { SONG_STATUS_LABEL } from "../../pages/era";
import { ComparePanel, STAGE_LABEL, lineageSentence, lineageSpan, nodeDate } from "../../pages/LineagePage";
import { player, usePlayer } from "../../stores/player";
import { nav } from "../../stores/router";
import { useSettings } from "../../stores/settings";
import { PageFailed, PageLoading } from "./common";
import { Empty, GoPill, Hero, PageFrame, PlayRow } from "./kit";
import "../../pages/lineage.css";

function Node({ n, playing, active, pickedAs, onPlay, onPick }: { n: LineageNode; playing: boolean; active: boolean; pickedAs: number; onPlay: () => void; onPick: () => void }) {
  const version = versionLabel(n.version);
  const picked = pickedAs >= 0;
  const tags = [STAGE_LABEL[n.stage], n.status !== "released" ? SONG_STATUS_LABEL[n.status] : null, version && version.toLowerCase() !== STAGE_LABEL[n.stage].toLowerCase() ? version : null].filter(Boolean);
  return (
    <li className={`ph-lineage__node ${n.current ? "is-current" : ""} ${active ? "is-active" : ""}`} aria-current={n.current || undefined}>
      <span className={`ph-lineage__date tabular ${n.when === null ? "is-unknown" : ""}`}>{nodeDate(n)}</span>
      <div className="ph-lineage__card">
        <button type="button" className="ph-lineage__play" aria-label={playing ? `Pause ${n.track.title}` : `Play ${n.track.title}`} onClick={onPlay}>
          <Art path={n.track.thumb} size={56} />
          <span className="ph-lineage__glyph" aria-hidden="true">
            {playing ? <Pause size={18} fill="currentColor" /> : <Play size={18} fill="currentColor" />}
          </span>
        </button>
        <div className="ph-row__text">
          <span className={`ph-row__title ${active ? "is-playing" : ""}`}>
            {n.track.title} {n.current && <span className="ph-x__tag is-accent">This one</span>}
          </span>
          <button type="button" className="ph-row__sub ph-lineage__album" onClick={() => nav.go({ name: "album", id: n.track.albumId })}>
            {[n.track.albumTitle, n.edition].filter(Boolean).join(" · ")} · {formatDuration(n.track.durationMs)}
          </button>
          <span className="ph-lineage__tags">
            {tags.map((t) => (
              <span key={t} className="ph-x__tag">
                {t}
              </span>
            ))}
            {n.era && (
              <button type="button" className="ph-x__tag ph-lineage__era" onClick={() => nav.go({ name: "era", id: n.track.artistId, era: n.era! })}>
                {n.era} era
              </button>
            )}
          </span>
        </div>
        <button
          type="button"
          className={`ph-icon-btn ph-lineage__pick ${picked ? "is-on" : ""}`}
          aria-pressed={picked}
          aria-label={picked ? `Remove ${n.track.title} from Compare` : `Compare ${n.track.title}`}
          onClick={onPick}
        >
          {picked ? <span className="ph-lineage__letter">{pickedAs === 0 ? "A" : "B"}</span> : <GitCompareArrows size={20} aria-hidden="true" />}
        </button>
      </div>
    </li>
  );
}

export function PhoneLineage({ id }: { id: string }) {
  const { data, error, retry } = useLoad(() => backend.trackLineage(id), [id]);
  const on = useSettings((s) => s.settings?.showLineage !== false);
  const nowId = usePlayer((s) => s.state?.track?.id);
  const status = usePlayer((s) => s.state?.status);
  const [picked, setPicked] = useState<string[]>([]);
  if (!on)
    return (
      <PageFrame name="lineage" title="Lineage">
        <Empty title="Lineage is off" text="Turn on Show lineage in Settings to see every version of a song on one timeline." action={<GoPill route={{ name: "settings", row: "showLineage" }}>Open Settings</GoPill>} />
      </PageFrame>
    );
  if (error && !data) return <PageFailed what="this song's lineage" retry={retry} framed />;
  if (!data) return <PageLoading />;
  const ids = data.nodes.map((n) => n.track.id);
  const playingHere = !!nowId && ids.includes(nowId) && status === "playing";
  const face = data.nodes.find((n) => n.stage === "release") ?? data.nodes[0];
  const span = lineageSpan(data);
  const pick = (trackId: string) => setPicked((p) => (p.includes(trackId) ? p.filter((x) => x !== trackId) : p.length >= 2 ? [p[1], trackId] : [...p, trackId]));
  const pickedNodes = picked.map((pid) => data.nodes.find((n) => n.track.id === pid)).filter((n): n is LineageNode => !!n);
  return (
    <PageFrame name="lineage" title={data.title} kicker="Lineage" className="ph-lineage">
      <Hero art={<Art path={face?.track.thumb ?? null} fluid requestSize={480} alt="" />}>
        <button type="button" className="ph-album__artist" onClick={() => nav.go({ name: "artist", id: data.artistId })}>
          {data.artistTitle}
        </button>
        <p className="ph-x__meta tabular">{[plural(data.nodes.length, "version"), span].filter(Boolean).join(" · ")}</p>
      </Hero>
      <PlayRow
        name="every version"
        playing={playingHere}
        onPlay={() => (playingHere ? void player.toggle() : void player.playTracks(ids))}
        left={<span className="ph-x__meta">Pick two to compare them.</span>}
      />
      <p className="lead lead--sm ph-lineage__lead">{lineageSentence(data)}</p>
      <ol className="ph-lineage__list" aria-label={`Versions of ${data.title}, oldest first`}>
        {data.nodes.map((n) => (
          <Node
            key={n.track.id}
            n={n}
            active={n.track.id === nowId}
            playing={n.track.id === nowId && status === "playing"}
            pickedAs={picked.indexOf(n.track.id)}
            onPlay={() => (n.track.id === nowId ? void player.toggle() : void player.playTracks(ids, n.track.id))}
            onPick={() => pick(n.track.id)}
          />
        ))}
      </ol>
      {pickedNodes.length > 0 && (
        <div className="ph-compare">
          <ComparePanel key={pickedNodes.map((n) => n.track.id).join(":")} nodes={pickedNodes} onClear={() => setPicked([])} />
        </div>
      )}
    </PageFrame>
  );
}
