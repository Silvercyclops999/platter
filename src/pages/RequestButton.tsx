/**
 * Request: Get's place for someone the server is shared with. A click asks
 * the server's owner (docs/IPC.md `asks_request`); the button then follows
 * the request: Requested (it can be taken back), Getting it, Ready. A no
 * shows as Not this time, and it can be asked for again.
 */
import { Check, Clock, Download, Search, Send } from "lucide-react";
import { useState } from "react";
import { Button } from "../components/Button";
import type { AskState } from "../lib/types";
import { cancelAsk, requestItem, useCanRequest, useMyAsk } from "../stores/asks";
import { toast } from "../stores/toasts";
import type { GetTarget } from "./GetButton";
import "./lidarr-request.css";

/** What a Request button says for a request's state. */
export function askLabel(state: AskState, progress?: number | null): string {
  switch (state) {
    case "sending":
    case "requested":
      return "Requested";
    case "getting":
      return progress != null ? `Getting it ${progress}%` : "Getting it";
    case "ready":
      return "Ready";
    case "notFound":
      return "Not found yet";
    case "declined":
      return "Not this time";
    case "refused":
      return "Can't be requested";
    case "failed":
      return "Didn't come through";
    case "cancelled":
      return "Request";
  }
}

/** The short label a card's narrow pill uses. */
function askShortLabel(state: AskState, progress?: number | null): string {
  if (state === "getting") return progress != null ? `${progress}%` : "Getting it";
  if (state === "notFound") return "Not found";
  if (state === "refused") return "Can't request";
  return askLabel(state, progress);
}

export function RequestButton({
  album,
  libraryAlbumId = null,
  size = "sm",
  idleLabel = "Get",
  compact = false,
  song = null,
}: {
  album: GetTarget;
  libraryAlbumId?: string | null;
  size?: "sm" | "md";
  /** "Get the album" on a song becomes "Request the album". */
  idleLabel?: string;
  compact?: boolean;
  /** A song asked for with its album. */
  song?: string | null;
}) {
  const canRequest = useCanRequest();
  const mine = useMyAsk(album);
  const [busy, setBusy] = useState(false);
  const name = album.title;
  const label = idleLabel === "Get" ? "Request" : idleLabel.replace(/^Get\b/, "Request");

  const request = async () => {
    setBusy(true);
    const a = await requestItem({
      releaseGroupId: album.releaseGroupId,
      title: song ?? album.title,
      artist: album.artist,
      artistMbid: album.artistMbid ?? null,
      album: song ? album.title : null,
      song: !!song,
      thumb: album.thumb ?? null,
    });
    setBusy(false);
    if (a) toast.info(`Requested ${name}. You'll hear when it's here`);
  };

  if (libraryAlbumId) {
    return (
      <span className="lidarr-state lidarr-state--imported get-state">
        <Check size={14} aria-hidden="true" />
        In your library
      </span>
    );
  }
  const state = mine?.state;
  // A no or a refusal can be asked again (the reason may have passed).
  const settled = !state || state === "cancelled" || state === "declined" || state === "failed" || state === "refused";
  if (state && !settled) {
    const full = askLabel(state, mine?.progress);
    const Icon = state === "ready" ? Check : state === "getting" ? Download : state === "notFound" ? Search : Clock;
    const cls = state === "ready" ? "imported" : state === "getting" ? "downloading" : state === "notFound" ? "notFound" : "requested";
    return (
      <span className="get-row">
        <span
          className={`lidarr-state lidarr-state--${cls} get-state ${compact ? "get-state--compact" : ""}`}
          role="status"
          title={[compact ? full : null, mine?.note].filter(Boolean).join(". ") || undefined}
          aria-label={compact ? full : undefined}
        >
          <Icon size={14} aria-hidden="true" />
          <span className="get-state__text">{compact ? askShortLabel(state, mine?.progress) : full}</span>
        </span>
        {(state === "sending" || state === "requested") && !compact && (
          <Button size="sm" variant="ghost" onClick={() => mine && void cancelAsk(mine)} aria-label={`Take back your request for ${name}`}>
            Take back
          </Button>
        )}
      </span>
    );
  }
  if (!canRequest) return null;
  if (state === "declined" || state === "refused") {
    return (
      <span className="get-row">
        <span className={`lidarr-state get-state ${compact ? "get-state--compact" : ""}`} role="status">
          <span className="get-state__text" title={mine?.note ?? undefined}>
            {state === "declined" ? "Not this time" : "Couldn't be requested"}
          </span>
        </span>
        <Button size="sm" variant="ghost" disabled={busy} onClick={() => void request()} aria-label={`Request ${name} again`}>
          Request again
        </Button>
      </span>
    );
  }
  return (
    <Button
      size={size}
      variant={state === "failed" ? "secondary" : "primary"}
      icon={<Send size={14} />}
      disabled={busy}
      onClick={() => void request()}
      aria-label={`${state === "failed" ? "Request again" : label === "Request" ? "Request" : `${label},`} ${name}`}
      title={state === "failed" ? (mine?.note ?? undefined) : "Asks the server's owner. You'll hear when it's here"}
    >
      {state === "failed" ? "Request again" : label}
    </Button>
  );
}
