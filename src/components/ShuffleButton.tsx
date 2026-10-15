import { Shuffle, Sparkles } from "lucide-react";
import { contextShuffleMode, shuffleLabel, smartOffered } from "../lib/shuffleMode";
import type { PlayContextKind } from "../lib/types";
import { player, usePlayer } from "../stores/player";
import { useSettings } from "../stores/settings";
import { IconButton } from "./Button";

/** The shuffle symbol, with a sparkle while smart shuffle is on. */
export function ShuffleIcon({ smart, size }: { smart: boolean; size: number }) {
  if (!smart) return <Shuffle size={size} />;
  return (
    <span className="smart-shuffle-icon">
      <Shuffle size={size} />
      <Sparkles size={Math.round(size * 0.55)} className="smart-shuffle-icon__spark" />
    </span>
  );
}

/**
 * A page's shuffle toggle for a context, playing or not (the same control as
 * the player bar's). Playlists and Liked Songs also offer smart shuffle.
 */
export function ContextShuffleButton({ id, kind, name }: { id: string; kind: PlayContextKind; name?: string | null }) {
  const mode = usePlayer((s) => contextShuffleMode(s.state, id));
  const smartAllowed = useSettings((s) => !!s.settings?.smartShuffle);
  const smart = smartAllowed && smartOffered(kind);
  const label = shuffleLabel(mode, smart || mode === "smart", name);
  return (
    <IconButton
      label={label}
      tooltip={label}
      size="lg"
      pressed={mode !== "off"}
      icon={<ShuffleIcon smart={mode === "smart"} size={22} />}
      onClick={() => player.cycleContextShuffle(id, kind, smartAllowed)}
    />
  );
}
