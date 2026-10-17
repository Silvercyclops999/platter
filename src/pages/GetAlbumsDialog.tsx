/**
 * "Get albums" for an artist: asks once which kinds (studio albums ticked
 * by default; EPs, live albums and compilations if wanted), shows how many
 * and which, most popular first, then queues them. They go to Lidarr a few
 * at a time, the same paced way as Bring in my listening, each through
 * every check. The artist is never added to Lidarr with everything
 * monitored; each album is asked for on its own.
 */
import { CircleAlert } from "lucide-react";
import { useEffect, useId, useState } from "react";
import { Button } from "../components/Button";
import { Dialog } from "../components/Dialog";
import { Skeleton } from "../components/Skeleton";
import { backend } from "../lib/backend";
import type { AppError, GetAlbumsPlan } from "../lib/types";
import { useSettings } from "../stores/settings";
import { toast } from "../stores/toasts";
import "./lidarr-request.css";

export const GET_KINDS: { id: string; label: string }[] = [
  { id: "studio", label: "Studio albums" },
  { id: "ep", label: "EPs" },
  { id: "live", label: "Live albums" },
  { id: "compilation", label: "Compilations" },
];

/** Titles listed in the dialog before "and N more". */
const LISTED = 8;

const errText = (e: unknown) => (e && typeof e === "object" && "message" in e ? String((e as AppError).message) : "");

export function GetAlbumsDialog({ mbid, name, open, onClose }: { mbid: string; name: string; open: boolean; onClose: () => void }) {
  const preset = useSettings((s) => s.settings?.getAlbumsKinds) ?? ["studio"];
  const [kinds, setKinds] = useState<string[]>(preset);
  const [plan, setPlan] = useState<GetAlbumsPlan | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [sending, setSending] = useState(false);
  const id = useId();
  const key = kinds.join(",");

  useEffect(() => {
    if (open) setKinds(preset);
    // Only when it opens: the ticks are the user's from then on.
  }, [open]);

  useEffect(() => {
    if (!open) return;
    let alive = true;
    setPlan(null);
    setError(null);
    backend.catalogueGetAlbumsPlan(mbid, name, key ? key.split(",") : []).then(
      (p) => alive && setPlan(p),
      (e) => alive && setError(errText(e) || "Platter couldn't list their albums. Try again."),
    );
    return () => {
      alive = false;
    };
  }, [open, mbid, name, key]);

  const toggle = (k: string) => setKinds((l) => (l.includes(k) ? l.filter((x) => x !== k) : GET_KINDS.map((g) => g.id).filter((g) => g === k || l.includes(g))));
  const count = plan?.albums.length ?? 0;

  const send = async () => {
    setSending(true);
    try {
      const out = await backend.catalogueGetAlbums(mbid, name, kinds);
      toast.info(
        out.queued === 0
          ? `Nothing new to get from ${name}.`
          : `${out.queued} album${out.queued === 1 ? "" : "s"} by ${name} queued. They go to Lidarr a few at a time.`,
      );
      onClose();
    } catch (e) {
      setError(errText(e) || "They weren't queued. Try again.");
    } finally {
      setSending(false);
    }
  };

  return (
    <Dialog open={open} onClose={onClose} label={`Get albums by ${name}`} className="confirm lidarr-request get-albums">
      <h2 className="confirm__title">Get albums by {name}</h2>
      <fieldset className="get-albums__kinds">
        <legend className="confirm__body">Which kinds?</legend>
        {GET_KINDS.map((k) => (
          <label key={k.id} className="get-albums__kind" htmlFor={`${id}-${k.id}`}>
            <input id={`${id}-${k.id}`} type="checkbox" checked={kinds.includes(k.id)} onChange={() => toggle(k.id)} />
            {k.label}
          </label>
        ))}
      </fieldset>
      {error && (
        <p className="lidarr-request__error" role="alert">
          <CircleAlert size={16} aria-hidden="true" />
          <span>{error}</span>
        </p>
      )}
      {!plan && !error && <Skeleton height={44} radius="card" />}
      {plan && (
        <div className="get-albums__plan" role="status">
          <p className="confirm__body">
            {count === 0
              ? "Nothing to get: you have these already, or they're asked for."
              : `${count} album${count === 1 ? "" : "s"} you don't have, most popular first. They go to Lidarr a few at a time, each checked like a single Get.`}
          </p>
          {count > 0 && (
            <ol className="get-albums__list" aria-label="Albums to get">
              {plan.albums.slice(0, LISTED).map((a) => (
                <li key={a.releaseGroupId}>
                  {a.title}
                  {a.year ? <span className="get-albums__year"> · {a.year}</span> : null}
                </li>
              ))}
              {count > LISTED && <li className="get-albums__more">and {count - LISTED} more</li>}
            </ol>
          )}
        </div>
      )}
      <div className="confirm__actions">
        <Button variant="ghost" onClick={onClose}>
          Cancel
        </Button>
        <Button variant="primary" disabled={!plan || count === 0 || kinds.length === 0 || sending} onClick={() => void send()}>
          {sending ? "Queueing…" : count > 0 ? `Get ${count} album${count === 1 ? "" : "s"}` : "Get albums"}
        </Button>
      </div>
    </Dialog>
  );
}
