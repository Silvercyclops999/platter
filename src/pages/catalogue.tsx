/**
 * Pieces shared by the pages for music that isn't in the library: dimmed
 * album and artist cards that open catalogue pages, Request (through the
 * usual Lidarr preview and confirm), Add to Missing, store links and
 * "Not interested".
 */
import { Ban, Check, Clock, Download, ExternalLink, ListPlus } from "lucide-react";
import { Suspense, lazy, useEffect, useState, type MouseEvent } from "react";
import { Art } from "../components/Art";
import { Button } from "../components/Button";
import { openContextMenu } from "../components/ContextMenu";
import { backend } from "../lib/backend";
import { STORE_LINKS } from "../lib/importStrings";
import { isOffline } from "../lib/offline";
import { useApp } from "../stores/app";
import type { AppError, CatAlbum, CatArtist, DismissedArtist, LidarrRequestInput, StoreLinkSet } from "../lib/types";
import type { MenuItem } from "../stores/ui";
import { nav } from "../stores/router";
import { updateSettings, useSettings } from "../stores/settings";
import { toast, useToasts } from "../stores/toasts";
import { STATE_LABEL } from "./lidarrShared";
import "./lidarr-request.css";
import "./catalogue.css";

const LidarrRequestDialog = lazy(() => import("./LidarrRequest").then((m) => ({ default: m.LidarrRequestDialog })));

export const NOT_IN_LIBRARY = "Not in your library";

export const errText = (e: unknown) => (e && typeof e === "object" && "message" in e ? String((e as AppError).message) : "");

const KIND_LABEL: Record<string, string> = { album: "Album", ep: "EP", single: "Single" };

/** "Album", "EP", "Single", "Live album"... */
export function catalogueKind(a: Pick<CatAlbum, "kind" | "secondary">): string {
  const base = KIND_LABEL[a.kind] ?? "Album";
  const sec = a.secondary[0];
  if (!sec) return base;
  return `${sec.charAt(0).toUpperCase()}${sec.slice(1)} ${base.toLowerCase()}`;
}

export const SECTION_TITLE: Record<string, string> = {
  album: "Albums",
  ep: "EPs",
  single: "Singles",
  other: "Live albums and compilations",
};

function actionToast(message: string, label: string, run: () => void) {
  useToasts.getState().push({ kind: "info", message, action: { label, run } }, 6000);
}

// ---------------------------------------------------------------- not interested

export function useDismissed(): DismissedArtist[] {
  return useSettings((s) => s.settings?.dismissedArtists) ?? [];
}

/** "Not interested": the artist is never suggested again (synced; undo in Settings or from the toast). */
export function dismissArtist(a: { mbid: string; name: string }) {
  const list = useSettings.getState().settings?.dismissedArtists ?? [];
  if (list.some((d) => d.mbid === a.mbid)) return;
  void updateSettings({ dismissedArtists: [...list, { mbid: a.mbid, name: a.name }] });
  actionToast(`You won't see ${a.name} in suggestions`, "Undo", () => undismissArtist(a.mbid));
}

export function undismissArtist(mbid: string) {
  const list = useSettings.getState().settings?.dismissedArtists ?? [];
  void updateSettings({ dismissedArtists: list.filter((d) => d.mbid !== mbid) });
}

// ---------------------------------------------------------------- store links and Missing

export function openStore(url: string) {
  return backend.importOpenLink(url).catch((e) => toast.error(`Couldn't open the link${errText(e) ? `: ${errText(e)}` : ""}`));
}

export function StoreLinks({ links, title }: { links: StoreLinkSet; title: string }) {
  return (
    <div className="cat-stores" role="group" aria-label={`Buy ${title}`}>
      <span className="cat-stores__label">Buy or find it</span>
      {STORE_LINKS.map((st) => (
        <Button key={st.key} size="sm" variant="secondary" icon={<ExternalLink size={14} />} onClick={() => void openStore(links[st.key])} aria-label={`Search ${st.name} for ${title}`}>
          {st.name}
        </Button>
      ))}
    </div>
  );
}

export async function addToMissing(a: CatAlbum): Promise<boolean> {
  try {
    await backend.importMissingAlbumAdd({ releaseGroupId: a.releaseGroupId, title: a.title, artist: a.artist, artistMbid: a.artistMbid });
    actionToast(`Added ${a.title} to Missing`, "See Missing", () => nav.go({ name: "import", view: "missing" }));
    return true;
  } catch (e) {
    toast.error(errText(e) || "Couldn't add it to Missing");
    return false;
  }
}

export async function removeFromMissing(a: CatAlbum): Promise<boolean> {
  try {
    await backend.importMissingAlbumRemove(a.releaseGroupId);
    return true;
  } catch (e) {
    toast.error(errText(e) || "Couldn't take it off Missing");
    return false;
  }
}

/** The request input for a catalogue album (the artist named by MusicBrainz id). */
export function requestInput(a: CatAlbum, artistLibraryId?: string | null): LidarrRequestInput {
  return { releaseGroupId: a.releaseGroupId, title: a.title, artist: a.artist, artistId: artistLibraryId ?? null, artistMbid: a.artistMbid };
}

/** Where a catalogue album stands: in the library, requested, can't be requested, or Request. */
export function CatalogueAction({ album, lidarrReady, onRequest }: { album: CatAlbum; lidarrReady: boolean; onRequest: () => void }) {
  if (album.libraryAlbumId) {
    return (
      <span className="lidarr-state lidarr-state--imported">
        <Check size={14} aria-hidden="true" />
        In your library
      </span>
    );
  }
  if (album.request && album.request !== "failed") {
    return (
      <span className={`lidarr-state lidarr-state--${album.request}`}>
        {album.request === "queued" ? <Clock size={14} aria-hidden="true" /> : <Download size={14} aria-hidden="true" />}
        {STATE_LABEL[album.request]}
      </span>
    );
  }
  if (album.refusal) {
    return (
      <span className="lidarr-state cat-blocked" title={album.refusal}>
        <Ban size={14} aria-hidden="true" />
        Can't request
        <span className="sr-only">. {album.refusal}</span>
      </span>
    );
  }
  if (!lidarrReady) return null;
  return (
    <Button size="sm" onClick={onRequest} aria-label={`Request ${album.title}`}>
      {album.request === "failed" ? "Request again" : "Request"}
    </Button>
  );
}

/** The request preview and confirm for one catalogue album. */
export function RequestDialog({ input, onClose, onDone }: { input: LidarrRequestInput | null; onClose: () => void; onDone: () => void }) {
  if (!input) return null;
  return (
    <Suspense fallback={null}>
      <LidarrRequestDialog open input={input} onClose={onClose} onDone={onDone} />
    </Suspense>
  );
}

function openAlbum(a: CatAlbum) {
  if (a.libraryAlbumId) nav.go({ name: "album", id: a.libraryAlbumId });
  else nav.go({ name: "catalogueAlbum", id: a.releaseGroupId });
}

/** A dimmed album card (unless the library has it) with its action below. */
export function CatalogueAlbumCard({
  album,
  lidarrReady,
  onChanged,
  subtitle = "year",
}: {
  album: CatAlbum;
  lidarrReady: boolean;
  onChanged?: () => void;
  /** "year": year and type (an artist's page); "artist": the artist (search, Home). */
  subtitle?: "year" | "artist";
}) {
  const [asking, setAsking] = useState(false);
  const owned = !!album.libraryAlbumId;
  const kind = catalogueKind(album);
  const sub = subtitle === "artist" ? album.artist : `${album.year ? `${album.year} · ` : ""}${kind}`;
  const menu = (e: MouseEvent<HTMLElement>) => {
    e.preventDefault();
    const items: MenuItem[] = [
      { label: "Open", onSelect: () => openAlbum(album) },
      ...(album.artistMbid ? [{ label: "Go to artist", onSelect: () => nav.go({ name: "catalogueArtist", mbid: album.artistMbid! }) }] : []),
      ...(!owned && !album.inMissing ? [{ label: "Add to Missing", onSelect: () => void addToMissing(album).then((ok) => ok && onChanged?.()) }] : []),
      ...(!owned && album.inMissing ? [{ label: "Remove from Missing", onSelect: () => void removeFromMissing(album).then((ok) => ok && onChanged?.()) }] : []),
      ...(!owned && lidarrReady && !album.refusal && !album.request ? [{ label: "Request", onSelect: () => setAsking(true) }] : []),
    ];
    openContextMenu(`Options for ${album.title}`, items, e);
  };
  return (
    <div
      className={`card card--album cat-card ${owned ? "" : "missing-release"}`}
      aria-label={owned ? `${album.title} by ${album.artist}` : `${album.title} by ${album.artist}, ${NOT_IN_LIBRARY.toLowerCase()}`}
      onContextMenu={menu}
    >
      <div className="card__art missing-release__art">
        <Art path={album.thumb} fluid kind="album" />
      </div>
      <button type="button" className="card__link" onClick={() => openAlbum(album)} title={album.title}>
        <span className="card__title missing-release__title truncate">{album.title}</span>
      </button>
      <span className="card__subtitle missing-release__sub truncate">
        {sub}
      </span>
      <div className="missing-release__action">
        <CatalogueAction album={album} lidarrReady={lidarrReady} onRequest={() => setAsking(true)} />
      </div>
      <RequestDialog
        input={asking ? requestInput(album) : null}
        onClose={() => setAsking(false)}
        onDone={() => {
          setAsking(false);
          onChanged?.();
        }}
      />
    </div>
  );
}

/** An artist card: a library artist opens their page as usual; one outside it is dimmed and opens their catalogue page. */
export function CatalogueArtistCard({ artist, because, onDismiss }: { artist: CatArtist; because?: string | null; onDismiss?: () => void }) {
  const owned = !!artist.libraryId;
  const open = () => (owned ? nav.go({ name: "artist", id: artist.libraryId! }) : nav.go({ name: "catalogueArtist", mbid: artist.mbid }));
  const dismiss = () => {
    dismissArtist({ mbid: artist.mbid, name: artist.name });
    onDismiss?.();
  };
  const menu = (e: MouseEvent<HTMLElement>) => {
    e.preventDefault();
    openContextMenu(
      `Options for ${artist.name}`,
      [{ label: "Open", onSelect: open }, ...(owned ? [] : [{ label: "Not interested", onSelect: dismiss }])],
      e,
    );
  };
  return (
    <div className={`card card--artist cat-card ${owned ? "" : "missing-release"}`} onContextMenu={menu} aria-label={owned ? artist.name : `${artist.name}, ${NOT_IN_LIBRARY.toLowerCase()}`}>
      <div className="card__art missing-release__art">
        <Art path={artist.photo} fluid round kind="artist" />
      </div>
      <button type="button" className="card__link" onClick={open} title={artist.name}>
        <span className="card__title missing-release__title truncate">{artist.name}</span>
      </button>
      <span className="card__subtitle missing-release__sub truncate">{owned ? "Artist" : because ? `Because you play ${because}` : NOT_IN_LIBRARY}</span>
      {!owned && onDismiss !== undefined && (
        <div className="missing-release__action">
          <Button size="sm" variant="ghost" onClick={dismiss} aria-label={`Not interested in ${artist.name}`}>
            Not interested
          </Button>
        </div>
      )}
    </div>
  );
}

/** "Add to Missing" / "On your Missing list" for a catalogue album page. */
export function MissingButton({ album, onChanged }: { album: CatAlbum; onChanged: () => void }) {
  const [busy, setBusy] = useState(false);
  if (album.libraryAlbumId) return null;
  const run = async () => {
    setBusy(true);
    const ok = album.inMissing ? await removeFromMissing(album) : await addToMissing(album);
    setBusy(false);
    if (ok) onChanged();
  };
  return (
    <Button size="sm" variant="secondary" icon={album.inMissing ? <Check size={14} /> : <ListPlus size={14} />} aria-pressed={album.inMissing} disabled={busy} onClick={() => void run()}>
      {album.inMissing ? "On your Missing list" : "Add to Missing"}
    </Button>
  );
}

// ---------------------------------------------------------------- hooks

/** Platter can reach the internet (catalogue lookups are hidden otherwise). */
export function useOnline(): boolean {
  return useApp((s) => !isOffline(s.health) && !s.health?.cachedOnly);
}

/** Artists outside the library for a library artist's "Fans also like" (none while the setting is off or offline). */
export function useCatalogueFans(id: string | null, name: string | undefined): CatArtist[] {
  const on = useSettings((s) => s.settings?.fansAlsoLikeCatalogue !== false);
  const online = useOnline();
  const dismissed = useDismissed();
  const [list, setList] = useState<CatArtist[]>([]);
  useEffect(() => {
    if (!id || !name || !on || !online) {
      setList([]);
      return;
    }
    let alive = true;
    backend.catalogueFansAlsoLike(id, name).then(
      (l) => alive && setList(l),
      () => alive && setList([]),
    );
    return () => {
      alive = false;
    };
  }, [id, name, on, online]);
  const gone = new Set(dismissed.map((d) => d.mbid));
  return list.filter((a) => !gone.has(a.mbid));
}
