/**
 * Every route's phone page, in one table. The type makes the table name
 * every route the router has, so a new route can't reach the phone without
 * a page of its own.
 */
import { lazy, Suspense, type ReactNode } from "react";
import type { Route } from "../stores/router";
import { PhoneAlbum } from "./pages/Album";
import { PhoneArchive } from "./pages/Archive";
import { PhoneArtist } from "./pages/Artist";
import { PhoneArtistShelf } from "./pages/ArtistShelf";
import { PhoneBlendNew } from "./pages/BlendNew";
import { PhoneBookmarks } from "./pages/Bookmarks";
import { PhoneCatalogueAlbum } from "./pages/CatalogueAlbum";
import { PhoneCatalogueArtist } from "./pages/CatalogueArtist";
import { BackBar, PageLoading } from "./pages/common";
import { PhoneCrate, PhoneCrates } from "./pages/Crates";
import { PhoneCredits } from "./pages/Credits";
import { PhoneDevices } from "./pages/Devices";
import { PhoneDiary } from "./pages/Diary";
import { PhoneDiscography } from "./pages/Discography";
import { PhoneDoctor } from "./pages/Doctor";
import { PhoneEra } from "./pages/Era";
import { PhoneHome } from "./pages/Home";
import { PhoneHomeShelf } from "./pages/HomeShelf";
import { PhoneImport } from "./pages/Import";
import { PhoneLibrary } from "./pages/Library";
import { PhoneLineage } from "./pages/Lineage";
import { PhoneLyricsRoute } from "./pages/LyricsRoute";
import { PhoneMix } from "./pages/Mix";
import { PhoneMixes } from "./pages/Mixes";
import { PhoneNewToYou } from "./pages/NewToYou";
import { PhoneOnThisDay } from "./pages/OnThisDay";
import { PhoneLiked, PhonePlaylist } from "./pages/Playlist";
import { PhoneRecents } from "./pages/Recents";
import { PhoneReleaseRadar } from "./pages/ReleaseRadar";
import { PhoneSearch } from "./pages/Search";
import { PhoneShelf } from "./pages/Shelf";
import { PhoneSoundMap } from "./pages/SoundMap";
import { PhoneStats } from "./pages/Stats";
import { PhoneTopSongs } from "./pages/TopSongs";
import { PhoneWhatsNew } from "./pages/WhatsNew";
import { PhoneYou } from "./pages/You";
import "./pages/parity.css";

const SettingsPage = lazy(() => import("../pages/SettingsPage").then((m) => ({ default: m.SettingsPage })));

type Pages = { [N in Route["name"]]: (r: Extract<Route, { name: N }>) => ReactNode };

export const PHONE_PAGES: Pages = {
  home: () => <PhoneHome />,
  search: () => <PhoneSearch />,
  library: () => <PhoneLibrary />,
  artist: (r) => <PhoneArtist id={r.id} />,
  discography: (r) => <PhoneDiscography id={r.id} type={r.type} catalogue={r.catalogue} />,
  lineage: (r) => <PhoneLineage key={r.id} id={r.id} />,
  era: (r) => <PhoneEra key={`${r.id}:${r.era}`} artistId={r.id} name={r.era} />,
  album: (r) => <PhoneAlbum id={r.id} />,
  playlist: (r) => <PhonePlaylist id={r.id} />,
  liked: () => <PhoneLiked />,
  settings: (r) => (
    <div className="ph-page ph-settings">
      <BackBar />
      <Suspense fallback={<PageLoading />}>
        <SettingsPage page={r.page} />
      </Suspense>
    </div>
  ),
  recents: () => <PhoneRecents />,
  whatsNew: () => <PhoneWhatsNew />,
  releaseRadar: () => <PhoneReleaseRadar />,
  lyrics: () => <PhoneLyricsRoute />,
  homeShelf: (r) => <PhoneHomeShelf shelf={r.shelf} />,
  artistShelf: (r) => <PhoneArtistShelf id={r.id} shelf={r.shelf} />,
  // Saved links to the old All-Time mix open the All-Time list.
  mix: (r) => (r.id === "topSongs" ? <PhoneTopSongs key="all" year={null} /> : <PhoneMix key={r.id} id={r.id} />),
  radio: (r) => <PhoneMix key={`radio-${r.seed}`} seed={r.seed} />,
  bookmarks: () => <PhoneBookmarks />,
  stats: (r) => <PhoneStats tab={r.tab} month={r.month} friend={r.friend} />,
  profile: () => <PhoneYou />,
  topSongs: (r) => <PhoneTopSongs key={r.year ?? "all"} year={r.year ?? null} />,
  import: (r) => <PhoneImport key={r.view ?? "home"} view={r.view} />,
  doctor: () => <PhoneDoctor />,
  devices: () => <PhoneDevices />,
  soundMap: (r) => <PhoneSoundMap key={r.album ?? "all"} album={r.album} />,
  crates: (r) => <PhoneCrates kind={r.kind} />,
  crate: (r) => <PhoneCrate kind={r.kind} crateKey={r.key} />,
  catalogueArtist: (r) => <PhoneCatalogueArtist key={r.mbid} mbid={r.mbid} />,
  catalogueAlbum: (r) => <PhoneCatalogueAlbum key={r.id} id={r.id} />,
  newToYou: () => <PhoneNewToYou />,
  onThisDay: () => <PhoneOnThisDay />,
  shelf: () => <PhoneShelf />,
  diary: (r) => <PhoneDiary edit={r.edit} />,
  blendNew: () => <PhoneBlendNew />,
  credits: (r) => <PhoneCredits key={r.person} person={r.person} />,
  mixes: () => <PhoneMixes />,
  archive: () => <PhoneArchive />,
};

/** The phone page for a route. */
export function PhoneRoutePage({ route }: { route: Route }) {
  const page = PHONE_PAGES[route.name] as (r: Route) => ReactNode;
  return <>{page(route)}</>;
}
