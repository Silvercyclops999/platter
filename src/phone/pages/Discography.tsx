/**
 * An artist's whole discography on the phone: type pills, newest first or A
 * to Z, one release a row. Records in the library play from their row;
 * ones you can get show where they stand, or Get (Lidarr). `catalogue`:
 * `id` is an artist outside the library (MusicBrainz id).
 */
import { ArrowUpDown, Pause, Play } from "lucide-react";
import { useState } from "react";
import { Art } from "../../components/Art";
import { backend } from "../../lib/backend";
import { isCreditId } from "../../lib/credits";
import type { Album } from "../../lib/types";
import { useLoad } from "../../lib/useLoad";
import { catalogueEntries, type DiscoEntry } from "../../pages/artist";
import { useContextState } from "../../pages/cards";
import { CatalogueAction, openAlbum } from "../../pages/catalogue";
import { SORT_LABEL, discographyGroups, discographyPrefs, prefsPatch, releaseLine, typeOptions, validType, type DiscographyType, type SectionId } from "../../pages/discography";
import { useArtistDiscography } from "../../pages/discographySources";
import { useLidarrReady } from "../../pages/lidarrShared";
import { player } from "../../stores/player";
import { nav } from "../../stores/router";
import { updateSettings, useSettings } from "../../stores/settings";
import { PageFailed } from "./common";
import { Empty, GhostRows, PageFrame, Pills, Row } from "./kit";

function PlayAlbum({ album }: { album: Album }) {
  const { isContext, playing } = useContextState("album", album.id);
  return (
    <button
      type="button"
      className="ph-play ph-play--row"
      aria-label={playing ? `Pause ${album.title}` : `Play ${album.title}`}
      onClick={() => (isContext ? void player.toggle() : void player.playAlbum(album.id))}
    >
      {playing ? <Pause size={18} fill="currentColor" aria-hidden="true" /> : <Play size={18} fill="currentColor" aria-hidden="true" />}
    </button>
  );
}

export function PhoneDiscographyView({
  name,
  onTitle,
  entries,
  startType,
  artistLibraryId,
  onChanged,
}: {
  name: string | undefined;
  onTitle: () => void;
  entries: DiscoEntry[] | null;
  startType?: SectionId;
  artistLibraryId?: string | null;
  onChanged: () => void;
}) {
  const [type, setType] = useState<DiscographyType>(startType ?? "all");
  const saved = discographyPrefs(useSettings((s) => s.settings));
  const [sort, setSort] = useState(saved.sort);
  const lidarrReady = useLidarrReady();
  const shown = entries ? validType(entries, type) : type;
  const groups = entries ? discographyGroups(entries, shown, sort, saved.reverse) : null;
  const flipSort = () => {
    const next = sort === "date" ? "name" : "date";
    setSort(next);
    void updateSettings(prefsPatch({ sort: next }));
  };
  return (
    <PageFrame
      name="discography"
      title="Discography"
      kicker={
        name ? (
          <button type="button" className="ph-link-btn ph-disco-page__artist" onClick={onTitle}>
            {name}
          </button>
        ) : undefined
      }
    >
      {entries && (
        <>
          <Pills label="Release type" items={typeOptions(entries)} value={shown} onChange={setType} />
          <button type="button" className="ph-text-btn" onClick={flipSort} aria-label={`Sorted by ${SORT_LABEL[sort].toLowerCase()}. Sort by ${sort === "date" ? "name" : "release date"}`}>
            <ArrowUpDown size={16} aria-hidden="true" />
            {sort === "date" ? "Newest first" : "A to Z"}
          </button>
        </>
      )}
      {!groups ? (
        <GhostRows n={6} />
      ) : groups.length === 0 ? (
        <Empty title="No releases to show yet" />
      ) : (
        groups.map((g) => (
          <section key={g.section} aria-label={g.label}>
            {groups.length > 1 && <h2 className="ph-x__day">{g.label}</h2>}
            <ul className="ph-rows">
              {g.entries.map((e) =>
                e.album ? (
                  <Row
                    key={e.key}
                    art={<Art path={e.album.thumb} size={64} sleeve={{ title: e.album.title, artist: e.album.artistTitle }} />}
                    title={e.album.title}
                    sub={releaseLine(e)}
                    onOpen={() => nav.go({ name: "album", id: e.album!.id })}
                    end={<PlayAlbum album={e.album} />}
                  />
                ) : (
                  <Row
                    key={e.key}
                    className="ph-x__row--get"
                    label={`${e.title}, not in your library`}
                    art={<Art path={e.cat!.thumb} size={64} kind="album" sleeve={{ title: e.cat!.title, artist: e.cat!.artist }} />}
                    title={e.title}
                    sub={releaseLine(e)}
                    onOpen={() => openAlbum(e.cat!)}
                    end={<CatalogueAction album={e.cat!} lidarrReady={lidarrReady || !!e.missing} artistLibraryId={artistLibraryId} onChanged={onChanged} compact />}
                  />
                ),
              )}
            </ul>
          </section>
        ))
      )}
    </PageFrame>
  );
}

export function PhoneDiscography({ id, type, catalogue }: { id: string; type?: SectionId; catalogue?: boolean }) {
  return catalogue ? <CatalogueDisco key={`mb:${id}`} mbid={id} type={type} /> : <LibraryDisco key={id} id={id} type={type} />;
}

function LibraryDisco({ id, type }: { id: string; type?: SectionId }) {
  const { data, error, retry } = useLoad(() => backend.artistPage(id), [id]);
  const disco = useArtistDiscography(id, data?.artist.title, data?.sections);
  if (error && !data) return <PageFailed what="this discography" retry={retry} framed />;
  return (
    <PhoneDiscographyView
      name={data?.artist.title}
      onTitle={() => nav.go({ name: "artist", id })}
      entries={data ? disco.entries : null}
      startType={type}
      artistLibraryId={isCreditId(id) ? null : id}
      onChanged={disco.reload}
    />
  );
}

function CatalogueDisco({ mbid, type }: { mbid: string; type?: SectionId }) {
  const [version, setVersion] = useState(0);
  const { data, error, retry } = useLoad(() => backend.catalogueArtist(mbid), [mbid, version]);
  if (error && !data) return <PageFailed what="this discography" retry={retry} framed />;
  return (
    <PhoneDiscographyView
      name={data?.artist.name}
      onTitle={() => nav.go({ name: "catalogueArtist", mbid })}
      entries={data ? catalogueEntries(data.sections, data.popular) : null}
      startType={type}
      onChanged={() => setVersion((v) => v + 1)}
    />
  );
}
