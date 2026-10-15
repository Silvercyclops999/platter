import { CircleAlert, Clock, Download } from "lucide-react";
import { useEffect, useState } from "react";
import { Art } from "../components/Art";
import { Button } from "../components/Button";
import { Dialog } from "../components/Dialog";
import { Skeleton } from "../components/Skeleton";
import { backend } from "../lib/backend";
import type { AppError, LidarrMissingItem, LidarrPreview, LidarrRequest, LidarrRequestInput } from "../lib/types";
import { STATE_LABEL } from "./lidarrShared";
import { toast } from "../stores/toasts";
import "./lidarr-request.css";

const errText = (e: unknown) => (e && typeof e === "object" && "message" in e ? String((e as AppError).message) : "");

const KIND_LABEL: Record<string, string> = { album: "Album", ep: "EP" };

/** A dimmed card for a release the library lacks, with Request (or where its request stands). */
export function MissingReleaseCard({
  item,
  artist,
  artistId,
  onChanged,
}: {
  item: LidarrMissingItem;
  artist: string;
  artistId: string;
  onChanged: () => void;
}) {
  const [open, setOpen] = useState(false);
  const kind = KIND_LABEL[item.kind] ?? "Album";
  return (
    <div className="card card--album missing-release" aria-label={`${item.title}, not in your library`}>
      <div className="card__art missing-release__art">
        <Art path={item.thumb} fluid kind="album" />
      </div>
      <span className="card__title missing-release__title truncate">{item.title}</span>
      <span className="card__subtitle missing-release__sub">
        {item.year ? `${item.year} · ` : ""}
        {kind} · Not in your library
      </span>
      <div className="missing-release__action">
        {item.request && item.request !== "failed" ? (
          <span className={`lidarr-state lidarr-state--${item.request}`}>
            {item.request === "queued" ? <Clock size={14} aria-hidden="true" /> : <Download size={14} aria-hidden="true" />}
            {STATE_LABEL[item.request]}
          </span>
        ) : (
          <Button size="sm" onClick={() => setOpen(true)} aria-label={`Request ${item.title}`}>
            Request
          </Button>
        )}
      </div>
      <LidarrRequestDialog
        open={open}
        input={{ releaseGroupId: item.releaseGroupId, title: item.title, artist, artistId }}
        onClose={() => setOpen(false)}
        onDone={() => {
          setOpen(false);
          onChanged();
        }}
      />
    </div>
  );
}

/**
 * Request confirmation: where the files will land (read from Lidarr after a
 * full check), whether the artist is new to Lidarr, and for the very first
 * request the album's name typed out. When Lidarr can't be reached the
 * request is kept and sent later.
 */
export function LidarrRequestDialog({
  open,
  input,
  onClose,
  onDone,
}: {
  open: boolean;
  input: LidarrRequestInput;
  onClose: () => void;
  onDone: (r: LidarrRequest) => void;
}) {
  const [preview, setPreview] = useState<LidarrPreview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [typed, setTyped] = useState("");
  const [sending, setSending] = useState(false);
  const { releaseGroupId, title, artist, artistId } = input;
  const artistMbid = input.artistMbid ?? null;

  useEffect(() => {
    if (!open) return;
    let alive = true;
    setPreview(null);
    setError(null);
    setTyped("");
    backend.lidarrRequestPreview({ releaseGroupId, title, artist, artistId, ...(artistMbid ? { artistMbid } : {}) }).then(
      (p) => alive && setPreview(p),
      (e) => alive && setError(errText(e) || "Platter couldn't check this request. Try again."),
    );
    return () => {
      alive = false;
    };
  }, [open, releaseGroupId, title, artist, artistId, artistMbid]);

  const fold = (s: string) => s.trim().replace(/\s+/g, " ").toLowerCase();
  const typedOk = !preview?.needsTyped || fold(typed) === fold(preview.title);

  const send = async () => {
    if (!preview || !typedOk) return;
    setSending(true);
    try {
      const r = await backend.lidarrRequest({
        releaseGroupId,
        title: preview.title,
        artist,
        artistId,
        ...(artistMbid ? { artistMbid } : {}),
        typed: preview.needsTyped ? typed : null,
      });
      toast.info(r.state === "queued" ? `Saved. ${preview.title} sends when Platter can reach Lidarr.` : `Requested ${preview.title} from Lidarr`);
      onDone(r);
    } catch (e) {
      setError(errText(e) || "The request didn't go through. Try again.");
    } finally {
      setSending(false);
    }
  };

  const typedId = `lidarr-typed-${releaseGroupId}`;
  return (
    <Dialog open={open} onClose={onClose} label={`Request ${title}`} className="confirm lidarr-request">
      <h2 className="confirm__title">Request {title}?</h2>
      {!preview && !error && (
        <div className="lidarr-request__loading" aria-busy="true">
          <p className="confirm__body">Checking Lidarr's setup first…</p>
          <Skeleton height={44} radius="card" />
        </div>
      )}
      {error && (
        <p className="lidarr-request__error" role="alert">
          <CircleAlert size={16} aria-hidden="true" />
          <span>{error}</span>
        </p>
      )}
      {preview && preview.existing && !error && (
        <p className="confirm__body">
          Lidarr already has {preview.title} ({STATE_LABEL[preview.existing].toLowerCase()}), so Platter won't send it again.
        </p>
      )}
      {preview && !preview.existing && (
        <>
          {!preview.reachable && (
            <p className="lidarr-request__queued">
              <Clock size={16} aria-hidden="true" />
              <span>
                Lidarr can't be reached right now. Platter keeps the request on this computer and sends it when it can reach Lidarr, after checking
                Lidarr's setup again.
              </span>
            </p>
          )}
          <p className="confirm__body">{preview.destinationIsGuess ? "Files should land in (Lidarr's folder at the last check):" : "Lidarr will download it into:"}</p>
          <p className="lidarr-request__path">{preview.destination ?? "Lidarr's download folder"}</p>
          <p className="confirm__body lidarr-request__note">
            {preview.addsArtist
              ? `Lidarr adds ${artist} with only this album to download. It won't look for their other albums or rename anything in your library.`
              : "Only this album is marked for download. Nothing in your library is renamed, retagged or moved."}
          </p>
          {preview.needsTyped && (
            <div className="lidarr-request__typed">
              <label htmlFor={typedId} className="lidarr-request__typed-label">
                This is your first request. Type the album's name to confirm.
              </label>
              <input
                id={typedId}
                className="setting-text lidarr-request__typed-field"
                value={typed}
                placeholder={preview.title}
                autoComplete="off"
                spellCheck={false}
                onChange={(e) => setTyped(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    e.preventDefault();
                    void send();
                  }
                }}
              />
            </div>
          )}
        </>
      )}
      <div className="confirm__actions">
        <Button variant="ghost" onClick={onClose}>
          {preview && !preview.existing ? "Cancel" : "Close"}
        </Button>
        {preview && !preview.existing && (
          <Button variant="primary" disabled={!typedOk || sending} onClick={() => void send()}>
            {sending ? "Sending…" : preview.reachable ? "Request" : "Save request"}
          </Button>
        )}
      </div>
    </Dialog>
  );
}

/** Settings: requests made on this device, with where each stands, and cancel for waiting ones. */
export function LidarrRequestsList() {
  const [list, setList] = useState<LidarrRequest[] | null>(null);
  useEffect(() => {
    let alive = true;
    backend.lidarrRequests(true).then(
      (l) => alive && setList(l),
      () => alive && setList([]),
    );
    return () => {
      alive = false;
    };
  }, []);
  if (!list) return <Skeleton height={44} radius="card" />;
  if (!list.length) return <p className="lidarr-requests__empty">No requests yet. Artist pages show Request on official releases you don't have.</p>;
  const cancel = async (r: LidarrRequest) => {
    try {
      setList(await backend.lidarrRequestCancel(r.id));
    } catch (e) {
      toast.error(errText(e) || "Couldn't cancel that.");
    }
  };
  return (
    <ul className="lidarr-requests" aria-label="Lidarr requests">
      {list.map((r) => {
        const canCancel = r.state === "queued" || r.state === "imported" || r.state === "failed" || r.alreadyInLidarr;
        return (
          <li key={r.id} className="lidarr-requests__item">
            <div className="lidarr-requests__text">
              <p className="lidarr-requests__title">
                {r.title} <span className="lidarr-requests__artist">· {r.artist}</span>
              </p>
              {r.auto && <p className="lidarr-requests__meta">{r.state === "failed" ? "Auto-request couldn't request it." : "Requested by auto-request."}</p>}
              {r.destination && <p className="lidarr-requests__meta">{r.destination}</p>}
              {r.message && <p className="lidarr-requests__meta">{r.message}</p>}
              {r.notes.map((n) => (
                <p key={n} className="lidarr-requests__meta">
                  {n}
                </p>
              ))}
            </div>
            <span className={`lidarr-state lidarr-state--${r.state}`}>{STATE_LABEL[r.state]}</span>
            {canCancel && (
              <Button size="sm" variant="ghost" onClick={() => void cancel(r)} aria-label={`${r.state === "queued" ? "Cancel" : "Clear"} ${r.title}`}>
                {r.state === "queued" ? "Cancel" : "Clear"}
              </Button>
            )}
          </li>
        );
      })}
    </ul>
  );
}
