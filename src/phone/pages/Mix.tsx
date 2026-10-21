/**
 * A made-for-you mix or a radio on the phone: its art, what it is, play and
 * shuffle, the steering pills on mixes that steer, Save as playlist and
 * Refresh (Make a new radio), then the songs. The same core calls as the
 * computer's mix page.
 */
import { ListPlus, RotateCw } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { PhoneDownload } from "../PhoneDownload";
import { backend } from "../../lib/backend";
import { failText } from "../../lib/errorText";
import { formatLongDuration, plural } from "../../lib/format";
import type { MixPage as MixData, MixSteer } from "../../lib/types";
import { useLoad } from "../../lib/useLoad";
import { useContextState } from "../../pages/cards";
import { MixArt } from "../../pages/MixCard";
import { steers } from "../../pages/MixPage";
import { MixSleeve } from "../../pages/MixesPage";
import { useCurate } from "../../stores/curate";
import { player, usePlayer } from "../../stores/player";
import { toast } from "../../stores/toasts";
import { PageFailed, PageLoading, SongRow } from "./common";
import { Empty, Hero, PageFrame, Pills, PlayRow } from "./kit";

const STEER: { id: "none" | MixSteer; label: string }[] = [
  { id: "none", label: "As made" },
  { id: "familiar", label: "More familiar" },
  { id: "new", label: "More new" },
];

export function PhoneMix({ id, seed }: { id?: string; seed?: string }) {
  const key = id ?? seed ?? "";
  const radio = !!seed;
  const [steer, setSteer] = useState<MixSteer | null>(null);
  const [salt, setSalt] = useState(() => Date.now() % 1_000_000_007);
  const [version, setVersion] = useState(0);
  const { data, error, retry } = useLoad(() => (radio ? backend.radioPage(key, steer, salt) : backend.mixPage(key, steer)), [key, steer, salt, version]);
  const [shown, setShown] = useState<MixData | null>(null);
  useEffect(() => {
    if (data) setShown(data);
  }, [data]);
  const hiddenTracks = useCurate((s) => s.hiddenTracks);
  const hiddenArtists = useCurate((s) => s.hiddenArtists);
  const page = useMemo(() => {
    if (!shown) return null;
    const keep = shown.tracks.map((t) => !hiddenTracks.has(t.id) && !hiddenArtists.has(t.artistId));
    return { ...shown, tracks: shown.tracks.filter((_, i) => keep[i]), familiar: shown.familiar.filter((_, i) => keep[i]) };
  }, [shown, hiddenTracks, hiddenArtists]);
  const { isContext, playing } = useContextState(radio ? "radio" : "mix", key);
  const shuffle = usePlayer((s) => s.state?.shuffledContexts?.includes(key) ?? false);
  if (error && !page) return <PageFailed what={radio ? "this radio" : "this mix"} retry={retry} framed />;
  if (!page) return <PageLoading />;
  const mix = page.mix;
  const ids = page.tracks.map((t) => t.id);
  const playIds = radio || steer ? ids : undefined;
  const play = (start?: string) => (radio ? player.playRadio(key, playIds, start) : player.playMix(key, playIds, start));
  const save = async () => {
    try {
      const pl = await backend.playlistCreate(mix.kind === "daylist" ? `daylist: ${mix.title}` : mix.title, ids);
      toast.info(`Saved as a playlist: ${pl.title}`);
    } catch (e) {
      toast.error(failText("Couldn't save the playlist", e));
    }
  };
  const refresh = async () => {
    if (radio) return setSalt((x) => x + 1);
    try {
      setShown(await backend.mixRefresh(key));
      setSteer(null);
      setVersion((v) => v + 1);
    } catch (e) {
      toast.error(failText("Couldn't refresh this mix", e));
    }
  };
  return (
    <PageFrame name={radio ? "radio" : "mix"} title={mix.title} kicker={radio ? "Radio" : mix.kind === "genre" || mix.kind === "mood" ? "Mix" : "Made for you"} className="ph-mix" lede={mix.description || undefined}>
      <Hero art={mix.kind === "daylist" ? <MixSleeve mix={mix} size={240} /> : <MixArt mix={mix} size={480} />}>
        <p className="ph-x__meta tabular">
          {plural(mix.trackCount, "song")}, {formatLongDuration(mix.durationMs)}
        </p>
      </Hero>
      <PlayRow
        name={mix.title}
        playing={isContext && playing}
        disabled={!ids.length}
        onPlay={() => (isContext && !steer ? void player.toggle() : void play())}
        shuffleOn={shuffle}
        onShuffle={() => void player.setContextShuffle(key, !shuffle)}
        left={
          <>
            {!radio && <PhoneDownload kind="mix" id={key} title={mix.title} trackIds={() => ids} />}
            <button type="button" className="ph-icon-btn" aria-label="Save as playlist" disabled={!ids.length} onClick={() => void save()}>
              <ListPlus size={24} aria-hidden="true" />
            </button>
            <button type="button" className="ph-icon-btn" aria-label={radio ? "Make a new radio" : "Refresh"} onClick={() => void refresh()}>
              <RotateCw size={22} aria-hidden="true" />
            </button>
          </>
        }
      />
      {steers(mix.kind) && <Pills label="Steer this mix" items={STEER} value={steer ?? "none"} onChange={(v) => setSteer(v === "none" ? null : v)} />}
      {page.tracks.length ? (
        <ul className="ph-songs" aria-label={`${mix.title} songs`}>
          {page.tracks.map((t, i) => (
            <SongRow key={`${t.id}-${i}`} track={t} index={i + 1} onPlay={() => void play(t.id)} />
          ))}
        </ul>
      ) : (
        <Empty title="Nothing to play here right now" text="Mixes are made again each day as you listen." />
      )}
    </PageFrame>
  );
}
