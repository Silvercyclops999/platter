/**
 * Connect your account: the import source that reads the streaming
 * service's playlists through its Web API with the user's own developer
 * app. The card on the import page and the setup block in Settings share
 * the state and the buttons here. Every network step happens in the core;
 * the sign-in opens in the system browser.
 */
import { useCallback, useEffect, useRef, useState } from "react";
import { Button } from "../components/Button";
import { Skeleton } from "../components/Skeleton";
import { Switch } from "../components/Switch";
import { backend } from "../lib/backend";
import { useDomId } from "../lib/domId";
import type { AccountStatus } from "../lib/importTypes";
import { ACCOUNT_TEXT } from "../lib/importStrings";
import { nav } from "../stores/router";
import { copyLink } from "../stores/share";
import { updateSettings, useSettings } from "../stores/settings";
import { toast } from "../stores/toasts";
import { failText } from "../lib/errorText";
import { importChanged } from "./importShared";

export const ACCOUNT_SOURCE = "streaming-account";

function day(ms: number): string {
  return new Date(ms).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
    year: "numeric",
  });
}

function clock(ms: number): string {
  return new Date(ms).toLocaleTimeString(undefined, {
    hour: "numeric",
    minute: "2-digit",
  });
}

export interface AccountControls {
  status: AccountStatus | null;
  signIn: () => Promise<void>;
  read: () => Promise<void>;
  cancel: () => Promise<void>;
  signOut: () => Promise<void>;
  /** True when it was kept (or removed). */
  setClientId: (id: string | null) => Promise<boolean>;
}

/** The connection's state, loaded once and kept current by the core's events. */
export function useAccount(): AccountControls {
  const [status, setStatus] = useState<AccountStatus | null>(null);
  const sources = useSettings((s) => s.settings?.importSources.join());
  const last = useRef<AccountStatus | null>(null);
  const take = useCallback((s: AccountStatus) => {
    // A read that just finished changed the import: open screens load again.
    const was = last.current;
    if (was && ((was.state === "reading" && s.state !== "reading") || was.lastRead !== s.lastRead)) importChanged();
    last.current = s;
    setStatus(s);
  }, []);
  useEffect(() => {
    let alive = true;
    backend.streamingAccountStatus().then(
      (s) => alive && take(s),
      () => {},
    );
    return () => {
      alive = false;
    };
  }, [sources, take]);
  useEffect(() => {
    let alive = true;
    const un = backend.listen(
      "streaming-account://status",
      (s) => alive && take(s),
    );
    return () => {
      alive = false;
      void un.then((f) => f());
    };
  }, [take]);
  const run = (f: () => Promise<AccountStatus>, fail: string) => async () => {
    try {
      take(await f());
    } catch (e) {
      toast.error(failText(fail, e));
    }
  };
  return {
    status,
    signIn: run(
      () => backend.streamingAccountSignIn(),
      "Couldn't start signing in",
    ),
    read: run(
      () => backend.streamingAccountRead(),
      "Couldn't read your playlists",
    ),
    cancel: run(() => backend.streamingAccountCancel(), "Couldn't stop"),
    signOut: run(() => backend.streamingAccountSignOut(), "Couldn't sign out"),
    setClientId: async (id) => {
      try {
        take(await backend.streamingAccountClientIdSet(id));
        toast.info(id ? "Client ID saved" : "Client ID removed");
        return true;
      } catch (e) {
        toast.error(failText("Couldn't save the Client ID", e));
        return false;
      }
    },
  };
}

/** Where it stands, in a sentence or two. */
export function accountLine(s: AccountStatus): string {
  switch (s.state) {
    case "notSetUp":
      return "Add your developer app's Client ID in Settings to connect.";
    case "signedOut":
      return "Not connected.";
    case "signingIn":
      return "Finish signing in in your browser. Platter waits here.";
    case "reading":
      return s.total
        ? `Reading your playlists: ${s.done.toLocaleString()} of ${s.total.toLocaleString()}.`
        : "Reading your playlists.";
    case "connected": {
      const who = s.name ? `Connected as ${s.name}.` : "Connected.";
      const when = s.lastRead
        ? ` Playlists read ${day(s.lastRead)}.`
        : " Your playlists haven't been read yet.";
      return `${who}${when}${s.message ? ` ${s.message}` : ""}`;
    }
    case "refused":
      return s.message ?? "The service refused the sign-in. Try again.";
    case "expired":
      return "Your sign-in ran out (the service ends them after a while). Sign in again to keep your playlists up to date.";
    case "rateLimited":
      return s.retryAt
        ? `The service asked Platter to slow down. Try again after ${clock(s.retryAt)}.`
        : "The service asked Platter to slow down. Try again in a few minutes.";
    case "offline":
      return s.message ?? "You're offline.";
  }
}

/** The buttons for each state. */
export function AccountButtons({
  a,
  inSettings,
}: {
  a: AccountControls;
  inSettings?: boolean;
}) {
  const s = a.status;
  if (!s) return <Skeleton width={160} height={32} radius="pill" />;
  const signOut = (
    <Button size="sm" variant="ghost" onClick={() => void a.signOut()}>
      Sign out
    </Button>
  );
  switch (s.state) {
    case "notSetUp":
      return inSettings ? null : (
        <Button
          size="sm"
          onClick={() => nav.go({ name: "settings", page: "libraryImport" })}
        >
          Set up in Settings
        </Button>
      );
    case "signedOut":
      return (
        <Button size="sm" variant="primary" onClick={() => void a.signIn()}>
          Connect your account
        </Button>
      );
    case "signingIn":
    case "reading":
      return (
        <Button size="sm" onClick={() => void a.cancel()}>
          Stop
        </Button>
      );
    case "connected":
      return (
        <>
          <Button size="sm" onClick={() => void a.read()}>
            Read playlists now
          </Button>
          {signOut}
        </>
      );
    case "refused":
      return (
        <Button size="sm" onClick={() => void a.signIn()}>
          Try again
        </Button>
      );
    case "expired":
      return (
        <Button size="sm" variant="primary" onClick={() => void a.signIn()}>
          Sign in again
        </Button>
      );
    case "rateLimited":
      return (
        <>
          <Button size="sm" disabled>
            Read playlists now
          </Button>
          {signOut}
        </>
      );
    case "offline":
      return (
        <>
          <Button size="sm" onClick={() => void a.read()}>
            Try again
          </Button>
          {signOut}
        </>
      );
  }
}

/** The card on the import page, beside the reports. */
export function AccountCard() {
  const a = useAccount();
  const s = a.status;
  const alert =
    s && ["refused", "expired", "rateLimited", "offline"].includes(s.state);
  return (
    <article
      className="import-account"
      aria-labelledby="import-account-h"
      data-testid="account-card"
    >
      <div className="import-account__text">
        <h2 id="import-account-h" className="import-account__title">
          {s && s.name && ["connected", "reading", "rateLimited", "offline"].includes(s.state) ? `Your ${ACCOUNT_TEXT.service} account` : `Connect your ${ACCOUNT_TEXT.service} account`}
        </h2>
        <p className="import-account__brings">{ACCOUNT_TEXT.brings}</p>
        <p className="import-account__cant">{ACCOUNT_TEXT.cant}</p>
        {s ? (
          <p
            className={`import-account__state${alert ? " import-account__state--alert" : ""}`}
            role={alert ? "alert" : "status"}
            data-state={s.state}
          >
            {accountLine(s)}
          </p>
        ) : (
          <Skeleton width="60%" height={16} />
        )}
      </div>
      <div className="import-account__actions">
        <AccountButtons a={a} />
      </div>
    </article>
  );
}

/**
 * The Client ID: a field until one is kept, then "Remove". It goes to the
 * keychain on this device and never comes back to the UI.
 */
function ClientIdControl({ a, labelledBy, describedBy }: { a: AccountControls; labelledBy: string; describedBy: string }) {
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  if (!a.status) return <Skeleton width={260} height={32} />;
  if (a.status.state !== "notSetUp") {
    return (
      <span className="import-account__saved">
        <span className="import-account__saved-note">Saved on this computer</span>
        <Button size="sm" onClick={() => void a.setClientId(null)}>
          Remove
        </Button>
      </span>
    );
  }
  return (
    <form
      className="info-key"
      onSubmit={(e) => {
        e.preventDefault();
        if (!text.trim()) return;
        setBusy(true);
        void a.setClientId(text.trim()).then((ok) => {
          setBusy(false);
          if (ok) setText("");
        });
      }}
    >
      <input
        type="text"
        className="info-key__field import-account__client"
        value={text}
        maxLength={64}
        placeholder="Client ID"
        spellCheck={false}
        autoComplete="off"
        aria-labelledby={labelledBy}
        aria-describedby={describedBy}
        onChange={(e) => setText(e.target.value)}
      />
      <Button size="sm" type="submit" disabled={busy || !text.trim()}>
        Save
      </Button>
    </form>
  );
}

/** Settings > Library import, under the account source once it's on. */
export function AccountSetup() {
  const id = useDomId();
  const a = useAccount();
  const recheck = useSettings((s) => s.settings?.streamingRecheck);
  const uri = a.status?.redirectUri ?? "http://127.0.0.1/callback";
  return (
    <div className="import-account-setup" data-testid="account-setup">
      <div className="setting-row">
        <div className="setting-row__text">
          <div id={`${id}-cid`} className="setting-row__title">
            Client ID
          </div>
          <p id={`${id}-cid-d`} className="setting-row__desc">
            From your app's page on the developer dashboard. Kept in this
            computer's keychain.
          </p>
        </div>
        <div className="setting-row__control">
          <ClientIdControl a={a} labelledBy={`${id}-cid`} describedBy={`${id}-cid-d`} />
        </div>
      </div>
      <div className="setting-row">
        <div className="setting-row__text">
          <div id={`${id}-uri`} className="setting-row__title">
            Redirect URI
          </div>
          <p id={`${id}-uri-d`} className="setting-row__desc">
            Add this exactly in your app's settings. Each sign-in uses a free
            port on 127.0.0.1.
          </p>
        </div>
        <div className="setting-row__control import-account__uri">
          <code aria-labelledby={`${id}-uri`}>{uri}</code>
          <Button
            size="sm"
            variant="ghost"
            onClick={() => void copyLink(uri, "Redirect URI copied")}
          >
            Copy
          </Button>
        </div>
      </div>
      <div className="setting-row" data-setting="streamingRecheck">
        <div className="setting-row__text">
          <div id={`${id}-re`} className="setting-row__title">
            Re-check daily
          </div>
          <p id={`${id}-re-d`} className="setting-row__desc">
            While Platter runs, read your playlists once a day. Nothing you
            imported is removed. Synced.
          </p>
        </div>
        <div className="setting-row__control">
          {recheck === undefined ? (
            <Skeleton width={40} height={24} radius="pill" />
          ) : (
            <Switch
              checked={recheck}
              onChange={(v) => void updateSettings({ streamingRecheck: v })}
              labelledBy={`${id}-re`}
              describedBy={`${id}-re-d`}
            />
          )}
        </div>
      </div>
      <div className="import-source__actions">
        <AccountButtons a={a} inSettings />
        {a.status && a.status.state !== "notSetUp" && (
          <span
            className="import-source__busy"
            role="status"
            data-state={a.status.state}
          >
            {accountLine(a.status)}
          </span>
        )}
      </div>
    </div>
  );
}
