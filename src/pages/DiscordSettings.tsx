import { useEffect, useState } from "react";
import { Button } from "../components/Button";
import { Skeleton } from "../components/Skeleton";
import { Switch } from "../components/Switch";
import { backend } from "../lib/backend";
import type { DiscordStatus } from "../lib/types";
import { updateSettings, useSettings } from "../stores/settings";
import { toast } from "../stores/toasts";
import "./artist-info-settings.css";

/** Discord's state for the Settings rows: loaded when the page opens, then kept current. */
export function useDiscordStatus() {
  const [status, setStatus] = useState<DiscordStatus | null>(null);
  useEffect(() => {
    let alive = true;
    backend.discordStatus().then(
      (s) => alive && setStatus(s),
      () => {},
    );
    const un = backend.listen("discord://status", (s) => alive && setStatus(s));
    return () => {
      alive = false;
      void un.then((f) => f());
    };
  }, []);
  /** Save or remove the client ID; one that's accepted turns uploads on, removing it turns them off. */
  const setKey = async (key: string | null) => {
    try {
      const st = await backend.discordImgurKeySet(key);
      setStatus((s) => (s ? { ...s, imgurKey: st, uploadFailed: false } : s));
      if (key && !st.rejected) await updateSettings({ discordUploadArt: true });
      if (!key) await updateSettings({ discordUploadArt: false });
      return st;
    } catch (e) {
      toast.error(`Couldn't save the client ID${e && typeof e === "object" && "message" in e ? `: ${String(e.message)}` : ""}`);
      return null;
    }
  };
  return { status, setKey };
}

/** The note under the Discord status row. */
export function discordNote(status: DiscordStatus | null, on: boolean | undefined): string | undefined {
  if (!status) return undefined;
  if (!status.available) return "This copy of Platter was built without a Discord app, so it can't show a status.";
  if (on && !status.connected) return "Discord isn't open. Platter shows your status once it is.";
  return undefined;
}

/** The note under the Imgur row: a refused client ID or a failed upload. */
export function imgurNote(status: DiscordStatus | null): string | undefined {
  if (!status) return undefined;
  if (status.imgurKey.rejected) return "Imgur rejected the client ID.";
  if (status.uploadFailed && status.imgurKey.saved) return "The last cover couldn't be uploaded, so Discord shows the Platter icon.";
  return undefined;
}

/** The note under the catbox.moe row: a failed upload, when catbox was the host used. */
export function catboxNote(status: DiscordStatus | null, on: boolean | undefined): string | undefined {
  if (!status || !on || !status.uploadFailed) return undefined;
  if (status.imgurKey.saved && !status.imgurKey.rejected) return undefined;
  return "The last cover couldn't be uploaded, so Discord shows the Platter icon.";
}

/**
 * The Imgur control: a client ID field until one is saved, then the on/off
 * switch and a way to remove it. The ID goes to the OS keychain and is never
 * shown again.
 */
export function ImgurKeyControl({
  ids,
  status,
  setKey,
}: {
  ids: { title: string; desc: string };
  status: DiscordStatus | null;
  setKey: (key: string | null) => Promise<unknown>;
}) {
  const s = useSettings((x) => x.settings);
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  if (!s || !status) return <Skeleton width={160} height={32} radius="pill" />;
  const key = status.imgurKey;

  if (!key.saved) {
    const save = async () => {
      if (!text.trim()) return;
      setBusy(true);
      await setKey(text.trim());
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
          placeholder="Client ID"
          autoComplete="off"
          spellCheck={false}
          aria-label="Imgur client ID"
          aria-describedby={ids.desc}
        />
        <Button size="sm" type="submit" disabled={busy || !text.trim()}>
          {busy ? "Checking…" : "Save ID"}
        </Button>
      </form>
    );
  }
  return (
    <>
      <Switch
        checked={s.discordUploadArt && !key.rejected}
        disabled={key.rejected}
        onChange={(v) => void updateSettings({ discordUploadArt: v })}
        labelledBy={ids.title}
        describedBy={ids.desc}
      />
      <Button size="sm" onClick={() => void setKey(null)}>
        Remove ID
      </Button>
    </>
  );
}
