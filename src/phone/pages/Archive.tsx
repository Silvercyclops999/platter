/**
 * The archive on the phone: demos, mixtapes and unreleased records (the
 * library's albums of that type), two across, with the archive's own
 * filters (lossless, a decade, added recently) as pills.
 */
import { Art } from "../../components/Art";
import { FORMAT_LABEL, NO_CHIPS, chipsOn, decadeLabel, type LibChips } from "../../lib/libraryChips";
import { facetEntries, facetOptions, useAlbumFacts, useFacets } from "../../shell/FacetChips";
import { openEntry, useLibraryLoader, useLibraryModel } from "../../shell/useLibrary";
import { useLibraryView } from "../../stores/library";
import { Empty, GhostRows, PageFrame, Tile, TileGrid } from "./kit";

export function PhoneArchive() {
  useLibraryLoader();
  const model = useLibraryModel("", false);
  const items = useLibraryView((s) => s.items);
  const facts = useAlbumFacts(items);
  const [chips, setChips] = useFacets("archive");
  const options = facetOptions("archive", facts);
  const albums = model.all.filter((e) => e.kind === "album");
  const shown = facetEntries(albums, { ...chips, type: "archive" }, facts).sort((a, b) => (a.sortTitle ?? a.title).localeCompare(b.sortTitle ?? b.title, undefined, { sensitivity: "base", numeric: true }));
  const set = (c: Partial<LibChips>) => setChips({ ...chips, ...c });
  const pills: { key: string; label: string; on: boolean; run: () => void }[] = [
    ...options.formats.map((f) => ({ key: `f:${f}`, label: FORMAT_LABEL[f], on: chips.format === f, run: () => set({ format: chips.format === f ? null : f }) })),
    ...(options.recent ? [{ key: "recent", label: "Added recently", on: chips.recent, run: () => set({ recent: !chips.recent }) }] : []),
    ...options.decades.map((d) => ({ key: `d:${d}`, label: decadeLabel(d), on: chips.decade === d, run: () => set({ decade: chips.decade === d ? null : d }) })),
  ];
  return (
    <PageFrame name="archive" title="Archive" lede="Demos, mixtapes and records that were never released.">
      {pills.length > 0 && (
        <div className="ph-pills ph-pills--scroll ph-x__pills" role="group" aria-label="Filter the archive">
          {pills.map((p) => (
            <button key={p.key} type="button" className={`ph-chip ${p.on ? "is-on" : ""}`} aria-pressed={p.on} onClick={p.run}>
              {p.label}
            </button>
          ))}
        </div>
      )}
      {model.loading ? (
        <GhostRows />
      ) : shown.length === 0 ? (
        chipsOn(chips) ? (
          <Empty title="Nothing matches" text="No archive records match these filters." action={<button type="button" className="ph-pill" onClick={() => setChips(NO_CHIPS)}>Clear filters</button>} />
        ) : (
          <Empty title="The archive is empty" text="Records tagged as demos or unreleased show up here." />
        )
      ) : (
        <TileGrid label="Archive">
          {shown.map((e) => (
            <Tile key={e.id} title={e.title} sub={e.creator ?? undefined} art={<Art path={e.thumb} fluid requestSize={240} sleeve={{ title: e.title, artist: e.creator ?? "" }} />} onOpen={() => openEntry(e)} />
          ))}
        </TileGrid>
      )}
    </PageFrame>
  );
}
