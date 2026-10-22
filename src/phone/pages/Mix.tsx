/**
 * A made-for-you mix or a radio on the phone: its art, what it is, play and
 * shuffle, the steering pills on mixes that steer, the download and add
 * buttons (downloading saves the mix to Your Library; removing the download
 * leaves it there), Save as playlist and Refresh (Make a new radio), then
 * the songs. The same core calls as the computer's mix page.
 */
import { CircleCheck, CirclePlus, ListPlus, RotateCw } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { Button } from "../../components/Button";
import { Dialog } from "../../components/Dialog";
import { PhoneDownload } from "../PhoneDownload";
import { ListDownloadingFor } from "../DownloadMark";
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
import { findItem, setDownload, useDownloads } from "../../stores/downloads";
import { saveMix, unsaveMix, useSavedMixes } from "../../stores/savedMixes";
import { player, usePlayer } from "../../stores/player";
import { toast } from "../../stores/toasts";
import { tap } from "../bits";
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
            {!radio && <SaveMixButton id={key} title={mix.title} trackCount={ids.length} />}
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
        <ListDownloadingFor item={radio ? undefined : { kind: "mix", id: key }}>
          <ul className="ph-songs" aria-label={`${mix.title} songs`}>
            {page.tracks.map((t, i) => (
              <SongRow key={`${t.id}-${i}`} track={t} index={i + 1} onPlay={() => void play(t.id)} />
            ))}
          </ul>
        </ListDownloadingFor>
      ) : (
        <Empty title="Nothing to play here right now" text="Mixes are made again each day as you listen." />
      )}
    </PageFrame>
  );
}

/**
 * Add a mix to Your Library, or take it out. A downloaded mix is in Library
 * because of its download, so taking it out asks first: the download goes too.
 */
export function SaveMixButton({ id, title, trackCount }: { id: string; title: string; trackCount: number }) {
  const saved = useSavedMixes((s) => s.mixes.some((m) => m.id === id));
  const downloaded = useDownloads((s) => !!findItem(s.status, "mix", id));
  const [asking, setAsking] = useState(false);
  const cancel = useRef<HTMLButtonElement>(null);
  const on = saved || downloaded;
  const remove = () => {
    unsaveMix(id);
    toast.info("Removed from your library");
  };
  const press = () => {
    tap();
    if (!on) {
      saveMix({ id, title, trackCount });
      toast.info("Added to your library");
    } else if (downloaded) setAsking(true);
    else remove();
  };
  return (
    <>
      <button type="button" className={`ph-icon-btn ph-save ${on ? "is-on" : ""}`} aria-label={on ? `Remove ${title} from your library` : `Add ${title} to your library`} onClick={press}>
        {on ? <CircleCheck size={24} aria-hidden="true" /> : <CirclePlus size={24} aria-hidden="true" />}
      </button>
      <Dialog open={asking} onClose={() => setAsking(false)} label="Remove from your library?" className="confirm" initialFocus={cancel}>
        <h2 className="confirm__title">Remove from your library?</h2>
        <p className="confirm__body">Its download comes off this phone too, so it won't play offline.</p>
        <div className="confirm__actions">
          <Button ref={cancel} variant="ghost" onClick={() => setAsking(false)}>
            Cancel
          </Button>
          <Button
            variant="primary"
            onClick={() => {
              setAsking(false);
              remove();
              void setDownload({ kind: "mix", id, title }, false);
            }}
          >
            Remove
          </Button>
        </div>
      </Dialog>
    </>
  );
}
