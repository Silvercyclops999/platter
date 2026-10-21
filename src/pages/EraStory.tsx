/**
 * The top of an era page, on the computer and the phone: one plain line of
 * facts (when it ran, how many releases, how many songs unreleased). The
 * releases themselves follow as sleeves; no timeline, no sentence.
 */
import { plural } from "../lib/format";
import type { EraPage } from "../lib/types";
import { eraRange } from "./era";
import "./ledger.css";

/** "Sep 2018 to Feb 2019, 3 releases, 6 songs unreleased." */
export function eraLine(data: EraPage): string {
  const era = data.era;
  if (era.trackCount === 0) return "No songs here yet.";
  const from = eraRange({ start: era.start });
  const to = era.end ? eraRange({ start: era.end }) : from;
  const unreleased = data.groups.reduce((n, g) => n + g.tracks.filter((t) => t.songStatus && t.songStatus !== "released").length, 0);
  const parts = [
    from ? (to && to !== from ? `${from} to ${to}` : from) : null,
    plural(era.albumCount, "release"),
    unreleased === 0 ? null : unreleased >= era.trackCount ? "all unreleased" : `${plural(unreleased, "song")} unreleased`,
  ].filter(Boolean);
  return `${parts.join(", ")}.`;
}

export function EraStory({ data, className = "" }: { data: EraPage; className?: string }) {
  return (
    <p className={`era-story ${className}`} data-testid="era-story">
      {eraLine(data)}
    </p>
  );
}
