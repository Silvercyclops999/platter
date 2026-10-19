/**
 * A Top Songs list on the phone: the year in big numerals over its own
 * pattern, play, shuffle, save and download, Wrapped, then the songs by
 * rank. Made on this device from the listen log and imported history.
 */
import { ListPlus, Sparkles } from "lucide-react";
import { useMemo } from "react";
import { DownloadButton } from "../../components/DownloadButton";
import { backend } from "../../lib/backend";
import { failText } from "../../lib/errorText";
import { formatLongDuration, plural, trackArtistLine } from "../../lib/format";
import { useLoad } from "../../lib/useLoad";
import { useContextState } from "../../pages/cards";
import { rowNotes, topSongsKey } from "../../pages/TopSongsPage";
import { toneStyle, YearBanner } from "../../pages/TopSongsTile";
import { player, usePlayer } from "../../stores/player";
import { useRouter } from "../../stores/router";
import { useSettings } from "../../stores/settings";
import { toast } from "../../stores/toasts";
import { wrapped } from "../../stores/wrapped";
import { PageFailed, SongRow } from "./common";
import { Empty, GhostRows, GoPill, PageFrame, Pills, PlayRow } from "./kit";
import "../../pages/top-songs.css";

export function PhoneTopSongs({ year }: { year: number | null }) {
  const key = topSongsKey(year);
  const on = useSettings((s) => s.settings?.wrappedTopSongs !== false);
  const showPlays = useSettings((s) => !!s.settings?.topSongsShowPlays);
  const { data: list, error, retry } = useLoad(() => backend.topSongsList(year), [year, on]);
  const span = useLoad(() => backend.statsSpan(), []);
  const { isContext, playing } = useContextState("mix", key);
  const shuffle = usePlayer((s) => s.state?.shuffledContexts?.includes(key) ?? false);
  const title = year == null ? "All-Time Top Songs" : `Your Top Songs ${year}`;
  const ids = useMemo(() => list?.rows.map((r) => r.track.id) ?? [], [list]);
  const years = useMemo(() => [...new Set((span.data?.months ?? []).map((m) => Number(m.slice(0, 4))))], [span.data]);
  const play = (start?: string) => player.playMix(key, ids, start);
  const save = async () => {
    try {
      const pl = await backend.playlistCreate(title, ids);
      toast.info(`Saved as a playlist: ${pl.title}`);
    } catch (e) {
      toast.error(failText("Couldn't save the playlist", e));
    }
  };
  const pick = (
    <Pills
      label="Which list"
      items={[...years.map((y) => ({ id: y, label: String(y) })), { id: 0, label: "All time" }]}
      value={year ?? 0}
      onChange={(y) => {
        useRouter.getState().replace(y === 0 ? { name: "topSongs" } : { name: "topSongs", year: y });
      }}
    />
  );
  if (!on)
    return (
      <PageFrame name="topSongs" title="Top Songs">
        <Empty title="Top Songs are off" text="Turn on Wrapped and Top Songs in Settings, under Stats." action={<GoPill route={{ name: "settings", row: "wrappedTopSongs" }}>Open Settings</GoPill>} />
      </PageFrame>
    );
  return (
    <PageFrame name="topSongs" title={title} className="ph-top-songs">
      <div className="ph-top-songs__banner" style={toneStyle(year)} aria-hidden="true">
        <YearBanner year={year} w={400} h={160} />
        <span className="ph-top-songs__numerals">{year ?? "All time"}</span>
      </div>
      {years.length > 0 && pick}
      {error && !list ? (
        error.kind === "notFound" ? (
          <Empty title={`No list for ${year}`} text="A year needs 30 plays for a Top Songs list." />
        ) : (
          <PageFailed what={title} retry={retry} />
        )
      ) : !list ? (
        <GhostRows n={8} />
      ) : (
        <>
          <p className="ph-x__meta tabular">
            {year == null ? "The songs you've played most, ever." : "The songs you loved most this year."} {plural(list.rows.length, "song")}, {formatLongDuration(list.durationMs)}
          </p>
          <PlayRow
            name={title}
            playing={isContext && playing}
            disabled={!ids.length}
            onPlay={() => (isContext ? void player.toggle() : void play())}
            shuffleOn={shuffle}
            onShuffle={() => void player.setContextShuffle(key, !shuffle)}
            left={
              <>
                <DownloadButton kind="mix" id={key} title={title} trackIds={() => ids} disabled={!ids.length} size="md" />
                <button type="button" className="ph-icon-btn" aria-label="Save as playlist" disabled={!ids.length} onClick={() => void save()}>
                  <ListPlus size={24} aria-hidden="true" />
                </button>
                {year != null && (
                  <button type="button" className="ph-icon-btn" aria-label={`Open ${year} Wrapped`} onClick={() => wrapped.open(year)}>
                    <Sparkles size={22} aria-hidden="true" />
                  </button>
                )}
              </>
            }
          />
          {list.rows.length === 0 ? (
            <Empty title="None of these are here" text="None of these songs are in your library right now." />
          ) : (
            <ul className="ph-songs" aria-label={`${title} songs`}>
              {list.rows.map((r, i) => (
                <SongRow key={r.track.id} track={r.track} index={i + 1} sub={[trackArtistLine(r.track), ...rowNotes(r, { allTime: year == null, showPlays })].join(" · ")} onPlay={() => void play(r.track.id)} />
              ))}
            </ul>
          )}
        </>
      )}
    </PageFrame>
  );
}
