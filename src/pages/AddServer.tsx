import { useEffect, useId, useRef, useState } from "react";
import { Button } from "../components/Button";
import { backend } from "../lib/backend";
import type { AppError, SavedServer, ServerCheck } from "../lib/types";
import "./add-server.css";

/** How often the Quick Connect code is checked. */
export const QUICK_POLL_MS = 2000;

type Step =
  | { kind: "choose" }
  | { kind: "address" }
  | { kind: "code"; code: string; server: ServerCheck }
  | { kind: "password"; server: ServerCheck; note: string | null };

const message = (e: unknown, fallback: string) => (e as AppError)?.message || fallback;

interface Props {
  /** Start by choosing the kind of server; else at the Jellyfin address. */
  choose?: boolean;
  /** Plex is already signed in (its choice is shown as done). */
  plexSignedIn?: boolean;
  /** Plex was chosen: its own sign-in takes over. */
  onPlex?: () => void;
  /** The server was added (or signed in to again). */
  onDone: (s: SavedServer) => void;
  onCancel?: () => void;
  /** Signing in again: the address to use. */
  url?: string;
  pollMs?: number;
}

/**
 * Add a server: Plex or Jellyfin, then for Jellyfin its address, then a
 * Quick Connect code to approve in another Jellyfin app (or, when Quick
 * Connect is off or not wanted, a user name and password used once).
 */
export function AddServerFlow({ choose = false, plexSignedIn = false, onPlex, onDone, onCancel, url: initialUrl, pollMs = QUICK_POLL_MS }: Props) {
  const [step, setStep] = useState<Step>(choose ? { kind: "choose" } : { kind: "address" });
  const [url, setUrl] = useState(initialUrl ?? "");
  const [user, setUser] = useState("");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const id = useId();
  const first = useRef<HTMLInputElement>(null);
  const done = useRef(onDone);
  done.current = onDone;

  useEffect(() => {
    first.current?.focus();
  }, [step.kind]);

  // While a code is showing, ask whether it was approved.
  useEffect(() => {
    if (step.kind !== "code") return;
    let alive = true;
    const t = setInterval(() => {
      backend.jellyfinQuickPoll().then(
        (r) => {
          if (alive && r.state === "done") {
            clearInterval(t);
            done.current(r.server);
          }
        },
        (e) => {
          if (!alive) return;
          clearInterval(t);
          setError(message(e, "Quick Connect stopped."));
          setStep({ kind: "address" });
        },
      );
    }, pollMs);
    return () => {
      alive = false;
      clearInterval(t);
    };
  }, [step, pollMs]);

  const cancel = () => {
    if (step.kind === "code") void backend.jellyfinQuickCancel().catch(() => {});
    onCancel?.();
  };

  const check = async () => {
    setBusy(true);
    setError(null);
    try {
      const server = await backend.jellyfinCheck(url);
      setUrl(server.url);
      if (!server.quickConnect) {
        setStep({ kind: "password", server, note: "Quick Connect is turned off on this server, so sign in with your user name and password." });
        return;
      }
      const code = await backend.jellyfinQuickStart(server.url);
      setStep({ kind: "code", code: code.code, server });
    } catch (e) {
      setError(message(e, "Platter couldn't check that address."));
    } finally {
      setBusy(false);
    }
  };

  const signIn = async () => {
    if (step.kind !== "password") return;
    setBusy(true);
    setError(null);
    // The password leaves this form now; it isn't kept anywhere.
    const pw = password;
    setPassword("");
    try {
      onDone(await backend.jellyfinSignInPassword(step.server.url, user, pw));
    } catch (e) {
      setError(message(e, "Platter couldn't sign in."));
    } finally {
      setBusy(false);
    }
  };

  const errorLine = error && (
    <p className="add-server__error" id={`${id}-err`} role="alert">
      {error}
    </p>
  );

  if (step.kind === "choose") {
    return (
      <div className="add-server">
        <p className="add-server__hint">Which kind of server has your music?</p>
        <div className="add-server__kinds">
          <button type="button" className="add-server__kind" disabled={plexSignedIn} onClick={() => onPlex?.()}>
            <span className="add-server__kind-name">Plex</span>
            <span className="add-server__kind-note">{plexSignedIn ? "Signed in" : "Sign in with your Plex account"}</span>
          </button>
          <button type="button" className="add-server__kind" onClick={() => setStep({ kind: "address" })}>
            <span className="add-server__kind-name">Jellyfin</span>
            <span className="add-server__kind-note">Your server's address</span>
          </button>
        </div>
        {onCancel && (
          <div className="add-server__actions">
            <Button variant="ghost" onClick={cancel}>
              Cancel
            </Button>
          </div>
        )}
      </div>
    );
  }

  if (step.kind === "address") {
    return (
      <form
        className="add-server"
        onSubmit={(e) => {
          e.preventDefault();
          if (url.trim() && !busy) void check();
        }}
      >
        <div>
          <label className="add-server__label" htmlFor={`${id}-url`}>
            Jellyfin server address
          </label>
          <input
            ref={first}
            id={`${id}-url`}
            className="add-server__input"
            type="text"
            inputMode="url"
            autoComplete="url"
            spellCheck={false}
            placeholder="http://jellyfin.local:8096"
            value={url}
            aria-invalid={error ? true : undefined}
            aria-describedby={error ? `${id}-err` : `${id}-hint`}
            onChange={(e) => setUrl(e.target.value)}
          />
        </div>
        <p className="add-server__hint" id={`${id}-hint`}>
          The address you open Jellyfin at in a browser, with its port.
        </p>
        {errorLine}
        <div className="add-server__actions">
          {onCancel && (
            <Button variant="ghost" type="button" onClick={cancel}>
              Cancel
            </Button>
          )}
          <Button variant="primary" type="submit" disabled={!url.trim() || busy}>
            {busy ? "Checking…" : "Continue"}
          </Button>
        </div>
      </form>
    );
  }

  if (step.kind === "code") {
    return (
      <div className="add-server">
        <p className="add-server__hint">
          In a Jellyfin app where you're signed in to {step.server.name}, open your profile, choose Quick Connect and enter this code:
        </p>
        <div className="welcome__code add-server__code" aria-label={`Code ${step.code.split("").join(" ")}`}>
          {step.code}
        </div>
        <p className="welcome__waiting" role="status">
          <span className="welcome__pulse" aria-hidden="true" />
          Waiting for you to approve Platter…
        </p>
        {errorLine}
        <div className="add-server__actions">
          <Button
            variant="ghost"
            onClick={() => {
              void backend.jellyfinQuickCancel().catch(() => {});
              setStep({ kind: "password", server: step.server, note: null });
            }}
          >
            Use a password instead
          </Button>
          {onCancel && (
            <Button variant="secondary" onClick={cancel}>
              Cancel
            </Button>
          )}
        </div>
      </div>
    );
  }

  return (
    <form
      className="add-server"
      onSubmit={(e) => {
        e.preventDefault();
        if (user.trim() && password && !busy) void signIn();
      }}
    >
      {step.note && <p className="add-server__hint">{step.note}</p>}
      <div>
        <label className="add-server__label" htmlFor={`${id}-user`}>
          User name on {step.server.name}
        </label>
        <input
          ref={first}
          id={`${id}-user`}
          className="add-server__input"
          type="text"
          autoComplete="username"
          spellCheck={false}
          value={user}
          onChange={(e) => setUser(e.target.value)}
        />
      </div>
      <div>
        <label className="add-server__label" htmlFor={`${id}-pw`}>
          Password
        </label>
        <input
          id={`${id}-pw`}
          className="add-server__input"
          type="password"
          autoComplete="current-password"
          value={password}
          aria-invalid={error ? true : undefined}
          aria-describedby={error ? `${id}-err` : `${id}-pw-hint`}
          onChange={(e) => setPassword(e.target.value)}
        />
      </div>
      <p className="add-server__hint" id={`${id}-pw-hint`}>
        Platter uses your password once to sign in and never keeps it.
      </p>
      {errorLine}
      <div className="add-server__actions">
        {onCancel && (
          <Button variant="ghost" type="button" onClick={cancel}>
            Cancel
          </Button>
        )}
        <Button variant="primary" type="submit" disabled={!user.trim() || !password || busy}>
          {busy ? "Signing in…" : "Sign in"}
        </Button>
      </div>
    </form>
  );
}
