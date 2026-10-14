import { Heart } from "lucide-react";
import "./likes.css";

/**
 * The Liked Songs cover, drawn here rather than from art: a heart on a warm
 * ivory gradient from Platter's palette. `fluid` fills its parent's width.
 */
export function LikedCover({ size = 48, fluid, className = "" }: { size?: number; fluid?: boolean; className?: string }) {
  const style = fluid ? undefined : { width: size, height: size, ["--liked-heart" as string]: `${Math.round(size * 0.4)}px` };
  return (
    <span className={`liked-cover ${fluid ? "liked-cover--fluid" : ""} ${className}`} style={style} aria-hidden="true">
      <Heart className="liked-cover__heart" fill="currentColor" strokeWidth={0} />
    </span>
  );
}
