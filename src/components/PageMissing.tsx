import { Disc3, ListMusic, MicVocal, SearchX } from "lucide-react";
import type { AppError } from "../lib/types";
import { nav, useCanGoBack } from "../stores/router";
import { Button } from "./Button";
import { LoadError } from "./LoadError";

const ICON: Record<string, typeof SearchX> = { album: Disc3, artist: MicVocal, playlist: ListMusic, "smart playlist": ListMusic };

/**
 * A page whose album, artist, playlist (or mix, Blend...) couldn't be
 * loaded. One that isn't here reads "This album isn't in your library", laid
 * out like the other empty pages (an icon, the line, why, a way back); other
 * errors show the shared load error with Try again. Never shows a raw id.
 */
export function PageMissing({
  what,
  error,
  missing,
  reason,
  onRetry,
}: {
  what: string;
  error: AppError;
  /** The line itself, when "This … isn't in your library" doesn't fit. */
  missing?: string;
  /** The line under it; album, artist and playlist pages have one already. */
  reason?: string;
  onRetry?: () => void;
}) {
  const canGoBack = useCanGoBack();
  const back = () => (canGoBack ? nav.back() : nav.go({ name: "home" }));
  if (error.kind !== "notFound") return <LoadError what={`this ${what}`} error={error} onRetry={onRetry} />;
  const Icon = ICON[what] ?? SearchX;
  const why = reason ?? (missing ? undefined : "It may have been removed from your server, or it's in a library Platter isn't showing.");
  return (
    <div className="page-pad">
      <div className="empty-state page-missing" role="status">
        <Icon size={40} strokeWidth={1.5} aria-hidden="true" />
        <h1 className="empty-state__title">{missing ?? `This ${what} isn't in your library`}</h1>
        {why && <p className="empty-state__text">{why}</p>}
        <div className="empty-state__actions">
          <Button variant="primary" size="sm" onClick={back}>
            {canGoBack ? "Back" : "Go home"}
          </Button>
        </div>
      </div>
    </div>
  );
}
