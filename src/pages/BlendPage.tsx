/**
 * A Blend (`blend:<group>`): made on this computer from everyone's taste
 * summaries and the songs you can play, about 50 songs taking turns between
 * people, made again each day. Each row says whose taste it came from and
 * can be hidden in this Blend for good.
 */
import { Ellipsis, EyeOff, Pause, Play, Shuffle } from "lucide-react";
import { useState } from "react";
import { IconButton } from "../components/Button";
import { openContextMenu } from "../components/ContextMenu";
import { Face } from "../components/Faces";
import { Skeleton } from "../components/Skeleton";
import { TrackList, TrackRow, TrackRowSkeleton, trackMenuItems } from "../components/TrackRow";
import { backend } from "../lib/backend";
import { formatLongDuration, plural } from "../lib/format";
import type { BlendSummary, CirclePerson } from "../lib/types";
import { useLoad } from "../lib/useLoad";
import { useApp } from "../stores/app";
import { openCircleDialog, playBlend, useCircle } from "../stores/circle";
import { player, usePlayer } from "../stores/player";
import { copyText, toast } from "../stores/toasts";
import { usePageBar, useTitleStep } from "./detail";
import { MissingRow } from "./SharedPlaylistPage";
import "./playlist.css";
import { PageMissing } from "../components/PageMissing";
import { failText } from "../lib/errorText";

/** Everyone's pictures in one square: halves, a third each, or quarters. */
export function BlendCover({ people, size }: { people: Pick<CirclePerson, "id" | "name" | "avatar">[]; size: number }) {
  const shown = people.slice(0, 4);
  return (
    <span className={`blend-cover blend-cover--${Math.max(2, shown.length)}`} style={{ width: size, height: size }} aria-hidden="true">
      {shown.map((p) => (
        <Face key={p.id} person={p} size={size / 2} />
      ))}
      <span className="blend-cover__mark">Blend</span>
    </span>
  );
}

export function BlendPage({ id }: { id: string }) {
  const version = useCircle((s) => s.version);
  const [refreshed, setRefreshed] = useState(0);
  const { data, error, retry } = useLoad(() => backend.blendPage(id), [id, version, refreshed]);
  const [hidden, setHidden] = useState<Set<string>>(new Set());
  const live = useCircle((s) => s.view.blends.find((b) => b.id === id) ?? null);
  const summary: BlendSummary | null = live ?? data?.summary ?? null;
  const user = useApp((s) => s.auth?.username ?? null);
  const rows = (data?.rows ?? []).filter((r) => !hidden.has(r.track.id));
  const ids = rows.map((r) => r.track.id);
  const nowTrackId = usePlayer((s) => s.state?.track?.id);
  const status = usePlayer((s) => s.state?.status);
  const isThis = !!nowTrackId && ids.includes(nowTrackId);
  const playing = isThis && status === "playing";
  const play = (start?: string) => (isThis && !start ? player.toggle() : void playBlend(id, start));
  usePageBar(summary?.title, playing, () => play());
  const { header, column, step, compact } = useTitleStep(summary?.title);

  if (error && !data) {
    return <PageMissing what="Blend" error={error} missing="This Blend isn't here any more" onRetry={retry} />;
  }

  const hide = async (trackId: string, title: string) => {
    setHidden((h) => new Set(h).add(trackId));
    try {
      await backend.blendHide(id, trackId);
      toast.info(`Hidden in this Blend: ${title}`);
    } catch (e) {
      setHidden((h) => {
        const n = new Set(h);
        n.delete(trackId);
        return n;
      });
      toast.error(failText(`Couldn't hide ${title}`, e));
    }
  };
  const saveAsPlaylist = async () => {
    if (!summary || !rows.length) return;
    try {
      const pl = await backend.playlistCreate(summary.title, ids);
      toast.info(`Saved as a playlist: ${pl.title}`);
    } catch (e) {
      toast.error(failText(`Couldn't save the playlist`, e));
    }
  };
  const invite = async () => {
    try {
      copyText((await backend.blendInviteLink(id)).link);
    } catch (e) {
      toast.error(failText(`Couldn't make an invite link`, e));
    }
  };
  const refresh = async () => {
    try {
      await backend.blendRefresh(id);
      setRefreshed((n) => n + 1);
    } catch (e) {
      toast.error(failText(`Couldn't refresh the Blend`, e));
    }
  };
  const pageMenu = (el: HTMLElement) =>
    summary &&
    openContextMenu(
      `Options for ${summary.title}`,
      [
        { label: "Add to queue", disabled: !rows.length, onSelect: () => void player.addToQueue(rows.map((r) => r.track)) },
        { label: "Save as playlist", disabled: !rows.length, onSelect: () => void saveAsPlaylist() },
        { label: "Refresh", onSelect: () => void refresh() },
        { label: "Copy invite link", disabled: summary.people.length >= 10, onSelect: () => void invite() },
        { label: "Leave Blend", separator: true, onSelect: () => openCircleDialog({ kind: "leaveBlend", id, title: summary.title }) },
      ],
      el,
    );

  return (
    <div className="detail mix-page" data-testid="blend-page">
      <header ref={header} className={`detail-header ${compact ? "detail-header--compact" : ""}`}>
        <div className="detail-header__art">{summary ? <BlendCover people={summary.people} size={232} /> : <Skeleton width={232} height={232} />}</div>
        <div ref={column} className="detail-header__text">
          {summary ? (
            <>
              <span className="detail-header__type">Blend</span>
              <h1 className="detail-header__title" data-step={step}>
                {summary.title}
              </h1>
              <p className="mix-header__description">{summary.description}</p>
              <div className="detail-header__meta">
                <span className="mix-header__made">Made for {user ?? "you"}</span>
                <span aria-hidden="true">·</span>
                <span>{plural(rows.length, "song")},</span>
                <span className="detail-header__muted">{formatLongDuration(rows.reduce((n, r) => n + r.track.durationMs, 0))}</span>
              </div>
              {summary.people.length < 2 && (
                <p className="pl-notice" role="status">
                  Just you so far. Your Blend fills in as people join.
                </p>
              )}
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
            label={playing ? `Pause ${summary?.title ?? "Blend"}` : `Play ${summary?.title ?? "Blend"}`}
            variant="accent"
            size="xl"
            disabled={!rows.length}
            icon={playing ? <Pause size={24} fill="currentColor" /> : <Play size={24} fill="currentColor" className="nudge-right" />}
            onClick={() => play()}
          />
          <IconButton
            label="Shuffle play"
            size="lg"
            icon={<Shuffle size={22} />}
            disabled={!rows.length}
            onClick={() => {
              const shuffled = [...ids].sort(() => Math.random() - 0.5);
              void player.playTracks(shuffled);
            }}
          />
          <IconButton label={`More options for ${summary?.title ?? "Blend"}`} size="lg" icon={<Ellipsis size={22} />} disabled={!summary} onClick={(e) => pageMenu(e.currentTarget)} />
        </div>
        <TrackList label={summary ? `${summary.title} songs` : "Songs"} byColumn>
          {data
            ? rows.map((r, i) => (
                <TrackRow
                  key={r.track.id}
                  track={r.track}
                  index={i + 1}
                  showArt
                  active={r.track.id === nowTrackId}
                  playing={r.track.id === nowTrackId && status === "playing"}
                  onPlay={() => void playBlend(id, r.track.id)}
                  addedBy={r.by}
                  menuItems={trackMenuItems(r.track, { afterQueue: [{ label: "Hide in this playlist", icon: EyeOff, onSelect: () => void hide(r.track.id, r.track.title) }] })}
                  trailing={
                    <button
                      type="button"
                      tabIndex={-1}
                      className="track-row__hover-btn track-row__hide"
                      aria-label={`Hide ${r.track.title} in this playlist`}
                      title="Hide in this playlist"
                      onClick={(e) => {
                        e.stopPropagation();
                        void hide(r.track.id, r.track.title);
                      }}
                    >
                      <EyeOff size={16} aria-hidden="true" />
                    </button>
                  }
                />
              ))
            : Array.from({ length: 8 }, (_, i) => <TrackRowSkeleton key={i} showArt />)}
        </TrackList>
        {data && data.missing.length > 0 && (
          <section className="blend-missing" aria-label="Not in your library">
            <h2 className="blend-missing__title">Not in your library</h2>
            <p className="blend-missing__lede">Songs from the others' tastes you can't play here. Put them on your Missing list to find later.</p>
            <TrackList label="Not in your library" byColumn>
              {data.missing.map((m, i) => (
                <MissingRow key={`${m.title}-${m.artist}`} row={{ ...m, key: `${m.title}-${m.artist}`, durationMs: 0 }} index={i + 1} />
              ))}
            </TrackList>
          </section>
        )}
      </div>
    </div>
  );
}
