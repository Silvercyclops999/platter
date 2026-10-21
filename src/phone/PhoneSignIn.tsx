/**
 * Signing in on the phone: one column, the name at the top, and the
 * actions low on the screen where a thumb reaches. The same core calls as
 * the computer's welcome screen (Plex PIN, picking a server, adding a
 * Jellyfin or Subsonic server); only the arrangement is the phone's.
 */
import { Check, ChevronLeft, Copy, Server } from "lucide-react";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { backend } from "../lib/backend";
import type { AppError, PinInfo, ServerInfo } from "../lib/types";
import { AddServerFlow, type AddressKind } from "../pages/AddServer";
import { useApp } from "../stores/app";
import { Wordmark } from "../shell/Logo";
import { useUi } from "../stores/ui";
import { tap } from "./bits";
import { notchFromUrl } from "./form";
import { installTapGuard } from "./gestures";
import "./phone.css";

const NOTCH = typeof window !== "undefined" && notchFromUrl(window.location.search);

/** The phone frame these screens draw in, with the safe areas. */
function Frame({ children, step }: { children: ReactNode; step: string }) {
  const root = useRef<HTMLDivElement>(null);
  useEffect(() => (root.current ? installTapGuard(root.current) : undefined), []);
  return (
    <div ref={root} className={`phone ph-signin ${NOTCH ? "ph-notch" : ""}`} data-testid="phone-signin" data-step={step}>
      <main className="ph-signin__scroll">{children}</main>
    </div>
  );
}

/** The mark and name at the top of the sign-in screens. */
function Brand() {
  return (
    <p className="ph-wordmark" aria-hidden="true">
      <Wordmark size={27} />
    </p>
  );
}

/** What the sign-in screen is showing. */
export type SignInStep = "welcome" | "waiting" | AddressKind;

export function PhoneSignIn({ onBack }: { onBack?: () => void } = {}) {
  const auth = useApp((s) => s.auth);
  const [other, setOther] = useState<AddressKind | null>(null);
  const [pin, setPin] = useState<PinInfo | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [copied, setCopied] = useState(false);
  // Jellyfin and Navidrome sit behind one quiet button, under Plex.
  const [others, setOthers] = useState(false);
  const pending = auth?.state === "pending" || !!pin;
  const step: SignInStep = other ?? (pending ? "waiting" : "welcome");

  useEffect(() => {
    if (auth?.state === "signedOut") setPin(null);
  }, [auth?.state]);

  const start = async () => {
    tap();
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
  const copy = () => {
    if (!pin) return;
    void navigator.clipboard?.writeText(pin.authUrl).then(
      () => setCopied(true),
      () => {},
    );
  };

  if (other)
    return (
      <Frame step={step}>
        <div className="ph-signin__bar">
          <button type="button" className="ph-icon-btn" aria-label="Back" onClick={() => setOther(null)}>
            <ChevronLeft size={26} aria-hidden="true" />
          </button>
        </div>
        <h1 className="ph-title">{other === "jellyfin" ? "Add a Jellyfin server" : "Add a Navidrome or Subsonic server"}</h1>
        <p className="ph-signin__lead">
          {other === "jellyfin"
            ? "Type your server's address. You'll approve Platter with a code, or sign in with your password."
            : "Type your server's address. You'll sign in with an API key, or your user name and password."}
        </p>
        <div className="ph-signin__form">
          <AddServerFlow
            key={other}
            kind={other}
            onDone={() => {
              useUi.getState().setSignInOpen(false);
              void backend.serversSaved().then((l) => useApp.getState().setServers(l), () => {});
            }}
            onCancel={() => setOther(null)}
          />
        </div>
      </Frame>
    );

  const title = auth?.reauthRequired ? "Sign in again" : onBack ? "Add your Plex music" : "Your records, in your pocket";
  return (
    <Frame step={step}>
      {onBack && !pending && (
        <div className="ph-signin__bar">
          <button type="button" className="ph-text-btn" onClick={onBack}>
            Not now
          </button>
        </div>
      )}
      <Brand />
      {!pending ? (
        <>
          <div className="ph-signin__words">
            <h1 className="ph-title ph-signin__title">{title}</h1>
            <p className="ph-signin__lead">
              {auth?.reauthRequired
                ? "Plex asked you to sign in again. Music saved on this phone keeps playing in the meantime."
                : onBack
                  ? "Add your Plex music next to the servers you already have."
                  : "Sign in with Plex and your music is here. Nothing to install on your server."}
            </p>
          </div>
          <div className="ph-signin__actions">
            <button type="button" className="ph-big-btn is-primary" onClick={() => void start()} disabled={busy}>
              Sign in with Plex
            </button>
            {!onBack && !others && (
              <button type="button" className="ph-big-btn is-quiet" onClick={() => setOthers(true)}>
                Use another server
              </button>
            )}
            {!onBack && others && (
              <>
                <button type="button" className="ph-big-btn" onClick={() => setOther("jellyfin")}>
                  Use a Jellyfin server
                </button>
                <button type="button" className="ph-big-btn" onClick={() => setOther("subsonic")}>
                  Use Navidrome or Subsonic
                </button>
              </>
            )}
            <p className="ph-signin__fine">With Plex, Platter never sees your password: you approve this phone on plex.tv.</p>
          </div>
        </>
      ) : (
        <>
          <div className="ph-signin__words">
            <h1 className="ph-title ph-signin__title">Approve Platter on plex.tv</h1>
            <p className="ph-signin__lead">Platter opened plex.tv in your browser. Sign in there, approve Platter, then come back here.</p>
            <p className="ph-signin__waiting" role="status">
              <span className="ph-signin__pulse" aria-hidden="true" />
              Waiting for you to approve
            </p>
            {/* A long secure PIN is already in the link; a short code is only typed if plex.tv asks. */}
            {(!pin || pin.code.length <= 8) && (
              <p className="ph-signin__code-line">
                If plex.tv asks for a code, it's{" "}
                <span className="ph-code tabular" aria-label={pin ? `Code ${pin.code.split("").join(" ")}` : "Code loading"}>
                  {pin ? pin.code : " "}
                </span>
              </p>
            )}
          </div>
          <div className="ph-signin__actions">
            {pin && (
              <button type="button" className="ph-big-btn" onClick={copy}>
                {copied ? <Check size={18} aria-hidden="true" /> : <Copy size={18} aria-hidden="true" />}
                {copied ? "Link copied" : "Copy the sign-in link"}
              </button>
            )}
            <button type="button" className="ph-big-btn is-quiet" onClick={() => void cancel()}>
              Cancel
            </button>
          </div>
        </>
      )}
      {error && (
        <p className="ph-signin__error" role="alert">
          {error}
        </p>
      )}
    </Frame>
  );
}

/** Signed in, with several servers to choose from. */
export function PhoneServerPick() {
  const accessRemoved = useApp((s) => s.health?.banner?.kind === "accessRemoved");
  const [servers, setServers] = useState<ServerInfo[] | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [picking, setPicking] = useState<string | null>(null);
  useEffect(() => {
    backend.serversList().then(setServers, (e: AppError) => setErr(e.message));
  }, []);
  const pick = (s: ServerInfo) => {
    tap();
    setErr(null);
    setPicking(s.machineId);
    backend.serverSelect(s.machineId).then(
      () => {},
      (e: AppError) => {
        setPicking(null);
        setErr(e.message);
      },
    );
  };
  return (
    <Frame step="server">
      <Brand />
      <div className="ph-signin__words">
        <p className="ph-eyebrow">Signed in</p>
        <h1 className="ph-title ph-signin__title">Choose your server</h1>
        <p className="ph-signin__lead">{accessRemoved ? "You no longer have access to that server. Pick another one." : "Pick the Plex server that has your music."}</p>
      </div>
      <ul className="ph-servers" aria-label="Servers" aria-busy={!servers || undefined}>
        {servers
          ? servers.map((s) => (
              <li key={s.machineId}>
                <button type="button" className={`ph-server ${picking === s.machineId ? "is-busy" : ""}`} disabled={!s.online || !!picking} onClick={() => pick(s)}>
                  <span className="ph-server__icon">
                    <Server size={22} aria-hidden="true" />
                  </span>
                  <span className="ph-row__text">
                    <span className="ph-row__title">{s.name}</span>
                    <span className="ph-row__sub">{picking === s.machineId ? "Connecting" : s.owned ? "Your server" : "Shared with you"}</span>
                  </span>
                  <span className={`ph-server__state ${s.online ? "is-on" : ""}`}>
                    <span className="ph-server__lamp" aria-hidden="true" />
                    {s.online ? "Online" : "Offline"}
                  </span>
                </button>
              </li>
            ))
          : [0, 1].map((i) => <li key={i} className="ph-server ph-server--ghost" />)}
      </ul>
      {servers && servers.length === 0 && <p className="ph-signin__lead">No servers on this Plex account yet.</p>}
      {err && (
        <p className="ph-signin__error" role="alert">
          {err}
        </p>
      )}
    </Frame>
  );
}
