import { Server } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { Button } from "../components/Button";
import { Skeleton } from "../components/Skeleton";
import { Switch } from "../components/Switch";
import { backend } from "../lib/backend";
import type { AppError, LibrarySources, Settings, SourceServer } from "../lib/types";
import { updateSettings, useSettings } from "../stores/settings";
import "./library-sources.css";

const HOW: Record<string, string> = { local: "on your network", remote: "over remote access", relay: "through the Plex relay" };

/** "Yours, on your network" / "Shared with you, through the Plex relay" / "Offline". */
export function serverLine(s: SourceServer): string {
  const whose = s.owned ? "Yours" : "Shared with you";
  if (!s.online) return `${whose}, offline`;
  const how = s.connection ? HOW[s.connection] : null;
  return how ? `${whose}, ${how}` : whose;
}

/** The saved entry for a library: `<machine id>/<section id>`. */
const entryOf = (s: SourceServer, id: string) => `${s.machineId}/${id}`;

/** How many libraries are in, for the row's control. */
export function LibraryCount({ settings }: { settings: Settings | null }) {
  if (!settings) return <Skeleton width={80} height={20} />;
  const n = settings.libraries.length + 1;
  return <span className="library-count">{n === 1 ? "1 library" : `${n} libraries`}</span>;
}

/**
 * Every server and music library the account can reach, owned and shared,
 * with a switch for each library. The connected server's main library is
 * always in. Asks each server, so it loads when Settings opens.
 */
export function LibrarySourceList({ settings }: { settings: Settings | null }) {
  const [sources, setSources] = useState<LibrarySources | null>(null);
  const [error, setError] = useState<string | null>(null);
  const load = useCallback(() => {
    setError(null);
    backend.librarySources().then(setSources, (e: AppError) => setError(e.message || "Couldn't list your servers."));
  }, []);
  useEffect(load, [load]);

  if (error) {
    return (
      <p className="integration-state is-error" role="alert">
        {error}{" "}
        <Button size="sm" variant="ghost" onClick={load}>
          Try again
        </Button>
      </p>
    );
  }
  if (!sources || !settings) {
    return (
      <div className="library-sources" aria-busy="true">
        <Skeleton width="100%" height={56} />
        <Skeleton width="100%" height={56} />
      </div>
    );
  }
  const included = settings.libraries;
  const toggle = (s: SourceServer, id: string, on: boolean) => {
    const e = entryOf(s, id);
    const next = on ? [...included.filter((x) => x !== e), e] : included.filter((x) => x !== e);
    setSources({
      servers: sources.servers.map((x) =>
        x.machineId === s.machineId ? { ...x, libraries: x.libraries.map((l) => (l.id === id ? { ...l, included: on } : l)) } : x,
      ),
    });
    // A refused change shows what's really included again (updateSettings says why).
    void updateSettings({ libraries: next }).then(() => {
      const now = useSettings.getState().settings?.libraries ?? [];
      if (now.length !== next.length || now.some((x, i) => x !== next[i])) load();
    });
  };

  return (
    <ul className="library-sources" aria-label="Servers and libraries">
      {sources.servers.map((s) => (
        <li key={s.machineId} className="library-source" data-server={s.tag}>
          <div className="library-source__head">
            <Server size={20} aria-hidden="true" />
            <span className="library-source__name truncate">{s.name}</span>
            {s.current && <span className="library-source__badge">Connected</span>}
            <span className={`library-source__state ${s.online ? "is-online" : ""}`}>{serverLine(s)}</span>
          </div>
          {s.libraries.length === 0 ? (
            <p className="library-source__empty">{s.online ? "No music libraries." : "Its libraries show when it's back online."}</p>
          ) : (
            <ul className="library-source__libs">
              {s.libraries.map((l) => (
                <li key={l.id} className="library-lib">
                  <span className="library-lib__title truncate">{l.title}</span>
                  {l.main && <span className="library-lib__note">Main library</span>}
                  <Switch
                    checked={l.included}
                    disabled={l.main}
                    label={`Include ${l.title} on ${s.name}`}
                    onChange={(on) => toggle(s, l.id, on)}
                  />
                </li>
              ))}
            </ul>
          )}
        </li>
      ))}
    </ul>
  );
}
