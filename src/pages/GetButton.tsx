/**
 * One-click Get for an album the library doesn't have. A click sends it
 * straight into the request path (every check runs in the core) after a
 * few seconds in which it can be undone; the button then follows the
 * request as Lidarr works on it. The request dialog opens only for the very
 * first request (its name is typed once), when a check says no (with why),
 * or every time when "Ask before each Get" is on.
 */
import { Check, CircleAlert, Clock, Download, LoaderCircle, RotateCw, Search } from "lucide-react";
import { Suspense, lazy, useEffect, useRef, useState } from "react";
import { Button } from "../components/Button";
import { backend } from "../lib/backend";
import type { AppError, LidarrGetOutcome, LidarrRequestInput, LidarrRequestState } from "../lib/types";
import { nav } from "../stores/router";
import { upsertRequest, useRequestFor } from "../stores/requests";
import { useSettings } from "../stores/settings";
import { toast } from "../stores/toasts";
import "./lidarr-request.css";

const LidarrRequestDialog = lazy(() => import("./LidarrRequest").then((m) => ({ default: m.LidarrRequestDialog })));

/** How long a Get can be undone before it's sent. */
export const GET_UNDO_MS = 3000;

const errText = (e: unknown) => (e && typeof e === "object" && "message" in e ? String((e as AppError).message) : "");

/** What a Get button says for a request state. */
export function getLabel(state: LidarrRequestState, progress?: number | null): string {
  switch (state) {
    case "queued":
      return "Waiting to send";
    case "requested":
      return "Requested";
    case "searching":
      return "Looking on your indexers";
    case "downloading":
      return progress != null ? `Downloading ${progress}%` : "Downloading";
    case "importing":
      return "Importing";
    case "imported":
      return "In your library";
    case "failed":
      return "Didn't come through";
    case "notFound":
      return "Not found yet";
  }
}

export interface GetTarget {
  releaseGroupId: string;
  title: string;
  artist: string;
  artistMbid?: string | null;
  artistId?: string | null;
}

export function GetButton({
  album,
  known = null,
  libraryAlbumId = null,
  refusal = null,
  lidarrReady,
  size = "sm",
  onChanged,
  idleLabel = "Get",
}: {
  album: GetTarget;
  /** The request state the list came with (the live one wins). */
  known?: LidarrRequestState | null;
  libraryAlbumId?: string | null;
  /** Why it can't be asked for, when that's known already. */
  refusal?: string | null;
  lidarrReady: boolean;
  size?: "sm" | "md";
  onChanged?: () => void;
  /** What the button says before it's clicked ("Get the album" on a song). */
  idleLabel?: string;
}) {
  const rec = useRequestFor(album.releaseGroupId);
  const askFirst = useSettings((s) => s.settings?.askBeforeGet === true);
  const [phase, setPhase] = useState<"idle" | "undo" | "checking">("idle");
  const [needs, setNeeds] = useState<LidarrGetOutcome["needs"]>(null);
  const [dialog, setDialog] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(
    () => () => {
      if (timer.current) clearTimeout(timer.current);
    },
    [],
  );
  const input: LidarrRequestInput = {
    releaseGroupId: album.releaseGroupId,
    title: album.title,
    artist: album.artist,
    artistId: album.artistId ?? null,
    artistMbid: album.artistMbid ?? null,
  };
  const state = rec?.state ?? known ?? null;
  const name = album.title;

  const send = async () => {
    timer.current = null;
    setPhase("checking");
    try {
      const out = await backend.lidarrGet(input);
      if (out.record) {
        upsertRequest(out.record);
        setNeeds(null);
        onChanged?.();
      } else if (out.needs) {
        setNeeds(out.needs);
        // The first request ever goes through the dialog (its name is typed once).
        if (out.needs.kind === "firstTime") setDialog(true);
      }
    } catch (e) {
      toast.error(errText(e) || `Couldn't get ${name}. Try again in a moment.`);
    } finally {
      setPhase("idle");
    }
  };

  const get = () => {
    if (askFirst) {
      setDialog(true);
      return;
    }
    setNeeds(null);
    setPhase("undo");
    timer.current = setTimeout(() => void send(), GET_UNDO_MS);
  };

  const undo = () => {
    if (timer.current) clearTimeout(timer.current);
    timer.current = null;
    setPhase("idle");
  };

  const searchAgain = async () => {
    if (!rec) return;
    try {
      upsertRequest(await backend.lidarrSearchAgain(rec.id));
    } catch (e) {
      toast.error(errText(e) || "Lidarr didn't take the search. Try again later.");
    }
  };

  const dialogEl = dialog ? (
    <Suspense fallback={null}>
      <LidarrRequestDialog
        open
        input={input}
        onClose={() => setDialog(false)}
        onDone={(r) => {
          setDialog(false);
          setNeeds(null);
          upsertRequest(r);
          onChanged?.();
        }}
      />
    </Suspense>
  ) : null;

  if (libraryAlbumId || state === "imported") {
    return (
      <span className="lidarr-state lidarr-state--imported get-state">
        <Check size={14} aria-hidden="true" />
        In your library
      </span>
    );
  }
  if (phase !== "idle") {
    return (
      <span className="get-row">
        <span className="lidarr-state lidarr-state--checking get-state" role="status">
          <LoaderCircle size={14} aria-hidden="true" className="get-spin" />
          Checking
        </span>
        {phase === "undo" && (
          <Button size="sm" variant="ghost" onClick={undo} aria-label={`Undo Get ${name}`}>
            Undo
          </Button>
        )}
      </span>
    );
  }
  const why = needs?.kind === "refused" ? needs.reason : !state || state === "failed" ? refusal : null;
  if (why) {
    return (
      <>
        <Button size={size} variant="secondary" icon={<CircleAlert size={14} />} title={why} onClick={() => setDialog(true)} aria-label={`${name} needs you: ${why}`}>
          Needs you
        </Button>
        {dialogEl}
      </>
    );
  }
  if (state && state !== "failed") {
    const Icon = state === "queued" ? Clock : state === "notFound" ? Search : Download;
    return (
      <span className="get-row">
        <span className={`lidarr-state lidarr-state--${state} get-state`} role="status" title={rec?.message ?? undefined}>
          <Icon size={14} aria-hidden="true" />
          {getLabel(state, rec?.progress)}
        </span>
        {state === "notFound" && rec && (
          <>
            <Button size="sm" variant="secondary" icon={<RotateCw size={14} />} onClick={() => void searchAgain()} aria-label={`Search again for ${name}`}>
              Search again
            </Button>
            <Button size="sm" variant="ghost" onClick={() => nav.go({ name: "search", query: `${album.artist} ${album.title}` })} aria-label={`Try another edition of ${name}`}>
              Try another edition
            </Button>
          </>
        )}
      </span>
    );
  }
  if (!lidarrReady) return null;
  return (
    <>
      <Button
        size={size}
        variant={state === "failed" ? "secondary" : "primary"}
        icon={<Download size={14} />}
        onClick={get}
        aria-label={`${state === "failed" ? "Get again" : idleLabel === "Get" ? "Get" : `${idleLabel},`} ${name}`}
        title={state === "failed" ? (rec?.message ?? undefined) : undefined}
      >
        {state === "failed" ? "Get again" : idleLabel}
      </Button>
      {dialogEl}
    </>
  );
}
