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
export function ContextShuffleButton({
  id,
  kind,
  name,
  beforeShuffle,
}: {
  id: string;
  kind: PlayContextKind;
  name?: string | null;
  /** Asked before shuffle goes on (a mixed playlist); calls `go` to carry on. */
  beforeShuffle?: (go: () => void) => void;
}) {
  const mode = usePlayer((s) => contextShuffleMode(s.state, id));
  const smartAllowed = useSettings((s) => !!s.settings?.smartShuffle);
  const smart = smartAllowed && smartOffered(kind);
  const label = shuffleLabel(mode, smart || mode === "smart", name);
  const go = () => player.cycleContextShuffle(id, kind, smartAllowed);
  return (
    <IconButton
      label={label}
      tooltip={label}
      size="lg"
      text={mode === "smart" ? "Smart shuffle" : "Shuffle"}
      pressed={mode !== "off"}
      icon={<ShuffleIcon smart={mode === "smart"} size={22} />}
      onClick={() => (mode === "off" && beforeShuffle ? beforeShuffle(go) : go())}
    />
  );
}
