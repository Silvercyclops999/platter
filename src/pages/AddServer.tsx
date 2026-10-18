import { useEffect, useId, useRef, useState } from "react";
import { Badge } from "../components/Badge";
import { Button } from "../components/Button";
import { backend } from "../lib/backend";
import type { AppError, SavedServer, ServerCheck, SubsonicCheck } from "../lib/types";
import "./add-server.css";

/** How often the Quick Connect code is checked. */
export const QUICK_POLL_MS = 2000;

type Step =
  | { kind: "choose" }
  | { kind: "address" }
  | { kind: "code"; code: string; server: ServerCheck }
  | { kind: "password"; server: ServerCheck; note: string | null }
  | { kind: "osAddress" }
  | { kind: "osSignIn"; server: SubsonicCheck; useKey: boolean };

/** The kinds of server added by address. */
export type AddressKind = "jellyfin" | "subsonic";

const message = (e: unknown, fallback: string) => (e as AppError)?.message || fallback;

interface Props {
  /** Start by choosing the kind of server; else at the address for `kind`. */
  choose?: boolean;
  /** Without `choose`: the kind of server (Jellyfin unless said). */
  kind?: AddressKind;
  /** Plex is already signed in (its choice is shown as done). */
  plexSignedIn?: boolean;
  /** Plex was chosen: its own sign-in takes over. */
  onPlex?: () => void;
  /** The server was added (or signed in to again). */
  onDone: (s: SavedServer) => void;
  onCancel?: () => void;
  /** Signing in again: the address to use. */
  url?: string;
  /** Signing in again: the server's tag, which it keeps even at a new address. */
  tag?: string;
  pollMs?: number;
}

/**
 * Add a server: Plex, Jellyfin, or Navidrome or another Subsonic server.
 * Jellyfin: its address, then a Quick Connect code to approve in another
 * Jellyfin app (or, when Quick Connect is off or not wanted, a user name
 * and password used once). Subsonic: its address, then an API key (where
 * the server takes them) or a user name and password.
 */
export function AddServerFlow({ choose = false, kind = "jellyfin", plexSignedIn = false, onPlex, onDone, onCancel, url: initialUrl, tag, pollMs = QUICK_POLL_MS }: Props) {
  const [step, setStep] = useState<Step>(choose ? { kind: "choose" } : kind === "subsonic" ? { kind: "osAddress" } : { kind: "address" });
  const [apiKey, setApiKey] = useState("");
  const [url, setUrl] = useState(initialUrl ?? "");
  const [user, setUser] = useState("");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const id = useId();
  const first = useRef<HTMLInputElement>(null);
  const done = useRef(onDone);
  done.current = onDone;

  const keyStep = step.kind === "osSignIn" && step.useKey;
  useEffect(() => {
    first.current?.focus();
  }, [step.kind, keyStep]);

  // While a code is showing, ask whether it was approved.
  useEffect(() => {
    if (step.kind !== "code") return;
    let alive = true;
    // One question at a time: a slow answer can't overlap the next tick, so
    // an approval adds the server once.
    let asking = false;
    const t = setInterval(() => {
      if (!alive || asking) return;
      asking = true;
      backend.jellyfinQuickPoll().then(
        (r) => {
          asking = false;
          if (alive && r.state === "done") {
            alive = false;
            clearInterval(t);
            done.current(r.server);
          }
        },
        (e) => {
          asking = false;
          if (!alive) return;
          alive = false;
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
    try {
      const server = await backend.jellyfinSignInPassword(step.server.url, user, password);
      // Signed in: the password isn't kept anywhere. A failed try keeps it to fix.
      setPassword("");
      onDone(server);
    } catch (e) {
      setError(message(e, "Platter couldn't sign in."));
    } finally {
      setBusy(false);
    }
  };

  const osCheck = async () => {
    setBusy(true);
    setError(null);
    try {
      const server = await backend.subsonicCheck(url);
      setUrl(server.url);
      setStep({ kind: "osSignIn", server, useKey: server.apiKeys });
    } catch (e) {
      setError(message(e, "Platter couldn't check that address."));
    } finally {
      setBusy(false);
    }
  };

  const osSignIn = async () => {
    if (step.kind !== "osSignIn") return;
    setBusy(true);
    setError(null);
    // The key or password goes to the core, which keeps it in the keychain.
    const creds = step.useKey ? { apiKey } : { user, password };
    try {
      const server = await backend.subsonicSignIn(step.server.url, creds, tag);
      // Only once it worked: a failed try keeps what was typed to fix.
      setApiKey("");
      setPassword("");
      onDone(server);
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
            <span className="add-server__kind-name">
              Jellyfin <Badge tone="accent">Beta</Badge>
            </span>
            <span className="add-server__kind-note">Your server's address</span>
          </button>
          <button type="button" className="add-server__kind" onClick={() => setStep({ kind: "osAddress" })}>
            <span className="add-server__kind-name">
              Navidrome or another Subsonic server <Badge tone="accent">Beta</Badge>
            </span>
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

  if (step.kind === "osAddress") {
    return (
      <form
        className="add-server"
        onSubmit={(e) => {
          e.preventDefault();
          if (url.trim() && !busy) void osCheck();
        }}
      >
        <div>
          <label className="add-server__label" htmlFor={`${id}-os-url`}>
            Server address
          </label>
          <input
            ref={first}
            id={`${id}-os-url`}
            className="add-server__input"
            type="text"
            inputMode="url"
            autoComplete="url"
            spellCheck={false}
            placeholder="http://navidrome.local:4533"
            value={url}
            aria-invalid={error ? true : undefined}
            aria-describedby={error ? `${id}-err` : `${id}-os-hint`}
            onChange={(e) => setUrl(e.target.value)}
          />
        </div>
        <p className="add-server__hint" id={`${id}-os-hint`}>
          The address you open Navidrome or your other Subsonic server at in a browser, with its port.
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

  if (step.kind === "osSignIn") {
    const { server, useKey } = step;
    const ready = useKey ? !!apiKey.trim() : !!user.trim() && !!password;
    return (
      <form
        className="add-server"
        onSubmit={(e) => {
          e.preventDefault();
          if (ready && !busy) void osSignIn();
        }}
      >
        {!server.apiKeys && <p className="add-server__hint">This server doesn't take API keys, so sign in with your user name and password.</p>}
        {useKey ? (
          <div>
            <label className="add-server__label" htmlFor={`${id}-key`}>
              API key from {server.name}
            </label>
            <input
              ref={first}
              id={`${id}-key`}
              className="add-server__input"
              type="password"
              autoComplete="off"
              spellCheck={false}
              value={apiKey}
              aria-invalid={error ? true : undefined}
              aria-describedby={error ? `${id}-err` : `${id}-key-hint`}
              onChange={(e) => setApiKey(e.target.value)}
            />
          </div>
        ) : (
          <>
            <div>
              <label className="add-server__label" htmlFor={`${id}-os-user`}>
                User name on {server.name}
              </label>
              <input
                ref={first}
                id={`${id}-os-user`}
                className="add-server__input"
                type="text"
                autoComplete="username"
                spellCheck={false}
                value={user}
                onChange={(e) => setUser(e.target.value)}
              />
            </div>
            <div>
              <label className="add-server__label" htmlFor={`${id}-os-pw`}>
                Password
              </label>
              <input
                id={`${id}-os-pw`}
                className="add-server__input"
                type="password"
                autoComplete="current-password"
                value={password}
                aria-invalid={error ? true : undefined}
                aria-describedby={error ? `${id}-err` : `${id}-key-hint`}
                onChange={(e) => setPassword(e.target.value)}
              />
            </div>
          </>
        )}
        <p className="add-server__hint" id={`${id}-key-hint`}>
          {useKey
            ? "Make an API key in your account settings on the server. Platter keeps it in this computer's keychain."
            : server.apiKeys
              ? "These servers check your password on every request, so Platter keeps it in this computer's keychain and sends only a scrambled check made from it. An API key is safer: you can take it back on the server without changing your password."
              : "These servers check your password on every request, so Platter keeps it in this computer's keychain and sends only a scrambled check made from it."}
        </p>
        {errorLine}
        <div className="add-server__actions">
          {server.apiKeys && (
            <Button
              variant="ghost"
              type="button"
              onClick={() => {
                setError(null);
                setStep({ kind: "osSignIn", server, useKey: !useKey });
              }}
            >
              {useKey ? "Use a password instead" : "Use an API key instead"}
            </Button>
          )}
          {onCancel && (
            <Button variant="secondary" type="button" onClick={cancel}>
              Cancel
            </Button>
          )}
          <Button variant="primary" type="submit" disabled={!ready || busy}>
            {busy ? "Signing in…" : "Sign in"}
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
