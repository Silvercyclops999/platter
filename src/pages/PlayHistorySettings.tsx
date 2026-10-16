/**
 * Settings for play history from before Platter and other apps: your Plex
 * server's history, Tautulli's and your Last.fm scrobbles. Each source has
 * an Include switch with how its import stands, a Remove button, and
 * Tautulli's address and key (the key goes to the keychain and never comes
 * back to the UI).
 */
import { useCallback, useEffect, useState } from "react";
import { Button } from "../components/Button";
import { Skeleton } from "../components/Skeleton";
import { backend } from "../lib/backend";
import { plural } from "../lib/format";
import type { PlayHistorySource, PlayHistorySourceStatus, PlayHistoryStatus, TautulliStatus } from "../lib/types";
import { updateSettings, useSettings } from "../stores/settings";
import { toast } from "../stores/toasts";
import { errText } from "./catalogue";

type Ids = { title: string; desc: string };

/** Each source's name in a sentence. */
export const SOURCE_NAMES: Record<PlayHistorySource, string> = {
  plexHistory: "Plex",
  tautulli: "Tautulli",
  lastfm: "Last.fm",
};

/** The Include setting for each source. */
export const INCLUDE_KEYS = {
  plexHistory: "includePlexHistory",
  tautulli: "includeTautulliHistory",
  lastfm: "includeLastfmHistory",
} as const satisfies Record<PlayHistorySource, string>;

export interface PlayHistoryState {
  status: PlayHistoryStatus | null;
  tautulli: TautulliStatus | null;
  source: (s: PlayHistorySource) => PlayHistorySourceStatus | undefined;
  remove: (s: PlayHistorySource) => Promise<void>;
  update: () => Promise<void>;
  cancel: () => Promise<void>;
  setKey: (key: string | null) => Promise<boolean>;
}

/** The import's state for the Settings rows: loaded when the page opens, then kept current. */
export function usePlayHistory(): PlayHistoryState {
  const [status, setStatus] = useState<PlayHistoryStatus | null>(null);
  const [tautulli, setTautulli] = useState<TautulliStatus | null>(null);
  // Turning a source on or off, or saving Tautulli's address, changes what's set up.
  const settings = useSettings((s) => s.settings);
  const watch = settings ? [settings.includePlexHistory, settings.includeTautulliHistory, settings.includeLastfmHistory, settings.tautulliUrl].join() : "";
  useEffect(() => {
    let alive = true;
    backend.playHistoryStatus().then(
      (s) => alive && setStatus(s),
      () => {},
    );
    backend.tautulliStatus().then(
      (s) => alive && setTautulli(s),
      () => {},
    );
    return () => {
      alive = false;
    };
  }, [watch]);
  useEffect(() => {
    let alive = true;
    const un = backend.listen("playhistory://status", (s) => alive && setStatus(s));
    return () => {
      alive = false;
      void un.then((f) => f());
    };
  }, []);
  const source = useCallback((s: PlayHistorySource) => status?.sources.find((x) => x.source === s), [status]);
  const remove = async (s: PlayHistorySource) => {
    try {
      setStatus(await backend.playHistoryRemove(s));
      // Off, so nothing comes back until it's turned on again.
      await updateSettings({ [INCLUDE_KEYS[s]]: false });
      toast.info(`Removed your plays from ${SOURCE_NAMES[s]}`);
    } catch (e) {
      toast.error(errText(e) || "Couldn't remove those plays");
    }
  };
  const update = async () => {
    try {
      setStatus(await backend.playHistoryUpdate());
    } catch (e) {
      toast.error(errText(e) || "Couldn't look for new plays");
    }
  };
  const cancel = async () => {
    try {
      setStatus(await backend.playHistoryCancel());
    } catch {
      /* the next status event says where it stands */
    }
  };
  const setKey = async (key: string | null) => {
    try {
      const st = await backend.tautulliKeySet(key);
      setTautulli(st);
      setStatus(await backend.playHistoryStatus());
      if (key) toast.info("Tautulli API key saved");
      return true;
    } catch (e) {
      toast.error(errText(e) || "Couldn't save the key");
      return false;
    }
  };
  return { status, tautulli, source, remove, update, cancel, setKey };
}

function day(ms: number): string {
  return new Date(ms).toLocaleDateString(undefined, { month: "short", day: "numeric", year: "numeric" });
}

/** How a source's import stands, in a sentence or two. */
export function historyNote(st: PlayHistorySourceStatus | undefined): string | undefined {
  if (!st) return undefined;
  const name = SOURCE_NAMES[st.source];
  if (!st.setUp) {
    return st.source === "plexHistory" ? "Available when you're signed in to a Plex server." : undefined;
  }
  if (st.importing) {
    const read = st.total ? `${st.done.toLocaleString()} of ${plural(st.total, "play")}` : plural(st.done, "play");
    return st.firstReadDone ? `Looking for new plays: ${read} read.` : `Bringing in your history from ${name}: ${read} read so far.`;
  }
  if (st.removed) return `Plays from ${name} removed. Turn this on to bring them in again.`;
  if (st.paused) return `Stopped${st.plays > 0 ? ` after ${plural(st.plays, "play")}` : ""}. Go on brings in the rest.`;
  const parts: string[] = [];
  if (st.plays > 0) {
    let line = `${plural(st.plays, "play")} brought in`;
    if (st.notInLibrary > 0) line += `, ${st.notInLibrary.toLocaleString()} of them songs not in your library (they still count for artists)`;
    if (st.shortPlays > 0) line += `. ${plural(st.shortPlays, "play")} too short to count left out`;
    parts.push(`${line}.`);
    if (!st.included) parts.push("Not counted while this is off.");
  }
  if (st.error) parts.push(st.error);
  else if (st.updatedAt) parts.push(`Updated ${day(st.updatedAt)}.`);
  else if (st.included && st.plays === 0) parts.push("Brought in automatically, in the background.");
  return parts.join(" ") || undefined;
}

/** Remove a source's plays (asks first), Stop while it's reading, Update when it failed. */
export function HistorySourceControl({
  history,
  source,
  onRemove,
}: {
  history: PlayHistoryState;
  source: PlayHistorySource;
  onRemove: (s: PlayHistorySource) => void;
}) {
  const st = history.source(source);
  if (!history.status) return <Skeleton width={160} height={32} radius="pill" />;
  if (!st) return null;
  return (
    <span className="shelf-buttons">
      {st.importing && (
        <Button size="sm" onClick={() => void history.cancel()}>
          Stop
        </Button>
      )}
      {!st.importing && st.paused && st.included && (
        <Button size="sm" onClick={() => void history.update()}>
          Go on
        </Button>
      )}
      {!st.importing && !st.paused && st.error && st.included && (
        <Button size="sm" onClick={() => void history.update()}>
          Try again
        </Button>
      )}
      <Button size="sm" variant="secondary" disabled={st.plays === 0} onClick={() => onRemove(source)}>
        Remove
      </Button>
    </span>
  );
}

/** Tautulli's address: saved on blur or Enter, like Lidarr's. */
export function TautulliUrlField({ ids }: { ids: Ids }) {
  const saved = useSettings((s) => s.settings?.tautulliUrl);
  const [value, setValue] = useState(saved ?? "");
  useEffect(() => {
    if (saved !== undefined) setValue(saved);
  }, [saved]);
  if (saved === undefined) return <Skeleton width={220} height={32} />;
  const save = () => {
    const next = value.trim().replace(/\/+$/, "");
    setValue(next);
    if (next !== saved) void updateSettings({ tautulliUrl: next });
  };
  return (
    <input
      type="url"
      className="setting-text"
      value={value}
      maxLength={300}
      placeholder="http://192.168.x.x:8181"
      spellCheck={false}
      autoComplete="off"
      aria-labelledby={ids.title}
      aria-describedby={ids.desc}
      onChange={(e) => setValue(e.target.value)}
      onBlur={save}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          save();
        }
      }}
    />
  );
}

/** Tautulli's API key: a field until one is saved, then "Remove key". Never shown again. */
export function TautulliKeyControl({ history, ids }: { history: PlayHistoryState; ids: Ids }) {
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  if (!history.tautulli) return <Skeleton width={160} height={32} radius="pill" />;
  if (history.tautulli.hasKey) {
    return (
      <Button size="sm" onClick={() => void history.setKey(null)}>
        Remove key
      </Button>
    );
  }
  return (
    <form
      className="info-key"
      onSubmit={(e) => {
        e.preventDefault();
        if (!text.trim()) return;
        setBusy(true);
        void history.setKey(text.trim()).then((ok) => {
          setBusy(false);
          if (ok) setText("");
        });
      }}
    >
      <input
        type="password"
        className="info-key__field"
        value={text}
        onChange={(e) => setText(e.target.value)}
        placeholder="API key"
        autoComplete="off"
        spellCheck={false}
        aria-label="Tautulli key"
        aria-describedby={ids.desc}
      />
      <Button size="sm" type="submit" disabled={busy || !text.trim()}>
        Save key
      </Button>
    </form>
  );
}
