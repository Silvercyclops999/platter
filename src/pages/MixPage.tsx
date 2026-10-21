import { Ellipsis, Pause, Play, Shuffle } from "lucide-react";
import { useEffect, useMemo, useState, type CSSProperties } from "react";
import { Art } from "../components/Art";
import { IconButton } from "../components/Button";
import { DownloadButton } from "../components/DownloadButton";
import { openContextMenu } from "../components/ContextMenu";
import { Skeleton } from "../components/Skeleton";
import { FilterChips } from "../components/Tabs";
import { TrackList, TrackRow, TrackRowSkeleton, trackMenuItems } from "../components/TrackRow";
import { artUrl, backend } from "../lib/backend";
import { formatLongDuration, plural } from "../lib/format";
import type { MixPage as MixData, MixSteer, MixSummary, Track } from "../lib/types";
import { useLoad } from "../lib/useLoad";
import { bookmarkPageItem, hideSongItem, useCurate } from "../stores/curate";
import { useApp } from "../stores/app";
import { downloadMenuItems } from "../stores/downloads";
import { player, usePlayer } from "../stores/player";
import { nav } from "../stores/router";
import { toast } from "../stores/toasts";
import { useContextState } from "./cards";
import { usePageBar, useTitleStep } from "./detail";
import { daylistGradient } from "./MixCard";
import { MixStory } from "./MixStory";
import "./mixes.css";
import { PageMissing } from "../components/PageMissing";
import { failText } from "../lib/errorText";

const TYPE_LABEL: Record<MixSummary["kind"], string> = {
  daily: "Made for you",
  discover: "Made for you",
  newInLibrary: "Made for you",
  onRepeat: "Made for you",
  rediscover: "Made for you",
  genre: "Mix",
  mood: "Mix",
  daylist: "daylist",
  topSongs: "Playlist",
  radio: "Radio",
};

/** Steering chips show on genre and mood mixes and on radio. */
export const steers = (kind: MixSummary["kind"]) => kind === "genre" || kind === "mood" || kind === "radio";

const STEER_CHIPS = [
  { id: "none", label: "As made" },
  { id: "familiar", label: "More familiar" },
  { id: "new", label: "More new" },
] as const;

function nextUpdate(at: number): string {
  return new Date(at).toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
}

/** The page header: a toned artist photo, the radio's three circles, or covers (Top Songs and the daylist). */
function Header({ mix, user, step, compact, header, column }: {
  mix: MixSummary;
  user: string | null;
  step: number;
  compact: boolean;
  header: React.RefObject<HTMLElement | null>;
  column: React.RefObject<HTMLDivElement | null>;
}) {
  const [a, b] = daylistGradient(new Date().getHours());
  const photo = (mix.kind === "radio" || mix.kind === "daylist" || mix.kind === "topSongs") ? null : mix.art;
  const style = {
    "--mix-color": mix.color,
    "--mix-a": a,
    "--mix-b": b,
    ...(photo ? { "--mix-photo": `url("${artUrl(photo, 1200, 480)}")` } : {}),
  } as CSSProperties;
  return (
    <header ref={header} className={`detail-header mix-header mix-header--${mix.kind} ${photo ? "mix-header--photo" : ""} ${compact ? "detail-header--compact" : ""}`} style={style}>
      {mix.kind === "radio" ? (
        <div className="detail-header__art mix-circles" aria-hidden="true">
          {[mix.covers[1], mix.covers[0], mix.covers[2]].map((p, i) => (
            <span key={i} className={`mix-circles__item ${i === 1 ? "mix-circles__item--seed" : ""}`}>
              <Art path={p ?? null} fluid requestSize={i === 1 ? 232 : 160} round kind="artist" />
            </span>
          ))}
        </div>
      ) : mix.kind === "topSongs" || (mix.kind === "daylist" && mix.covers.length > 0) ? (
        <div className="detail-header__art mix-cover" aria-hidden="true">
          {Array.from({ length: 4 }, (_, i) => (
            <Art key={i} path={mix.covers[i % Math.max(1, mix.covers.length)] ?? null} fluid requestSize={116} kind="album" />
          ))}
        </div>
      ) : null}
      <div ref={column} className="detail-header__text">
        <span className="detail-header__type">{TYPE_LABEL[mix.kind]}</span>
        <h1 className="detail-header__title" data-step={step}>
          {mix.title}
        </h1>
        {mix.description && <p className="mix-header__description">{mix.description}</p>}
        <div className="detail-header__meta">
          <span className="mix-header__made">Made for {user ?? "you"}</span>
          <span aria-hidden="true">·</span>
          <span>{plural(mix.trackCount, "song")},</span>
          <span className="detail-header__muted">{formatLongDuration(mix.durationMs)}</span>
          {mix.nextUpdateAt != null && (
            <>
              <span aria-hidden="true">·</span>
              <span className="detail-header__muted">Next update at {nextUpdate(mix.nextUpdateAt)}</span>
            </>
          )}
        </div>
      </div>
    </header>
  );
}

/** A made-for-you mix (`id`) or a radio (`seed`): the same page, made on this computer. */
export function MixPage({ id, seed }: { id?: string; seed?: string }) {
  const key = id ?? seed ?? "";
  const radio = !!seed;
  const [steer, setSteer] = useState<MixSteer | null>(null);
  const [version, setVersion] = useState(0);
  // A radio keeps its songs while the page is open; "Make a new radio" picks again.
  const [salt, setSalt] = useState(() => Date.now() % 1_000_000_007);
  useEffect(() => setSteer(null), [key]);
  const { data, error, retry } = useLoad(
    () => (radio ? backend.radioPage(key, steer, salt) : backend.mixPage(key, steer)),
    [key, steer, version, salt],
  );
  const [shown, setShown] = useState<MixData | null>(null);
  useEffect(() => {
    if (data) setShown(data);
  }, [data]);
  useEffect(() => setShown(null), [key]);
  // A new day or the next daylist: read it again.
  useEffect(() => {
    if (radio) return;
    let un: (() => void) | undefined;
    let alive = true;
    backend.listen("mixes://changed", () => setVersion((v) => v + 1)).then((u) => (alive ? (un = u) : u()));
    return () => {
      alive = false;
      un?.();
    };
  }, [radio]);
  // Songs hidden while the page is open leave it at once.
  const hiddenTracks = useCurate((st) => st.hiddenTracks);
  const hiddenArtists = useCurate((st) => st.hiddenArtists);
  const page = useMemo(() => {
    if (!shown || (!hiddenTracks.size && !hiddenArtists.size)) return shown;
    const keep = shown.tracks.map((t) => !hiddenTracks.has(t.id) && !hiddenArtists.has(t.artistId));
    if (keep.every(Boolean)) return shown;
    return { ...shown, tracks: shown.tracks.filter((_, i) => keep[i]), familiar: shown.familiar.filter((_, i) => keep[i]) };
  }, [shown, hiddenTracks, hiddenArtists]);
  const mix = page?.mix;
  const user = useApp((s) => s.auth?.username ?? null);
  const kind = radio ? "radio" : "mix";
  const { isContext, playing } = useContextState(kind, key);
  const nowTrackId = usePlayer((s) => s.state?.track?.id);
  const status = usePlayer((s) => s.state?.status);
  const shuffle = usePlayer((s) => s.state?.shuffledContexts?.includes(key) ?? false);
  const smooth = usePlayer((s) => isContext && !!s.state?.smoothTransitions);
  const ids = page?.tracks.map((t) => t.id);
  // A steered mix and a radio page play the songs on the page.
  const playIds = radio || steer ? ids : undefined;
  const play = (start?: string) => (radio ? player.playRadio(key, playIds, start) : player.playMix(key, playIds, start));
  const toggle = () => (isContext && !steer ? player.toggle() : play());
  usePageBar(mix?.title, isContext && playing, toggle);
  const { header, column, step, compact } = useTitleStep(mix?.title);

  if (error && !page) {
    return <PageMissing what="mix" error={error} missing="This mix isn't here any more" reason="Mixes are made again each day." onRetry={retry} />;
  }

  const saveAsPlaylist = async () => {
    if (!page || !mix) return;
    try {
      const pl = await backend.playlistCreate(mix.kind === "daylist" ? `daylist: ${mix.title}` : mix.title, page.tracks.map((t) => t.id));
      toast.info(`Saved as a playlist: ${pl.title}`);
    } catch (e) {
      toast.error(failText(`Couldn't save the playlist`, e));
    }
  };

  const refresh = async () => {
    if (radio) {
      setSalt((x) => x + 1);
      return;
    }
    try {
      setShown(await backend.mixRefresh(key));
      setSteer(null);
    } catch (e) {
      toast.error(failText(`Couldn't refresh this mix`, e));
    }
  };

  const pageMenu = (el: HTMLElement) =>
    mix &&
    openContextMenu(
      `Options for ${mix.title}`,
      [
        { label: "Add to queue", disabled: !page?.tracks.length, onSelect: () => void player.addToQueue(page!.tracks, mix.covers[0] ?? null) },
        { label: "Save as playlist", disabled: !page?.tracks.length, onSelect: () => void saveAsPlaylist() },
        // A radio is made when its page opens, so only mixes download.
        ...(radio || !page ? [] : downloadMenuItems({ kind: "mix", id: key, title: mix.title, trackIds: () => page.tracks.map((t) => t.id) })),
        { label: radio ? "Make a new radio" : "Refresh", onSelect: () => void refresh() },
        bookmarkPageItem(radio ? "radio" : "mix", key, mix.title, { thumb: mix.art ?? mix.covers[0] ?? null }),
        {
          label: smooth ? "Turn off smooth transitions" : "Turn on smooth transitions",
          disabled: !isContext,
          onSelect: () => void player.setSmoothTransitions(!smooth),
        },
      ],
      el,
    );

  const rowMenu = (t: Track) => trackMenuItems(t, { afterQueue: [hideSongItem(t)] });

  return (
    <div className="detail mix-page" data-testid="mix-page">
      {mix ? (
        <Header mix={mix} user={user} step={step} compact={compact} header={header} column={column} />
      ) : (
        <header ref={header} className="detail-header">
          <div ref={column} className="detail-header__text">
            <Skeleton width={60} height={12} />
            <Skeleton width="50%" height={48} style={{ margin: "12px 0" }} />
            <Skeleton width="30%" height={14} />
          </div>
        </header>
      )}
      <div className="detail-body">
        <div className="action-row">
          <IconButton
            label={isContext && playing ? `Pause ${mix?.title ?? "mix"}` : `Play ${mix?.title ?? "mix"}`}
            variant="accent"
            size="xl"
            disabled={!page?.tracks.length}
            icon={isContext && playing ? <Pause size={24} fill="currentColor" /> : <Play size={24} fill="currentColor" className="nudge-right" />}
            onClick={toggle}
          />
          <IconButton
            label={shuffle ? "Disable shuffle" : "Enable shuffle"}
            size="lg"
            pressed={shuffle}
            icon={<Shuffle size={22} />}
            onClick={() => player.setContextShuffle(key, !shuffle)}
          />
          {!radio && mix && page && (
            <DownloadButton kind="mix" id={key} title={mix.title} trackIds={() => page.tracks.map((t) => t.id)} disabled={!page.tracks.length} />
          )}
          <IconButton label={`More options for ${mix?.title ?? "mix"}`} size="lg" icon={<Ellipsis size={22} />} disabled={!mix} onClick={(e) => pageMenu(e.currentTarget)} />
        </div>
        {mix && steers(mix.kind) && (
          <div className="mix-steer">
            <FilterChips
              label="Steer this mix"
              items={STEER_CHIPS.map((c) => ({ id: c.id, label: c.label }))}
              value={steer ?? "none"}
              onChange={(v) => setSteer(v === "none" ? null : (v as MixSteer))}
            />
          </div>
        )}
        {page && mix?.kind !== "topSongs" && <MixStory tracks={page.tracks} familiar={page.familiar} />}
        <TrackList label={mix ? `${mix.title} songs` : "Songs"} showPlays={mix?.kind === "topSongs"}>
          {page
            ? page.tracks.map((t, i) => (
                <TrackRow
                  key={`${t.id}-${i}`}
                  rowKey={`${t.id}-${i}`}
                  track={t}
                  index={i + 1}
                  showArt
                  showPlays={mix?.kind === "topSongs"}
                  active={t.id === nowTrackId}
                  playing={t.id === nowTrackId && status === "playing"}
                  onPlay={() => play(t.id)}
                  menuItems={rowMenu(t)}
                />
              ))
            : Array.from({ length: 8 }, (_, i) => <TrackRowSkeleton key={i} showArt />)}
        </TrackList>
        {page && !page.tracks.length && (
          <p className="home__empty" role="status">
            Nothing to play here right now.{" "}
            <button type="button" className="see-more" onClick={() => nav.go({ name: "home" })}>
              Back to Home
            </button>
          </p>
        )}
      </div>
    </div>
  );
}
