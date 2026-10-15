import { Ellipsis, Pause, Play, SlidersHorizontal } from "lucide-react";
import { useEffect, useState } from "react";
import { Button, IconButton } from "../components/Button";
import { openContextMenu } from "../components/ContextMenu";
import { FIELDS, rangeWords } from "../components/SmartPlaylistEditor";
import { Skeleton } from "../components/Skeleton";
import { TrackList, TrackRow, TrackRowSkeleton, trackMenuItems } from "../components/TrackRow";
import { backend } from "../lib/backend";
import { formatLongDuration, plural } from "../lib/format";
import type { SmartPage, SmartRules } from "../lib/types";
import { useLoad } from "../lib/useLoad";
import { bookmarkPageItem, deleteSmartPlaylist, editSmartPlaylist } from "../stores/curate";
import { player, usePlayer } from "../stores/player";
import "../components/smart-editor.css";

const OP_WORDS: Record<string, string> = {
  is: "is",
  isNot: "is not",
  contains: "contains",
  notContains: "doesn't contain",
  gt: "over",
  lt: "under",
  inLast: "in the last",
  notInLast: "not in the last",
  between: "between",
};

/** "30d" -> "30 days", "1y" -> "year". */
export function spanWords(v: string): string {
  const m = /^(\d+)(d|w|m|y)$/.exec(v.trim());
  if (!m) return v;
  const n = Number(m[1]);
  const unit = { d: "day", w: "week", m: "month", y: "year" }[m[2] as "d"];
  return n === 1 ? unit : `${n} ${unit}s`;
}

/** "Genre is Rock, and Plays over 3". */
export function rulesLine(r: SmartRules): string {
  if (!r.rules.length) return "Every song in your library";
  const parts = r.rules.map((x) => {
    const label = FIELDS.find((f) => f.value === x.field)?.label ?? x.field;
    const value = x.value === "true" ? "yes" : x.value === "false" ? "no" : x.op === "between" ? rangeWords(x.value) : x.op === "inLast" || x.op === "notInLast" ? spanWords(x.value) : x.value;
    return `${label} ${OP_WORDS[x.op] ?? x.op} ${value}`;
  });
  return parts.join(r.matchAll ? ", and " : ", or ");
}

function updateLine(p: SmartPage["playlist"]): string | null {
  if (p.update === "live" || !p.nextUpdateAt) return null;
  const when = new Date(p.nextUpdateAt * 1000).toLocaleString(undefined, { weekday: "short", hour: "numeric", minute: "2-digit" });
  return `${p.update === "daily" ? "Updates daily" : "Updates weekly"}, next ${when}`;
}

/** A Platter smart playlist: songs its rules find on this computer. */
export function SmartPlaylistPage({ id }: { id: string }) {
  const [version, setVersion] = useState(0);
  const { data, error } = useLoad(() => backend.smartPage(id), [id, version]);
  useEffect(() => {
    let un: (() => void) | undefined;
    let alive = true;
    backend.listen("smart://changed", () => setVersion((v) => v + 1)).then((u) => (alive ? (un = u) : u()));
    return () => {
      alive = false;
      un?.();
    };
  }, []);
  const nowTrackId = usePlayer((s) => s.state?.track?.id);
  const status = usePlayer((s) => s.state?.status);
  const ids = data?.tracks.map((t) => t.id) ?? [];
  const playing = !!nowTrackId && ids.includes(nowTrackId) && status === "playing";
  const play = (start?: string) => void player.playTracks(ids, start);

  if (error && !data) return <p className="page-pad page-error">{error.kind === "notFound" ? "This smart playlist isn't here any more." : `Couldn't load this smart playlist: ${error.message}`}</p>;

  const pl = data?.playlist;
  const menu = (el: HTMLElement) =>
    pl &&
    openContextMenu(
      `Options for ${pl.title}`,
      [
        { label: "Edit rules", onSelect: () => void editSmartPlaylist(pl.id, pl.title) },
        { label: "Add to queue", disabled: !ids.length, onSelect: () => void player.addToQueue(data!.tracks) },
        bookmarkPageItem("smart", pl.id, pl.title),
        { label: "Delete", separator: true, onSelect: () => void deleteSmartPlaylist(pl.id) },
      ],
      el,
    );

  return (
    <div className="page-pad" data-testid="smart-page">
      <header className="smart-head">
        <p className="detail-header__type">
          Smart playlist <span className="platter-mark" title="Kept by Platter and filled on this computer">Platter</span>
        </p>
        {pl ? <h1 className="page-title">{pl.title}</h1> : <Skeleton width="40%" height={40} />}
        {data && <p className="smart-rules-line">{data.prompt ? `"${data.prompt}". ` : ""}{rulesLine(data.rules)}</p>}
        {pl && (
          <p className="smart-rules-line">
            {plural(pl.trackCount, "song")}, {formatLongDuration(pl.durationMs)}
            {updateLine(pl) ? ` • ${updateLine(pl)}` : ""}
          </p>
        )}
      </header>
      <div className="action-row">
        <IconButton
          label={playing ? `Pause ${pl?.title ?? "playlist"}` : `Play ${pl?.title ?? "playlist"}`}
          variant="accent"
          size="xl"
          disabled={!ids.length}
          icon={playing ? <Pause size={24} fill="currentColor" /> : <Play size={24} fill="currentColor" className="nudge-right" />}
          onClick={() => (playing ? void player.toggle() : play())}
        />
        <Button variant="secondary" icon={<SlidersHorizontal size={16} />} disabled={!pl} onClick={() => pl && void editSmartPlaylist(pl.id, pl.title)}>
          Edit rules
        </Button>
        <IconButton label={`More options for ${pl?.title ?? "playlist"}`} size="lg" icon={<Ellipsis size={22} />} disabled={!pl} onClick={(e) => menu(e.currentTarget)} />
      </div>
      <TrackList label={pl ? `${pl.title} songs` : "Songs"}>
        {data
          ? data.tracks.map((t, i) => (
              <TrackRow
                key={`${t.id}-${i}`}
                rowKey={`${t.id}-${i}`}
                track={t}
                index={i + 1}
                showArt
                active={t.id === nowTrackId}
                playing={t.id === nowTrackId && status === "playing"}
                onPlay={() => play(t.id)}
                menuItems={trackMenuItems(t)}
              />
            ))
          : Array.from({ length: 8 }, (_, i) => <TrackRowSkeleton key={i} showArt />)}
      </TrackList>
      {data && !data.tracks.length && (
        <p className="home__empty" role="status">
          No songs match these rules right now.
        </p>
      )}
    </div>
  );
}
