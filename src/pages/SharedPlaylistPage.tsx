/**
 * A playlist someone else owns and shared with you (`shared:<group>`). It
 * plays the songs you can play here; the rest show as "Not in your library"
 * and can go to the Missing list. Collaborators remove the songs they added;
 * nobody but the owner renames, reorders or invites.
 */
import { ArrowDownUp, Ellipsis, ListMusic, Pause, Play } from "lucide-react";
import { useMemo, useState } from "react";
import { Art } from "../components/Art";
import { IconButton } from "../components/Button";
import { openContextMenu } from "../components/ContextMenu";
import { Face } from "../components/Faces";
import { PageMissing } from "../components/PageMissing";
import { Skeleton } from "../components/Skeleton";
import { TrackList, TrackRow, TrackRowSkeleton, trackMenuItems } from "../components/TrackRow";
import { backend } from "../lib/backend";
import { formatDuration, formatLongDuration, plural } from "../lib/format";
import type { SharedRow } from "../lib/types";
import { useLoad } from "../lib/useLoad";
import { leavePlaylist, openCircleDialog, playShared, setInLibrary, useCircle, useShared } from "../stores/circle";
import { player, usePlayer } from "../stores/player";
import { toast } from "../stores/toasts";
import { usePageBar, useTitleStep } from "./detail";
import { Byline, SORT_LABEL, sortRows, sortsFor, type PlaylistSort } from "./PlaylistPage";
import "./playlist.css";

/** Put a song you don't have on the Missing list (its album, as the list keeps albums). */
export async function addSongToMissing(r: { album: string | null; artist: string; releaseGroupId: string | null; title: string }) {
  if (!r.releaseGroupId) {
    toast.info(`Platter doesn't know which album ${r.title} is on, so it can't go on the Missing list.`);
    return;
  }
  try {
    await backend.importMissingAlbumAdd({ releaseGroupId: r.releaseGroupId, title: r.album ?? r.title, artist: r.artist });
    toast.info(`Added ${r.album ?? r.title} to your Missing list`);
  } catch (e) {
    toast.error(`Couldn't add it to the Missing list: ${(e as { message?: string })?.message ?? "something went wrong"}`);
  }
}

/** A row for a song this library doesn't have. */
export function MissingRow({ row, index, byColumn = true }: { row: Pick<SharedRow, "key" | "title" | "artist" | "album" | "durationMs" | "releaseGroupId" | "by">; index: number; byColumn?: boolean }) {
  const menu = (el: HTMLElement) =>
    openContextMenu(`Options for ${row.title}`, [{ label: "Add to Missing list", disabled: !row.releaseGroupId, onSelect: () => void addSongToMissing(row) }], el);
  return (
    <div
      role="row"
      className={`track-row is-missing ${byColumn ? "track-row--by" : ""}`}
      aria-label={`${row.title} by ${row.artist}, not in your library`}
      title="Not in your library"
      onContextMenu={(e) => {
        e.preventDefault();
        menu(e.currentTarget);
      }}
    >
      <div className="track-row__num" role="gridcell">
        <span className="track-row__index tabular">{index}</span>
      </div>
      <div className="track-row__main" role="gridcell">
        <span className="track-row__art">
          <Art path={null} size={40} />
        </span>
        <div className="track-row__text">
          <div className="track-row__title truncate">{row.title}</div>
          <div className="track-row__artist truncate">{row.artist}</div>
          <div className="track-row__missing">Not in your library</div>
        </div>
      </div>
      <div className="track-row__album truncate" role="gridcell">
        {row.album}
      </div>
      {byColumn && (
        <div className="track-row__by" role="gridcell">
          {row.by && (
            <>
              <Face person={row.by} size={20} />
              <span className="truncate">{row.by.name}</span>
            </>
          )}
        </div>
      )}
      <div className="track-row__badges" role="gridcell" />
      <div className="track-row__end" role="gridcell">
        {row.durationMs > 0 && <span className="track-row__duration tabular">{formatDuration(row.durationMs)}</span>}
        <button type="button" className="track-row__hover-btn" aria-label={`More options for ${row.title} by ${row.artist}`} onClick={(e) => menu(e.currentTarget)}>
          <Ellipsis size={16} aria-hidden="true" />
        </button>
      </div>
    </div>
  );
}

export function SharedPlaylistPage({ id }: { id: string }) {
  const version = useCircle((s) => s.version);
  const { data, error } = useLoad(() => backend.collabPage(id), [id, version]);
  const live = useShared(id);
  const s = live ?? data?.summary ?? null;
  const [sort, setSort] = useState<PlaylistSort>("custom");
  const nowTrackId = usePlayer((st) => st.state?.track?.id);
  const status = usePlayer((st) => st.state?.status);
  const rows = useMemo(() => (data ? data.rows.map((r) => ({ ...r, itemId: r.key, track: r.track! })) : []), [data]);
  const view = useMemo(() => {
    // Sorting by song needs a song: the ones you don't have keep their place at the end.
    const have = rows.filter((r) => !!r.track);
    const missing = rows.filter((r) => !r.track);
    return sort === "custom" ? rows : [...sortRows(have, sort, (r) => r.by?.name ?? ""), ...missing];
  }, [rows, sort]);
  const playable = rows.filter((r) => r.track).map((r) => r.track.id);
  const firstArt = rows.find((r) => r.track?.thumb)?.track.thumb ?? null;
  const isThis = usePlayer((st) => {
    const q = st.state?.queue;
    return !!q && q.context?.kind === "tracks" && playable.length > 0 && playable.includes(st.state?.track?.id ?? "");
  });
  const playing = isThis && status === "playing";
  const play = (start?: string) => (isThis && !start ? player.toggle() : void playShared(id, start));
  usePageBar(s?.title, playing, () => play());
  const { header, column, step, compact } = useTitleStep(s?.title);

  if (error && !data) return <PageMissing what="playlist" error={error} />;

  const you = s?.people.find((p) => p.you);
  const role = s?.role ?? "listener";
  const pageMenu = (el: HTMLElement) =>
    s &&
    openContextMenu(
      `Options for ${s.title}`,
      [
        { label: "Add to queue", disabled: !playable.length, onSelect: () => void player.addToQueue(rows.filter((r) => r.track).map((r) => r.track)) },
        ...(s.gone ? [] : [{ label: "Playlist permissions", onSelect: () => openCircleDialog({ kind: "permissions", playlistId: id }) }]),
        s.inLibrary ? { label: "Remove from Your Library", onSelect: () => void setInLibrary(id, false) } : { label: "Add to Your Library", onSelect: () => void setInLibrary(id, true) },
        s.gone
          ? { label: "Remove", separator: true, onSelect: () => void leavePlaylist(id) }
          : role === "follower"
            ? { label: "Stop following", separator: true, onSelect: () => void leavePlaylist(id) }
            : { label: "Leave playlist", separator: true, onSelect: () => openCircleDialog({ kind: "leave", playlistId: id }) },
      ],
      el,
    );
  const sortMenu = (el: HTMLElement) =>
    openContextMenu(
      "Sort songs",
      sortsFor(true).map((x, i) => ({ heading: i === 0 ? "Sort by" : undefined, label: SORT_LABEL[x], checked: sort === x, onSelect: () => setSort(x) })),
      el,
    );
  const remove = async (key: string) => {
    try {
      await backend.collabRemove(id, [key]);
      toast.info(s?.ownerUnreachable ? `Removed. It goes to ${s.owner.name} when they're back online.` : "Removed from this playlist");
    } catch (e) {
      toast.error(`Couldn't remove it: ${(e as { message?: string })?.message ?? "something went wrong"}`);
    }
  };

  return (
    <div className="detail">
      <header ref={header} className={`detail-header ${compact ? "detail-header--compact" : ""}`}>
        <div className="detail-header__art">{s ? <Art path={firstArt} size={232} kind="playlist" /> : <Skeleton width={232} height={232} />}</div>
        <div ref={column} className="detail-header__text">
          {s ? (
            <>
              <span className="detail-header__type">{s.public ? "Public playlist" : "Playlist"}</span>
              <h1 className="detail-header__title" data-step={step}>
                {s.title}
              </h1>
              {data?.description && <p className="pl-summary">{data.description}</p>}
              <Byline shared={s} />
              <div className="detail-header__meta">
                <span>{plural(rows.length, "song")},</span>
                <span className="detail-header__muted">{formatLongDuration(data?.durationMs ?? 0)}</span>
              </div>
              {s.gone ? (
                <p className="pl-notice pl-notice--warn" role="status">
                  This playlist isn't available any more. {s.owner.name} deleted it or took you off.
                </p>
              ) : s.ownerUnreachable ? (
                <p className="pl-notice pl-notice--warn" role="status">
                  {s.owner.name}'s Platter hasn't answered. Your changes go out when it's back online.
                </p>
              ) : s.waiting > 0 ? (
                <p className="pl-notice" role="status">
                  {s.waiting === 1 ? "1 change is" : `${s.waiting} changes are`} on the way to {s.owner.name}.
                </p>
              ) : null}
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
            label={playing ? `Pause ${s?.title ?? "playlist"}` : `Play ${s?.title ?? "playlist"}`}
            variant="accent"
            size="xl"
            disabled={!playable.length}
            icon={playing ? <Pause size={24} fill="currentColor" /> : <Play size={24} fill="currentColor" className="nudge-right" />}
            onClick={() => play()}
          />
          <IconButton label={`More options for ${s?.title ?? "playlist"}`} size="lg" icon={<Ellipsis size={22} />} disabled={!s} onClick={(e) => pageMenu(e.currentTarget)} />
          {rows.length > 0 && (
            <button type="button" className="pl-sort" aria-haspopup="menu" aria-label={`Sort songs: ${SORT_LABEL[sort]}`} onClick={(e) => sortMenu(e.currentTarget)}>
              <span>{SORT_LABEL[sort]}</span>
              <ArrowDownUp size={16} aria-hidden="true" />
            </button>
          )}
        </div>
        {data && rows.length === 0 ? (
          <div className="empty-state">
            <ListMusic size={40} strokeWidth={1.5} aria-hidden="true" />
            <h2 className="empty-state__title">Nothing here yet</h2>
            <p className="empty-state__text">{role === "collaborator" ? "Add songs from any song's menu: Add to playlist." : `Songs ${s?.owner.name ?? "the owner"} adds show up here.`}</p>
          </div>
        ) : (
          <TrackList label={s ? `${s.title} tracks` : "Tracks"} byColumn>
            {data
              ? view.map((r, i) =>
                  r.track ? (
                    <TrackRow
                      key={r.key}
                      rowKey={r.key}
                      track={r.track}
                      index={i + 1}
                      showArt
                      active={r.track.id === nowTrackId}
                      playing={r.track.id === nowTrackId && status === "playing"}
                      onPlay={() => void playShared(id, r.track.id)}
                      addedBy={r.by}
                      notes={r.pending ? [`On the way to ${s?.owner.name ?? "the owner"}`] : undefined}
                      menuItems={trackMenuItems(r.track, {
                        afterPlaylist: r.removable ? [{ label: "Remove from this playlist", onSelect: () => void remove(r.key) }] : [],
                      })}
                    />
                  ) : (
                    <MissingRow key={r.key} row={r} index={i + 1} />
                  ),
                )
              : Array.from({ length: 8 }, (_, i) => <TrackRowSkeleton key={i} showArt />)}
          </TrackList>
        )}
        {you && role === "listener" && !s?.gone && <p className="pl-notice">You're a listener. Only the owner and collaborators add songs.</p>}
      </div>
    </div>
  );
}
