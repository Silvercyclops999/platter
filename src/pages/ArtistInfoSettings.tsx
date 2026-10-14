import { useEffect, useState } from "react";
import { Button } from "../components/Button";
import { Skeleton } from "../components/Skeleton";
import { Switch } from "../components/Switch";
import { backend } from "../lib/backend";
import type { InfoKeysStatus, KeyedInfoSource, Settings } from "../lib/types";
import { updateSettings, useSettings } from "../stores/settings";
import { toast } from "../stores/toasts";
import "./artist-info-settings.css";

/** Display names, as each service writes its own. */
export const INFO_SOURCE_NAME: Record<KeyedInfoSource, string> = {
  lastfm: "Last.fm",
  fanart: "Fanart.tv",
  audiodb: "TheAudioDB",
  listenbrainz: "ListenBrainz",
};

/** The settings switch for each keyed source. */
export const INFO_SWITCH: Record<KeyedInfoSource, keyof Settings & `info${string}`> = {
  lastfm: "infoLastfm",
  fanart: "infoFanart",
  audiodb: "infoAudiodb",
  listenbrainz: "infoListenbrainz",
};

/** Sources that work without a key; a key (token) only helps. Saving or removing one leaves the switch alone. */
export const OPTIONAL_KEY: ReadonlySet<KeyedInfoSource> = new Set(["listenbrainz"]);

/** "Last.fm rejected the key." for a refused key, else nothing. */
export function keyNote(source: KeyedInfoSource, keys: InfoKeysStatus | null): string | undefined {
  const what = OPTIONAL_KEY.has(source) ? "token" : "key";
  return keys?.[source].rejected ? `${INFO_SOURCE_NAME[source]} rejected the ${what}.` : undefined;
}

/** Key status for the Settings rows, loaded when the page opens. */
export function useInfoKeys() {
  const [keys, setKeys] = useState<InfoKeysStatus | null>(null);
  useEffect(() => {
    let alive = true;
    backend.artistInfoKeys().then(
      (k) => alive && setKeys(k),
      () => {},
    );
    return () => {
      alive = false;
    };
  }, []);
  /** Save or remove a key; a key that's accepted turns its source on, removing one turns it off. */
  const setKey = async (source: KeyedInfoSource, key: string | null) => {
    try {
      const st = await backend.artistInfoKeySet(source, key);
      setKeys((k) => (k ? { ...k, [source]: st } : k));
      if (!OPTIONAL_KEY.has(source)) {
        if (key && !st.rejected) await updateSettings({ [INFO_SWITCH[source]]: true });
        if (!key) await updateSettings({ [INFO_SWITCH[source]]: false });
      }
      return st;
    } catch (e) {
      toast.error(`Couldn't save the key${e && typeof e === "object" && "message" in e ? `: ${String(e.message)}` : ""}`);
      return null;
    }
  };
  return { keys, setKey };
}

/**
 * A keyed source's control: a key field until a key is saved, then the
 * on/off switch and a way to remove the key. Keys go to the OS keychain and
 * are never shown again. A source with an optional key (ListenBrainz) has
 * its own switch row; here it gets only the token field or "Remove token".
 */
export function InfoKeyControl({
  source,
  ids,
  keys,
  setKey,
}: {
  source: KeyedInfoSource;
  ids: { title: string; desc: string };
  keys: InfoKeysStatus | null;
  setKey: (source: KeyedInfoSource, key: string | null) => Promise<unknown>;
}) {
  const s = useSettings((x) => x.settings);
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  if (!s || !keys) return <Skeleton width={160} height={32} radius="pill" />;
  const state = keys[source];
  const name = INFO_SOURCE_NAME[source];
  const optional = OPTIONAL_KEY.has(source);
  const what = optional ? "token" : "key";

  if (!state.saved) {
    const save = async () => {
      if (!text.trim()) return;
      setBusy(true);
      await setKey(source, text.trim());
      setBusy(false);
      setText("");
    };
    return (
      <form
        className="info-key"
        onSubmit={(e) => {
          e.preventDefault();
          void save();
        }}
      >
        <input
          type="password"
          className="info-key__field"
          value={text}
          onChange={(e) => setText(e.target.value)}
          placeholder={optional ? "User token" : "API key"}
          autoComplete="off"
          spellCheck={false}
          aria-label={optional ? `${name} user token` : `${name} API key`}
          aria-describedby={ids.desc}
        />
        <Button size="sm" type="submit" disabled={busy || !text.trim()}>
          {busy ? "Checking…" : `Save ${what}`}
        </Button>
      </form>
    );
  }
  if (optional) {
    return (
      <Button size="sm" onClick={() => void setKey(source, null)}>
        Remove token
      </Button>
    );
  }
  return (
    <>
      <Switch
        checked={!!s[INFO_SWITCH[source]] && !state.rejected}
        disabled={state.rejected}
        onChange={(v) => void updateSettings({ [INFO_SWITCH[source]]: v })}
        labelledBy={ids.title}
        describedBy={ids.desc}
      />
      <Button size="sm" onClick={() => void setKey(source, null)}>
        Remove key
      </Button>
    </>
  );
}
