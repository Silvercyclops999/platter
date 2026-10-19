/** A Home shelf's Show all on the phone: the shelf's full list, two across. Offline, only what can play. */
import { backend } from "../../lib/backend";
import type { HomeShelfId } from "../../lib/types";
import { useLoad } from "../../lib/useLoad";
import { useCachedLibrary } from "../../pages/cachedLibrary";
import { playableOffline, shelfTitle } from "../../pages/home";
import { useApp } from "../../stores/app";
import { nav } from "../../stores/router";
import { PageFailed } from "./common";
import { itemInfo } from "./common";
import { Empty, GhostRows, PageFrame, Tile, TileGrid } from "./kit";

export function PhoneHomeShelf({ shelf }: { shelf: HomeShelfId }) {
  const { data: full, error, retry } = useLoad(() => backend.homeShelf(shelf), [shelf]);
  const cachedOnly = useApp((s) => !!s.health?.cachedOnly);
  const { lib, failed } = useCachedLibrary();
  const data = full && cachedOnly && !failed ? (lib ? full.filter((i) => playableOffline(i, lib)) : undefined) : full;
  const title = shelfTitle(shelf);
  return (
    <PageFrame name="homeShelf" title={title}>
      {error && !data ? (
        <PageFailed what={title} retry={retry} />
      ) : !data ? (
        <GhostRows />
      ) : data.length === 0 ? (
        <Empty title="Nothing here yet" text="Play music from your library and this fills in." />
      ) : (
        <TileGrid label={title} round={shelf === "topArtists"}>
          {data.map((i) => {
            const info = itemInfo(i);
            return <Tile key={info.key} title={info.title} sub={info.sub || undefined} thumb={info.thumb} round={info.round} kind={info.kind} onOpen={() => nav.go(info.route)} />;
          })}
        </TileGrid>
      )}
    </PageFrame>
  );
}
