import { Server } from "lucide-react";
import { useEffect, useState } from "react";
import { Button } from "../components/Button";
import { Skeleton } from "../components/Skeleton";
import { backend } from "../lib/backend";
import type { AppError, PinInfo, ServerInfo } from "../lib/types";
import { useApp } from "../stores/app";
import { Logo } from "../shell/Logo";
import { WindowBar } from "../shell/TopBar";
import { useUi } from "../stores/ui";
import { AddServerFlow } from "./AddServer";

/** Welcome screen: Sign in with Plex → PIN code → waiting / cancel, or add a Jellyfin server. */
export function SignInPage({ onBack }: { onBack?: () => void } = {}) {
  const auth = useApp((s) => s.auth);
  const [jellyfin, setJellyfin] = useState(false);
  const [pin, setPin] = useState<PinInfo | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const pending = auth?.state === "pending" || !!pin;

  useEffect(() => {
    if (auth?.state === "signedOut") setPin(null);
  }, [auth?.state]);

  const start = async () => {
    setBusy(true);
    setError(null);
    try {
      setPin(await backend.authStart());
    } catch (e) {
      setError((e as AppError).message ?? "Couldn't reach plex.tv");
    } finally {
      setBusy(false);
    }
  };
  const cancel = async () => {
    setPin(null);
    await backend.authCancel().catch(() => {});
  };

  return (
    <div className="welcome">
      <WindowBar />
      <div className="welcome__panel" role="main">
        <Logo size={64} />
        <h1 className="welcome__title">{auth?.reauthRequired ? "Sign in again" : onBack ? "Sign in with Plex" : "Welcome to Platter"}</h1>
        {jellyfin ? (
          <>
            <p className="welcome__lead">Add your Jellyfin server. You'll approve Platter with a code, or sign in with your password.</p>
            <AddServerFlow
              onDone={() => {
                useUi.getState().setSignInOpen(false);
                void backend.serversSaved().then((l) => useApp.getState().setServers(l), () => {});
              }}
              onCancel={() => setJellyfin(false)}
            />
          </>
        ) : !pending ? (
          <>
            <p className="welcome__lead">
              {auth?.reauthRequired
                ? "Plex asked you to sign in again. Music saved on this computer keeps playing in the meantime."
                : onBack
                  ? "Add your Plex music next to the servers you already have."
                  : "Your music library, beautifully played. Sign in with Plex or add your Jellyfin server to start listening."}
            </p>
            <Button variant="primary" size="lg" onClick={start} disabled={busy} autoFocus>
              Sign in with Plex
            </Button>
            {!onBack && (
              <Button variant="secondary" size="lg" onClick={() => setJellyfin(true)}>
                Use a Jellyfin server
              </Button>
            )}
            {onBack && (
              <Button variant="ghost" onClick={onBack}>
                Not now
              </Button>
            )}
            <p className="welcome__fine">With Plex, Platter never sees your password: you approve this device on plex.tv.</p>
          </>
        ) : (
          <>
            {/* Secure PINs are long and already in the sign-in link; only
                short codes are meant to be typed at plex.tv/link. */}
            {pin && pin.code.length > 8 ? (
              <p className="welcome__lead">
                We opened plex.tv in your browser. Sign in there and approve Platter.
              </p>
            ) : (
              <>
                <p className="welcome__lead">We opened plex.tv in your browser. If it asks for a code, enter:</p>
                <div className="welcome__code" aria-label={pin ? `Code ${pin.code.split("").join(" ")}` : "Code loading"}>
                  {pin ? pin.code : <Skeleton width={180} height={48} />}
                </div>
              </>
            )}
            <p className="welcome__waiting" role="status">
              <span className="welcome__pulse" aria-hidden="true" />
              Waiting for you to approve Platter on plex.tv…
            </p>
            <div className="welcome__actions">
              {pin && (
                <Button variant="ghost" onClick={() => navigator.clipboard?.writeText(pin.authUrl)}>
                  Copy sign-in link
                </Button>
              )}
              <Button variant="secondary" onClick={cancel}>
                Cancel
              </Button>
            </div>
          </>
        )}
        {error && (
          <p className="welcome__error" role="alert">
            {error}
          </p>
        )}
      </div>
    </div>
  );
}

/** Shown when signed in but several servers are available. */
export function ServerPickPage() {
  const accessRemoved = useApp((s) => s.health?.banner?.kind === "accessRemoved");
  const [servers, setServers] = useState<ServerInfo[] | null>(null);
  const [err, setErr] = useState<string | null>(null);
  useEffect(() => {
    backend.serversList().then(setServers, (e: AppError) => setErr(e.message));
  }, []);
  return (
    <div className="welcome">
      <WindowBar />
      <div className="welcome__panel" role="main">
        <Logo size={48} />
        <h1 className="welcome__title">Choose your server</h1>
        <p className="welcome__lead">
          {accessRemoved
            ? "You no longer have access to this server. Pick another one."
            : "Pick the Plex server that has your music."}
        </p>
        <ul className="server-list">
          {servers
            ? servers.map((s) => (
                <li key={s.machineId}>
                  <button
                    type="button"
                    className="server-item"
                    disabled={!s.online}
                    onClick={() => backend.serverSelect(s.machineId).catch((e: AppError) => setErr(e.message))}
                  >
                    <Server size={20} aria-hidden="true" />
                    <span className="server-item__name">{s.name}</span>
                    <span className={`server-item__state ${s.online ? "is-online" : ""}`}>{s.online ? "Online" : "Offline"}</span>
                  </button>
                </li>
              ))
            : [0, 1].map((i) => (
                <li key={i}>
                  <Skeleton height={56} radius="card" />
                </li>
              ))}
        </ul>
        {err && (
          <p className="welcome__error" role="alert">
            {err}
          </p>
        )}
      </div>
    </div>
  );
}
