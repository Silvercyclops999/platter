import { Bookmark, Boxes, Heart, Orbit } from "lucide-react";
import { StatsGlyph } from "./StatsGlyph";
import "./likes.css";

/**
 * The Liked Songs cover, drawn here rather than from art: a heart on a warm
 * ivory gradient from Platter's palette. `fluid` fills its parent's width.
 * Bookmarks share it; Stats gets its own symbol on a tile in the accent colour.
 * Pinned pages (Crates, Sound map) draw their line icon on the same tile.
 */
export function LikedCover({ size = 48, fluid, className = "", icon = "heart" }: { size?: number; fluid?: boolean; className?: string; icon?: "heart" | "bookmark" | "stats" | "crates" | "soundMap" }) {
  const style = fluid ? undefined : { width: size, height: size, ["--liked-heart" as string]: `${Math.round(size * 0.4)}px` };
  if (icon === "stats")
    return (
      <span className={`liked-cover liked-cover--stats ${fluid ? "liked-cover--fluid" : ""} ${className}`} style={style} aria-hidden="true" data-icon="stats">
        <StatsGlyph className="liked-cover__glyph" size="100%" />
      </span>
    );
  if (icon === "crates" || icon === "soundMap") {
    const Line = icon === "crates" ? Boxes : Orbit;
    return (
      <span className={`liked-cover ${fluid ? "liked-cover--fluid" : ""} ${className}`} style={style} aria-hidden="true" data-icon={icon}>
        <Line className="liked-cover__heart" strokeWidth={1.75} />
      </span>
    );
  }
  const Glyph = icon === "bookmark" ? Bookmark : Heart;
  return (
    <span className={`liked-cover ${fluid ? "liked-cover--fluid" : ""} ${className}`} style={style} aria-hidden="true">
      <Glyph className="liked-cover__heart" fill="currentColor" strokeWidth={0} />
    </span>
  );
}
