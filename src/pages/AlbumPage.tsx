import { Disc3, Ellipsis, List, Pause, Play, Rows3, Shuffle } from "lucide-react";
import { Fragment, useEffect, useRef, useState } from "react";
import { Art } from "../components/Art";
import { Badge, QualityBadge } from "../components/Badge";
import { IconButton } from "../components/Button";
import { openContextMenu } from "../components/ContextMenu";
import { bookmarkPageItem } from "../stores/curate";
import { yourStatsItem } from "../stores/stats";
import { StatsStrip } from "../components/StatsStrip";
import { shareItem } from "../components/ShareMenu";
import { addToPlaylistItem } from "../components/PlaylistPicker";
import { plexEditItem } from "../components/PlexEditMenu";
import { CoverPopup } from "../components/CoverPopup";
import { ShelfBadge, physicalItem } from "../components/PhysicalDialog";
import { DownloadButton } from "../components/DownloadButton";
import { downloadMenuItems } from "../stores/downloads";
import { Shelf, ShelfItem } from "../components/Shelf";
import { Skeleton } from "../components/Skeleton";
import { TrackList, TrackRow, TrackRowSkeleton } from "../components/TrackRow";
import { backend } from "../lib/backend";
import { formatDuration, formatLongDuration, plural, releaseTypeLabel } from "../lib/format";
import type { AlbumArchive, AlbumCredits } from "../lib/types";
import { formatArchiveDate } from "./era";
import { useAlbumCredits } from "../lib/useAlbumCredits";
import { useLoad } from "../lib/useLoad";
import { useArtTint } from "../lib/useTint";
import { usePins } from "../stores/pins";
import { player, usePlayer } from "../stores/player";
import { nav } from "../stores/router";
import { copyText, toast } from "../stores/toasts";
import { transcribe, useTranscribe } from "../stores/transcribe";
import { AlbumCard, useContextState } from "./cards";
import { usePageBar, useTitleStep } from "./detail";
import { EditionPicker } from "./EditionPicker";
import { ReleaseStatusLabel, useReleaseStatus } from "./ReleaseStatusLabel";
import {
  ALBUM_VIEW_LABEL,
  albumCreditGroups,
  discGroups,
  formatReleaseDate,
  labelLine,
  loadAlbumView,
  releaseDateFor,
  saveAlbumView,
  type AlbumView,
} from "./album";
import "./album.css";
import { CatalogueAction, MissingButton, NOT_IN_LIBRARY, RequestDialog, StoreLinks, catalogueKind, requestInput } from "./catalogue";
import { useLidarrReady } from "./lidarrShared";
import { LookForBetterDialog, UpgradeCard, upgradeMenuItems, useAlbumUpgrade } from "./LidarrUpgrade";
import { useRouter } from "../stores/router";
import type { CatTrack } from "../lib/types";

/** An album page; `catalogue`: `id` is a MusicBrainz release group the library doesn't have. */
export function AlbumPage({ id, catalogue }: { id: string; catalogue?: boolean }) {
  return catalogue ? <CatalogueAlbumView key={`mb:${id}`} rgid={id} /> : <LibraryAlbumView id={id} />;
}

function LibraryAlbumView({ id }: { id: string }) {
  const { data, error } = useLoad(() => backend.albumPage(id), [id]);
  const album = data?.album;
  useArtTint(album?.tint, album?.thumb);
  const nowTrackId = usePlayer((s) => s.state?.track?.id);
  const status = usePlayer((s) => s.state?.status);
  const shuffle = usePlayer((s) => s.state?.shuffledContexts?.includes(id) ?? false);
  const { isContext, playing: isThisPlaying } = useContextState("album", id);
  const pinned = usePins((s) => s.isPinned("album", id));
  const transcribeOn = useTranscribe((s) => !!s.status?.enabled && s.status.available);
  const mb = useAlbumCredits(album ? id : null);
  const releaseStatus = useReleaseStatus(album ? id : null);
  const play = () => (isContext ? player.toggle() : player.playAlbum(id));
  usePageBar(album?.title, isThisPlaying, play);
  const { header, column, step, compact } = useTitleStep(album?.title);
  const [coverOpen, setCoverOpen] = useState(false);
  const coverBtn = useRef<HTMLButtonElement>(null);
  const upgrade = useAlbumUpgrade(album ? id : null);
  const [lookingForBetter, setLookingForBetter] = useState(false);
  const [view, setView] = useState<AlbumView>(loadAlbumView);
  const chooseView = (v: AlbumView) => {
    setView(v);
    saveAlbumView(v);
  };
  const viewMenu = (el: HTMLElement) =>
    openContextMenu(
      "View as",
      (["compact", "list"] as AlbumView[]).map((v, i) => ({
        heading: i === 0 ? "View as" : undefined,
        label: ALBUM_VIEW_LABEL[v],
        checked: view === v,
        onSelect: () => chooseView(v),
      })),
      el,
    );

  if (error && !data) return <p className="page-pad page-error">Couldn't load this album: {error.message}</p>;

  const firstTrack = data?.tracks[0];
  const untagged = album && album.format === null;

  return (
    <div className="detail">
      <header ref={header} className={`detail-header ${compact ? "detail-header--compact" : ""}`}>
        <div className="detail-header__art">
          {album ? (
            <button
              ref={coverBtn}
              type="button"
              className="cover-button"
              aria-label="View the cover art"
              aria-haspopup="dialog"
              onClick={() => setCoverOpen(true)}
            >
              <Art path={album.thumb} size={232} alt="" />
            </button>
          ) : (
            <Skeleton width={232} height={232} />
          )}
        </div>
        <div ref={column} className="detail-header__text">
          {album ? (
            <>
              <span className="detail-header__type">{releaseTypeLabel(album)}</span>
              <h1 className="detail-header__title" data-step={step}>{album.title}</h1>
              <div className="detail-header__meta">
                {data!.artistThumb && <Art path={data!.artistThumb} size={24} round kind="artist" className="detail-header__artist-photo" />}
                <button type="button" className="link detail-header__artist" onClick={() => nav.go({ name: "artist", id: album.artistId })}>
                  {album.artistTitle}
                </button>
                {album.year && <span>• {album.year}</span>}
                <span>• {plural(data!.tracks.length, "song")},</span>
                <span className="detail-header__muted">{formatLongDuration(data!.totalDurationMs)}</span>
              </div>
              <div className="detail-header__badges">
                {firstTrack && <QualityBadge track={firstTrack} />}
                {untagged && (
                  <Badge tone="version" title="No release type tag yet; listed under Albums">
                    Untagged
                  </Badge>
                )}
                <ReleaseStatusLabel status={releaseStatus} />
                <ShelfBadge albumId={album.id} />
                {album.genres.slice(0, 2).map((g) => (
                  <Badge key={g}>{g}</Badge>
                ))}
              </div>
              <EditionPicker albumId={album.id} />
            </>
          ) : (
            <>
              <Skeleton width={60} height={12} />
              <Skeleton width="60%" height={48} style={{ margin: "12px 0" }} />
              <Skeleton width="40%" height={14} />
            </>
          )}
        </div>
      </header>

      {album && (
        <CoverPopup open={coverOpen} onClose={() => setCoverOpen(false)} path={album.thumb} title={album.title} returnFocus={coverBtn} />
      )}

      <div className="detail-body">
        {album && data!.archive && <EraBanner archive={data!.archive} artistId={album.artistId} />}
        <div className="action-row">
          <IconButton
            label={isThisPlaying ? `Pause ${album?.title ?? "album"}` : `Play ${album?.title ?? "album"}`}
            variant="accent"
            size="xl"
            disabled={!album}
            icon={isThisPlaying ? <Pause size={24} fill="currentColor" /> : <Play size={24} fill="currentColor" className="nudge-right" />}
            onClick={play}
          />
          <IconButton
            label={shuffle ? "Disable shuffle" : "Enable shuffle"}
            size="lg"
            pressed={shuffle}
            icon={<Shuffle size={22} />}
            onClick={() => player.setContextShuffle(id, !shuffle)}
          />
          <DownloadButton kind="album" id={id} title={album?.title ?? "this album"} disabled={!album} />
          <IconButton
            label={`More options for ${album?.title ?? "album"}`}
            size="lg"
            icon={<Ellipsis size={22} />}
            disabled={!album}
            onClick={(e) =>
              album &&
              openContextMenu(
                `Options for ${album.title}`,
                [
                  { label: "Add to queue", onSelect: () => void player.addToQueue(data!.tracks, album.thumb) },
                  addToPlaylistItem(() => data?.tracks.map((t) => t.id) ?? []),
                  ...downloadMenuItems({ kind: "album", id: album.id, title: album.title }),
                  { label: "Go to artist", onSelect: () => nav.go({ name: "artist", id: album.artistId }) },
                  { label: "Go to artist radio", onSelect: () => nav.go({ name: "radio", seed: `artist:${album.artistId}` }) },
                  shareItem("album", album.id, album.thumb),
                  bookmarkPageItem("album", album.id, album.title, { subtitle: album.artistTitle, thumb: album.thumb }),
                  yourStatsItem("album", album.id, album.title),
                  physicalItem({ albumId: album.id, title: album.title, artist: album.artistTitle, thumb: album.thumb }),
                  {
                    label: pinned ? "Unpin from sidebar" : "Pin to sidebar",
                    onSelect: () =>
                      usePins
                        .getState()
                        .toggle({ kind: "album", id: album.id, title: album.title, subtitle: `Album · ${album.artistTitle}`, thumb: album.thumb }),
                  },
                  {
                    label: "Copy album title",
                    onSelect: () => copyText(`${album.title} - ${album.artistTitle}`),
                  },
                  ...(transcribeOn
                    ? [{ label: "Transcribe songs without lyrics", onSelect: () => void transcribe.album(album.id) }]
                    : []),
                  plexEditItem({ id: album.id, kind: "album", title: album.title, thumb: album.thumb, artistTitle: album.artistTitle }),
                  ...upgradeMenuItems(upgrade.info, () => setLookingForBetter(true)),
                ],
                e.currentTarget,
              )
            }
          />
          <button
            type="button"
            className="pl-sort"
            aria-haspopup="menu"
            aria-label={`View as: ${ALBUM_VIEW_LABEL[view]}`}
            onClick={(e) => viewMenu(e.currentTarget)}
          >
            <span>{ALBUM_VIEW_LABEL[view]}</span>
            {view === "list" ? <List size={16} aria-hidden="true" /> : <Rows3 size={16} aria-hidden="true" />}
          </button>
        </div>

        <UpgradeCard upgrade={upgrade} onChanged={upgrade.reload} />
        {album && (
          <LookForBetterDialog
            open={lookingForBetter}
            albumId={album.id}
            title={album.title}
            quality={upgrade.info?.quality ?? null}
            confirmed={upgrade.info?.confirmed ?? null}
            onClose={() => setLookingForBetter(false)}
            onDone={() => {
              setLookingForBetter(false);
              upgrade.reload();
            }}
          />
        )}
        <StatsStrip kind="album" id={id} />
        <div className={`album-tracks album-tracks--${view}`}>
          <TrackList label={album ? `${album.title} tracks` : "Tracks"} showAlbum={false} showPlays>
            {data
              ? discGroups(data.tracks).map((g) => (
                  <Fragment key={g.disc ?? "all"}>
                    {g.disc !== null && (
                      <div className="album-disc" role="row" aria-label={`Disc ${g.disc}`}>
                        <span className="album-disc__cell" role="gridcell">
                          <span className="album-disc__icon">
                            <Disc3 size={16} aria-hidden="true" />
                          </span>
                          <span className="album-disc__label">Disc {g.disc}</span>
                        </span>
                      </div>
                    )}
                    {g.tracks.map((t) => (
                      <TrackRow
                        key={t.id}
                        track={t}
                        index={t.trackNumber}
                        showAlbum={false}
                        showPlays
                        showArtist={view === "list" && (!!t.trackArtist || t.artistId !== album!.artistId)}
                        active={t.id === nowTrackId}
                        playing={t.id === nowTrackId && status === "playing"}
                        onPlay={() => player.playAlbum(id, t.id)}
                      />
                    ))}
                  </Fragment>
                ))
              : Array.from({ length: 8 }, (_, i) => <TrackRowSkeleton key={i} showAlbum={false} showPlays />)}
          </TrackList>
        </div>

        {album && <AlbumFooter released={releaseDateFor(data!.released, mb)} studio={data!.studio || mb?.label} year={album.year} />}

        {mb && <AlbumCreditsSection credits={mb} />}

        {album && data!.moreByArtist.length > 0 && (
          <div className="detail-more">
            <Shelf
              title={`More by ${album.artistTitle}`}
              showAllLabel="See discography"
              onShowAll={() => nav.go({ name: "discography", id: album.artistId })}
            >
              {data!.moreByArtist.map((a) => (
                <ShelfItem key={a.id}>
                  <AlbumCard album={a} subtitle="year" />
                </ShelfItem>
              ))}
            </Shelf>
          </div>
        )}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------- an album that isn't in the library

/**
 * The same page for an album the library doesn't have: its cover and the
 * tracklist of its first official release, dimmed with nothing to play,
 * then Request (Lidarr), Add to Missing and store links. No previews.
 */
function CatalogueAlbumView({ rgid }: { rgid: string }) {
  const [version, setVersion] = useState(0);
  const { data, error } = useLoad(() => backend.catalogueAlbum(rgid), [rgid, version]);
  const lidarrReady = useLidarrReady();
  const [asking, setAsking] = useState(false);
  const album = data?.album;
  useArtTint(null, album?.thumb ?? undefined);
  const { header, column, step, compact } = useTitleStep(album?.title);
  const owned = album?.libraryAlbumId;
  useEffect(() => {
    if (owned) useRouter.getState().replace({ name: "album", id: owned });
  }, [owned]);

  if (error && !data) return <p className="page-pad page-error">Couldn't load this album: {error.message}</p>;
  const reload = () => setVersion((v) => v + 1);
  const total = data?.tracks.reduce((n, t) => n + (t.durationMs ?? 0), 0) ?? 0;
  const goArtist = () => {
    if (!data) return;
    if (data.artistLibraryId) nav.go({ name: "artist", id: data.artistLibraryId });
    else if (data.album.artistMbid) nav.go({ name: "catalogueArtist", mbid: data.album.artistMbid });
  };
  const discs = new Set(data?.tracks.map((t) => t.disc ?? 1)).size > 1;
  return (
    <div className="detail cat-page">
      <header ref={header} className={`detail-header ${compact ? "detail-header--compact" : ""}`}>
        <div className="detail-header__art">{album ? <Art path={album.thumb} size={232} alt="" /> : <Skeleton width={232} height={232} />}</div>
        <div ref={column} className="detail-header__text">
          {album ? (
            <>
              <span className="detail-header__type">
                {catalogueKind(album)} <span className="cat-tag">{NOT_IN_LIBRARY}</span>
              </span>
              <h1 className="detail-header__title" data-step={step}>
                {album.title}
              </h1>
              <div className="detail-header__meta">
                <button type="button" className="link detail-header__artist" onClick={goArtist}>
                  {album.artist}
                </button>
                {album.year && <span>• {album.year}</span>}
                {data!.tracks.length > 0 && <span>• {plural(data!.tracks.length, "song")},</span>}
                {total > 0 && <span className="detail-header__muted">{formatLongDuration(total)}</span>}
              </div>
            </>
          ) : (
            <>
              <Skeleton width={60} height={12} />
              <Skeleton width="60%" height={48} style={{ margin: "12px 0" }} />
              <Skeleton width="40%" height={14} />
            </>
          )}
        </div>
      </header>
      <div className="detail-body">
        <div className="action-row cat-actions">
          {album && <CatalogueAction album={album} lidarrReady={lidarrReady} onRequest={() => setAsking(true)} />}
          {album && <MissingButton album={album} onChanged={reload} />}
          <IconButton
            label={`More options for ${album?.title ?? "album"}`}
            size="lg"
            icon={<Ellipsis size={22} />}
            disabled={!album}
            onClick={(e) =>
              album &&
              openContextMenu(
                `Options for ${album.title}`,
                [
                  { label: "Go to artist", onSelect: goArtist },
                  { label: "Copy album title", onSelect: () => copyText(`${album.title} - ${album.artist}`) },
                  {
                    label: "Open on MusicBrainz",
                    onSelect: () => void backend.openInfoLink(`https://musicbrainz.org/release-group/${album.releaseGroupId}`).catch(() => {}),
                  },
                ],
                e.currentTarget,
              )
            }
          />
          {album && !lidarrReady && !album.refusal && <span className="cat-actions__note">Set up Lidarr in Settings to request it.</span>}
        </div>
        {album?.refusal && <p className="cat-actions__note">{album.refusal}</p>}
        {data && <StoreLinks links={data.links} title={data.album.title} />}
        {data ? (
          data.tracks.length > 0 ? (
            <ol className="cat-tracks" aria-label={`${data.album.title} tracks, ${NOT_IN_LIBRARY.toLowerCase()}`}>
              {data.tracks.map((t, i) => (
                <Fragment key={`${t.disc ?? 1}-${t.number}-${i}`}>
                  {discs && (i === 0 || data.tracks[i - 1].disc !== t.disc) && (
                    <li className="cat-disc" aria-hidden="true">
                      Disc {t.disc ?? 1}
                    </li>
                  )}
                  <CatalogueTrackRow t={t} />
                </Fragment>
              ))}
            </ol>
          ) : (
            <p className="cat-bio__none">MusicBrainz has no tracklist for this album yet.</p>
          )
        ) : (
          <TrackList label="Tracks" showAlbum={false}>
            {Array.from({ length: 8 }, (_, i) => (
              <TrackRowSkeleton key={i} showAlbum={false} />
            ))}
          </TrackList>
        )}
        {data && <AlbumFooter released={data.released} studio={data.label} year={data.album.year} />}
      </div>
      <RequestDialog
        input={asking && album ? requestInput(album, data?.artistLibraryId) : null}
        onClose={() => setAsking(false)}
        onDone={() => {
          setAsking(false);
          reload();
        }}
      />
    </div>
  );
}

function CatalogueTrackRow({ t }: { t: CatTrack }) {
  return (
    <li className="cat-track" aria-label={`${t.number}. ${t.title}${t.artist ? `, ${t.artist}` : ""}`}>
      <span className="cat-track__n">{t.number}</span>
      <span className="cat-track__text">
        <span className="cat-track__title truncate">{t.title}</span>
        {t.artist && <span className="cat-track__artist truncate">{t.artist}</span>}
      </span>
      <span className="cat-track__time">{t.durationMs ? formatDuration(t.durationMs) : ""}</span>
    </li>
  );
}

/**
 * Credits from MusicBrainz: each role once, with the people under it (library
 * artists link to their page), and where the credits came from.
 */
function AlbumCreditsSection({ credits }: { credits: AlbumCredits }) {
  const groups = albumCreditGroups(credits);
  if (!groups.length) return null;
  return (
    <section className="album-credits" aria-labelledby="album-credits-title">
      <h2 id="album-credits-title" className="section-title">
        Credits
      </h2>
      <dl className="album-credits__list">
        {groups.map((g) => (
          <div key={g.role} className="album-credits__row">
            <dt>{g.label}</dt>
            <dd>
              <ul className="album-credits__people">
                {g.people.map((p) => (
                  <li key={p.name}>
                    {p.artistId ? (
                      <button type="button" className="link" onClick={() => nav.go({ name: "artist", id: p.artistId! })}>
                        {p.name}
                      </button>
                    ) : (
                      p.name
                    )}
                  </li>
                ))}
              </ul>
            </dd>
          </div>
        ))}
      </dl>
      {credits.url && (
        <p className="album-credits__source">
          <button
            type="button"
            className="link"
            onClick={() => void backend.openInfoLink(credits.url!).catch(() => toast.error("Couldn't open the browser"))}
          >
            Credits from MusicBrainz
          </button>
        </p>
      )}
    </section>
  );
}

/** An unreleased build's era, date, edition and source, under the header. The era opens its page. */
function EraBanner({ archive, artistId }: { archive: AlbumArchive; artistId: string }) {
  const date = archive.date ? formatArchiveDate(archive.date) : null;
  return (
    <aside className="album-era" aria-label="From the archive">
      {archive.era ? (
        <button type="button" className="link album-era__name" onClick={() => nav.go({ name: "era", id: artistId, era: archive.era! })}>
          {archive.era} era
        </button>
      ) : (
        <span className="album-era__name">From the archive</span>
      )}
      {date && <span>{date}</span>}
      {archive.edition && <span>{archive.edition}</span>}
      {archive.source && <span className="album-era__source">Source: {archive.source}</span>}
    </aside>
  );
}

/** Under the tracklist: the full release date, then the label line. Hidden when there's neither. */
function AlbumFooter({ released, studio, year }: { released?: string | null; studio?: string | null; year: number | null }) {
  const date = formatReleaseDate(released);
  const label = labelLine(studio, year);
  if (!date && !label) return null;
  return (
    <footer className="album-footer">
      {date && <p className="album-footer__date">{date}</p>}
      {label && <p className="album-footer__label">{label}</p>}
    </footer>
  );
}
