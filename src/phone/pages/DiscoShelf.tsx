/**
 * The discography shelf on the phone's artist pages: pills for the release
 * types (the pick is remembered, as on a computer), then one sideways row
 * of records, the library's and the ones you can get alike. A record you
 * can get has Get under it, which follows the request as Lidarr works.
 */
import { useState } from "react";
import { Art } from "../../components/Art";
import { activeChip, discographyChips, discographyItems, type DiscoEntry, type DiscographyFilter } from "../../pages/artist";
import { CatalogueAction, openAlbum } from "../../pages/catalogue";
import { catalogueKind } from "../../lib/format";
import { nav } from "../../stores/router";
import { updateSettings, useSettings } from "../../stores/settings";
import { Pills } from "./kit";
import { editionsNote, groupEditions } from "../editions";
import { useFileTags } from "../../stores/fileTags";
import type { DiscographyItem } from "../../pages/artist";
import type { TagIndex } from "../../lib/types";

/** Library editions of one record fold into one card with a count; records you can get stay as they are. */
export function foldEditions(items: DiscographyItem[], index?: TagIndex | null): (DiscographyItem & { editions: number })[] {
  const lib = items.filter((i) => i.entry.album);
  const groups = groupEditions(
    lib.map((i) => ({ id: i.entry.album!.id, title: i.entry.album!.title, viewCount: i.entry.album!.viewCount, item: i })),
    index,
  );
  const lead = new Map(groups.map((g) => [g.all[0].item, g.all.length]));
  const folded = new Set(groups.flatMap((g) => g.all.slice(1).map((x) => x.item)));
  return items
    .filter((i) => !folded.has(i))
    .map((i) => ({ ...i, editions: lead.get(i) ?? 1, latest: i.latest || (groups.find((g) => g.all.some((x) => x.item === i))?.all.some((x) => x.item.latest) ?? false) }));
}

export function DiscoShelf({
  entries,
  lidarrReady,
  artistLibraryId,
  onChanged,
  onShowAll,
}: {
  entries: DiscoEntry[];
  lidarrReady: boolean;
  artistLibraryId?: string | null;
  onChanged?: () => void;
  onShowAll: (f: DiscographyFilter) => void;
}) {
  const remembered = useSettings((s) => s.settings?.discographyChip);
  const grouping = useSettings((s) => s.settings?.groupEditions !== false);
  const tagIndex = useFileTags((s) => s.index);
  const index = grouping ? tagIndex : null;
  const [picked, setPicked] = useState<DiscographyFilter | null>(null);
  const chips = discographyChips(entries);
  const filter = activeChip(chips, picked ?? remembered);
  const pick = (id: DiscographyFilter) => {
    if (id === filter) return;
    setPicked(id);
    void updateSettings({ discographyChip: id });
  };
  if (!entries.length) return null;
  return (
    <section className="ph-disco" aria-label="Discography">
      <div className="ph-disco__head">
        <h2 className="ph-shelf__title">Discography</h2>
        <button type="button" className="ph-link-btn" onClick={() => onShowAll(filter)}>
          See all
        </button>
      </div>
      <Pills label="Release type" items={chips} value={filter} onChange={pick} />
      <div className="ph-shelf__row" role="list" aria-label="Releases">
        {(grouping ? foldEditions(discographyItems(entries, filter), index) : discographyItems(entries, filter).map((i) => ({ ...i, editions: 1 }))).map(({ entry: e, latest, editions }) => {
          if (e.album) {
            const a = e.album;
            return (
              <div key={e.key} role="listitem" className="ph-disco__tile">
                <button type="button" className="ph-card-tile" onClick={() => nav.go({ name: "album", id: a.id })}>
                  <Art path={a.thumb} size={136} sleeve={{ title: a.title, artist: a.artistTitle }} />
                  <span className="ph-card-tile__title">{a.title}</span>
                  <span className="ph-card-tile__sub">
                    {[latest ? "Latest release" : a.year ? String(a.year) : e.sectionLabel, editions > 1 ? editionsNote(editions) : null].filter(Boolean).join(" · ")}
                  </span>
                </button>
              </div>
            );
          }
          const c = e.cat!;
          return (
            <div key={e.key} role="listitem" className="ph-disco__tile is-catalogue" aria-label={`${c.title}, not in your library`}>
              <button type="button" className="ph-card-tile" onClick={() => openAlbum(c)}>
                <Art path={c.thumb} size={136} kind="album" sleeve={{ title: c.title, artist: c.artist }} />
                <span className="ph-card-tile__title">{c.title}</span>
                <span className="ph-card-tile__sub">{[latest ? "Latest release" : c.year ? String(c.year) : null, catalogueKind(c)].filter(Boolean).join(" · ")}</span>
              </button>
              <div className="ph-get">
                <CatalogueAction album={c} lidarrReady={lidarrReady || !!e.missing} artistLibraryId={artistLibraryId} onChanged={onChanged} compact />
              </div>
            </div>
          );
        })}
      </div>
    </section>
  );
}
