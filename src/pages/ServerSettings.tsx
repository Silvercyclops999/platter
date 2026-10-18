import { Server } from "lucide-react";
import { useCallback, useEffect, useId, useState } from "react";
import { Button } from "../components/Button";
import { Dialog } from "../components/Dialog";
import { Skeleton } from "../components/Skeleton";
import { Switch } from "../components/Switch";
import { backend } from "../lib/backend";
import type { AppError, SavedServer, ServerLibrary, ServerState } from "../lib/types";
import { useApp } from "../stores/app";
import { useToasts } from "../stores/toasts";
import { useUi } from "../stores/ui";
import { AddServerFlow, type AddressKind } from "./AddServer";
import "./add-server.css";
import "./library-sources.css";

const KIND: Record<SavedServer["kind"], string> = { plex: "Plex", jellyfin: "Jellyfin", subsonic: "Subsonic" };
const STATE: Record<ServerState, string> = { unknown: "Connecting", online: "Online", offline: "Offline", signInAgain: "Sign in again" };

const toastError = (e: unknown, fallback: string) =>
  useToasts.getState().push({ kind: "error", message: (e as AppError)?.message || fallback }, 6000);

/** "Jellyfin, signed in as me" / "Plex". */
export function serverLine(s: SavedServer): string {
  return s.user ? `${KIND[s.kind]}, signed in as ${s.user}` : KIND[s.kind];
}

/** How many servers, for the row's control. */
export function ServerCount() {
  const servers = useApp((s) => s.servers);
  if (!servers) return <Skeleton width={80} height={20} />;
  return <span className="library-count">{servers.length === 1 ? "1 server" : `${servers.length} servers`}</span>;
}

function Libraries({ server }: { server: SavedServer }) {
  const [libs, setLibs] = useState<ServerLibrary[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let alive = true;
    backend.serverLibraries(server.tag).then(
      (l) => alive && setLibs(l),
      (e: AppError) => alive && setError(e.message || "Couldn't list this server's libraries."),
    );
    return () => {
      alive = false;
    };
  }, [server.tag]);
  if (error) return <p className="server-row__note">{error}</p>;
  if (!libs) return <Skeleton width="60%" height={20} />;
  if (libs.length === 0) return <p className="server-row__note">No music libraries.</p>;
  const toggle = (id: string, on: boolean) => {
    const next = libs.map((l) => (l.id === id ? { ...l, included: on } : l));
    const ids = next.filter((l) => l.included).map((l) => l.id);
    const prev = libs;
    setLibs(next);
    backend.serverLibrariesSet(server.tag, ids.length === next.length ? null : ids).catch((e) => {
      setLibs(prev);
      toastError(e, "Couldn't change the libraries.");
    });
  };
  return (
    <ul className="library-source__libs" aria-label={`Libraries on ${server.name}`}>
      {libs.map((l) => (
        <li key={l.id} className="library-lib">
          <span className="library-lib__title truncate">{l.title}</span>
          <Switch checked={l.included} label={`Include ${l.title} on ${server.name}`} onChange={(on) => toggle(l.id, on)} />
        </li>
      ))}
    </ul>
  );
}

/**
 * A Subsonic server at a new address (https now, another port, a proxy in
 * front of it). The core checks it's the same server, so its songs,
 * downloads, likes and history stay.
 */
function EditAddress({ server, onDone, onCancel }: { server: SavedServer; onDone: () => void; onCancel: () => void }) {
  const [url, setUrl] = useState(server.url ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const id = useId();
  const save = async () => {
    setBusy(true);
    setError(null);
    try {
      await backend.subsonicChangeAddress(server.tag, url.trim());
      onDone();
    } catch (e) {
      setError((e as AppError)?.message || "Couldn't change the address.");
    } finally {
      setBusy(false);
    }
  };
  return (
    <form
      className="add-server"
      onSubmit={(e) => {
        e.preventDefault();
        if (url.trim() && !busy) void save();
      }}
    >
      <div>
        <label className="add-server__label" htmlFor={`${id}-url`}>
          Server address
        </label>
        <input
          id={`${id}-url`}
          className="add-server__input"
          type="text"
          inputMode="url"
          autoComplete="url"
          spellCheck={false}
          value={url}
          autoFocus
          aria-invalid={error ? true : undefined}
          aria-describedby={error ? `${id}-err` : `${id}-hint`}
          onChange={(e) => setUrl(e.target.value)}
        />
      </div>
      <p className="add-server__hint" id={`${id}-hint`}>
        Use this when the same server moved: https instead of http, another port, or a proxy in front of it. Its songs, downloads,
        likes and history stay as they are.
      </p>
      {error && (
        <p className="add-server__error" id={`${id}-err`} role="alert">
          {error}
        </p>
      )}
      <div className="add-server__actions">
        <Button variant="ghost" type="button" onClick={onCancel}>
          Cancel
        </Button>
        <Button variant="primary" type="submit" disabled={!url.trim() || busy}>
          {busy ? "Checking…" : "Save"}
        </Button>
      </div>
    </form>
  );
}

function ServerRow({
  s,
  onSignInAgain,
  onEditAddress,
  onRemove,
}: {
  s: SavedServer;
  onSignInAgain: () => void;
  onEditAddress: () => void;
  onRemove: () => void;
}) {
  const byAddress = s.kind !== "plex";
  return (
    <li className="library-source" data-server={s.tag}>
      <div className="library-source__head">
        <Server size={20} aria-hidden="true" />
        <span className="library-source__name truncate">{s.name}</span>
        {s.primary && <span className="library-source__badge">Primary</span>}
        {s.state && (
          <span className={`library-source__state server-row__state ${s.state === "online" ? "is-online" : ""} ${s.state === "signInAgain" ? "is-warning" : ""}`}>
            {STATE[s.state]}
          </span>
        )}
      </div>
      <p className="server-row__note">{serverLine(s)}</p>
      {byAddress && !s.canTranscode && <p className="server-row__note">This account can't have songs converted, so they always play as they are.</p>}
      {byAddress && !s.canDownload && <p className="server-row__note">This account can't download songs. The server's owner can allow it.</p>}
      {byAddress && <Libraries server={s} />}
      <div className="server-row__actions">
        {!s.primary && (
          <Button size="sm" onClick={() => backend.serverSetPrimary(s.tag).catch((e) => toastError(e, "Couldn't change the primary server."))}>
            Make primary
          </Button>
        )}
        {byAddress && s.state === "signInAgain" && (
          <Button size="sm" variant="primary" onClick={onSignInAgain}>
            Sign in again
          </Button>
        )}
        {s.kind === "subsonic" && (
          <Button size="sm" variant="ghost" onClick={onEditAddress}>
            Edit address
          </Button>
        )}
        {byAddress && (
          <Button size="sm" variant="ghost" onClick={onRemove}>
            Remove
          </Button>
        )}
      </div>
    </li>
  );
}

/**
 * Settings, Library: the music servers. Add one (Plex, Jellyfin or Subsonic), choose
 * the primary server, include each server's music libraries, sign in again
 * or remove one.
 */
export function ServerList() {
  const servers = useApp((s) => s.servers);
  const plexSignedIn = useApp((s) => s.auth?.state === "signedIn");
  const [adding, setAdding] = useState<{ url?: string; kind?: AddressKind; tag?: string } | null>(null);
  const [editing, setEditing] = useState<SavedServer | null>(null);
  const [removing, setRemoving] = useState<SavedServer | null>(null);
  const reload = useCallback(() => {
    backend.serversSaved().then((l) => useApp.getState().setServers(l), () => {});
  }, []);
  useEffect(reload, [reload]);

  const done = () => {
    setAdding(null);
    reload();
  };

  return (
    <>
      {!servers ? (
        <div className="server-rows" aria-busy="true">
          <Skeleton width="100%" height={56} />
        </div>
      ) : (
        <ul className="server-rows" aria-label="Music servers">
          {servers.map((s) => (
            <ServerRow
              key={s.tag}
              s={s}
              onSignInAgain={() => setAdding({ url: s.url ?? undefined, kind: s.kind === "subsonic" ? "subsonic" : "jellyfin", tag: s.tag })}
              onEditAddress={() => setEditing(s)}
              onRemove={() => setRemoving(s)}
            />
          ))}
        </ul>
      )}
      <div className="server-row__actions" style={{ paddingLeft: 0, marginTop: "var(--space-2)" }}>
        <Button size="sm" onClick={() => setAdding({})}>
          Add a server
        </Button>
      </div>
      <Dialog open={!!adding} onClose={() => setAdding(null)} label={adding?.url ? "Sign in again" : "Add a server"} className="confirm">
        <h2 className="confirm__title">{adding?.url ? "Sign in again" : "Add a server"}</h2>
        {adding && (
          <AddServerFlow
            choose={!adding.url}
            url={adding.url}
            kind={adding.kind}
            tag={adding.tag}
            plexSignedIn={plexSignedIn}
            onPlex={() => {
              setAdding(null);
              useUi.getState().setSignInOpen(true);
            }}
            onDone={done}
            onCancel={() => setAdding(null)}
          />
        )}
      </Dialog>
      <Dialog open={!!editing} onClose={() => setEditing(null)} label={`Edit the address of ${editing?.name ?? ""}`} className="confirm">
        <h2 className="confirm__title">Edit address</h2>
        {editing && (
          <EditAddress
            server={editing}
            onCancel={() => setEditing(null)}
            onDone={() => {
              setEditing(null);
              reload();
            }}
          />
        )}
      </Dialog>
      <Dialog open={!!removing} onClose={() => setRemoving(null)} label={`Remove ${removing?.name ?? ""}`} className="confirm">
        <h2 className="confirm__title">Remove {removing?.name}?</h2>
        <p className="confirm__body">
          Its music leaves your library on this computer, along with songs downloaded from it. Nothing changes on the server.
        </p>
        <div className="confirm__actions">
          <Button variant="ghost" onClick={() => setRemoving(null)}>
            Cancel
          </Button>
          <Button
            variant="primary"
            onClick={() => {
              const s = removing;
              setRemoving(null);
              if (s) backend.serverRemove(s.tag).then(reload, (e) => toastError(e, "Couldn't remove the server."));
            }}
          >
            Remove
          </Button>
        </div>
      </Dialog>
    </>
  );
}
