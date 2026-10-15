/**
 * Settings controls for hidden songs and artists, scrobbling and the AI
 * provider.
 */
import { useEffect, useState } from "react";
import { Art } from "../components/Art";
import { Button } from "../components/Button";
import { Skeleton } from "../components/Skeleton";
import { Switch } from "../components/Switch";
import { backend } from "../lib/backend";
import type { AppError, ScrobbleStatus } from "../lib/types";
import { loadAi, loadScrobble, setHidden, useCurate } from "../stores/curate";
import { updateSettings, useSettings } from "../stores/settings";
import { toast } from "../stores/toasts";
import "../components/smart-editor.css";
import "./artist-info-settings.css";

type Ids = { title: string; desc: string };
const errText = (e: unknown) => (e && typeof e === "object" && "message" in e ? String((e as AppError).message) : "");

// ---------------------------------------------------------------- hidden

export function HiddenCount() {
  const n = useCurate((s) => s.hidden.length);
  return <span className="integration-state">{n === 0 ? "None" : `${n} hidden`}</span>;
}

/** Every hidden song and artist, with Unhide. */
export function HiddenList() {
  const all = useCurate((s) => s.hidden);
  if (!all.length) return <p className="integration-state">Nothing is hidden. Use Hide song on a mix or radio, or Don't play this artist on an artist or song.</p>;
  return (
    <ul className="hidden-list" aria-label="Hidden songs and artists">
      {all.map((h) => (
        <li key={`${h.kind}:${h.id}`} className="hidden-row">
          <Art path={h.thumb ?? null} size={40} round={h.kind === "artist"} kind={h.kind === "artist" ? "artist" : "album"} />
          <span className="hidden-row__text">
            <span className="hidden-row__title truncate">{h.title || "Untitled"}</span>
            <span className="hidden-row__sub truncate">{h.kind === "artist" ? "Artist" : (h.subtitle ?? "Song")}</span>
          </span>
          <Button size="sm" onClick={() => void setHidden({ kind: h.kind, id: h.id, title: h.title }, false)} aria-label={`Unhide ${h.title}`}>
            Unhide
          </Button>
        </li>
      ))}
    </ul>
  );
}

// ---------------------------------------------------------------- scrobbling

export function useScrobble(): ScrobbleStatus | null {
  const st = useCurate((s) => s.scrobble);
  useEffect(() => void loadScrobble(), []);
  return st;
}

async function run(f: () => Promise<ScrobbleStatus>, fail: string) {
  try {
    useCurate.setState({ scrobble: await f() });
  } catch (e) {
    toast.error(errText(e) || fail);
  }
}

/** What the Last.fm row says under its description. */
export function lastfmNote(st: ScrobbleStatus | null): string | undefined {
  if (!st) return undefined;
  const l = st.lastfm;
  if (l.rejected) return "Last.fm signed Platter out. Sign in again; scrobbles wait until you do.";
  if (!l.hasKey) return "Add your Last.fm API key under Artist info first.";
  if (l.signedIn) return `Signed in as ${l.user ?? "you"}.${st.queued ? ` ${st.queued} waiting to be sent.` : ""}`;
  if (l.pending) return "Allow Platter on the Last.fm page that opened, then finish here.";
  return undefined;
}

export function LastfmControl({ ids }: { ids: Ids }) {
  const s = useSettings((x) => x.settings);
  const st = useScrobble();
  const [secret, setSecret] = useState("");
  const [busy, setBusy] = useState(false);
  if (!s || !st) return <Skeleton width={160} height={32} radius="pill" />;
  const l = st.lastfm;
  if (!l.hasKey) return null;
  if (!l.hasSecret) {
    return (
      <form
        className="info-key"
        onSubmit={(e) => {
          e.preventDefault();
          if (!secret.trim()) return;
          setBusy(true);
          void run(() => backend.lastfmSecretSet(secret.trim()), "Couldn't save the secret").then(() => {
            setBusy(false);
            setSecret("");
          });
        }}
      >
        <input
          type="password"
          className="info-key__field"
          value={secret}
          placeholder="Shared secret"
          autoComplete="off"
          spellCheck={false}
          aria-label="Last.fm shared secret"
          aria-describedby={ids.desc}
          onChange={(e) => setSecret(e.target.value)}
        />
        <Button size="sm" type="submit" disabled={busy || !secret.trim()}>
          Save secret
        </Button>
      </form>
    );
  }
  if (l.pending)
    return (
      <>
        <Button size="sm" variant="primary" onClick={() => void run(() => backend.lastfmSignInFinish(), "Couldn't finish signing in")}>
          Finish sign-in
        </Button>
        <Button size="sm" onClick={() => void run(() => backend.lastfmSignOut(), "Couldn't cancel")}>
          Cancel
        </Button>
      </>
    );
  if (!l.signedIn || l.rejected)
    return (
      <Button size="sm" onClick={() => void run(() => backend.lastfmSignInStart(), "Couldn't start signing in to Last.fm")}>
        {l.rejected ? "Sign in again" : "Sign in on Last.fm"}
      </Button>
    );
  return (
    <>
      <Switch checked={s.scrobbleLastfm} onChange={(v) => void updateSettings({ scrobbleLastfm: v })} labelledBy={ids.title} describedBy={ids.desc} />
      <Button size="sm" onClick={() => void run(() => backend.lastfmSignOut(), "Couldn't sign out")}>
        Sign out
      </Button>
    </>
  );
}

export function listenbrainzNote(st: ScrobbleStatus | null): string | undefined {
  if (!st) return undefined;
  const b = st.listenbrainz;
  if (!b.hasToken) return "Add your ListenBrainz user token under Artist info first.";
  if (b.rejected) return "ListenBrainz didn't accept your token. Check it under Artist info; scrobbles wait until it works.";
  if (b.user) return `Sending as ${b.user}.`;
  return undefined;
}

export function ListenBrainzControl({ ids }: { ids: Ids }) {
  const s = useSettings((x) => x.settings);
  const st = useScrobble();
  if (!s || !st) return <Skeleton width={160} height={32} radius="pill" />;
  if (!st.listenbrainz.hasToken) return null;
  return (
    <>
      <Switch checked={s.scrobbleListenbrainz} onChange={(v) => void updateSettings({ scrobbleListenbrainz: v })} labelledBy={ids.title} describedBy={ids.desc} />
      <Button size="sm" onClick={() => void run(() => backend.listenbrainzCheck(), "Couldn't check the token")}>
        Check token
      </Button>
    </>
  );
}

// ---------------------------------------------------------------- AI provider

export function useAi() {
  const ai = useCurate((s) => s.ai);
  useEffect(() => void loadAi(), []);
  return ai;
}

export function AiKeyControl({ ids }: { ids: Ids }) {
  const ai = useAi();
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  if (!ai) return <Skeleton width={160} height={32} radius="pill" />;
  if (ai.hasKey)
    return (
      <Button
        size="sm"
        onClick={async () => {
          try {
            useCurate.setState({ ai: await backend.aiKeySet(null) });
          } catch (e) {
            toast.error(errText(e) || "Couldn't remove the key");
          }
        }}
      >
        Remove key
      </Button>
    );
  return (
    <form
      className="info-key"
      onSubmit={async (e) => {
        e.preventDefault();
        if (!text.trim()) return;
        setBusy(true);
        try {
          useCurate.setState({ ai: await backend.aiKeySet(text.trim()) });
          setText("");
        } catch (err) {
          toast.error(errText(err) || "Couldn't save the key");
        }
        setBusy(false);
      }}
    >
      <input
        type="password"
        className="info-key__field"
        value={text}
        placeholder="API key"
        autoComplete="off"
        spellCheck={false}
        aria-label="Anthropic API key"
        aria-describedby={ids.desc}
        onChange={(e) => setText(e.target.value)}
      />
      <Button size="sm" type="submit" disabled={busy || !text.trim()}>
        Save key
      </Button>
    </form>
  );
}

/** The model name; saved when the field is left. */
export function AiModelField({ ids }: { ids: Ids }) {
  const s = useSettings((x) => x.settings);
  const [text, setText] = useState<string | null>(null);
  if (!s) return <Skeleton width={160} height={32} radius="pill" />;
  const value = text ?? s.aiModel;
  return (
    <input
      className="info-key__field"
      value={value}
      spellCheck={false}
      aria-labelledby={ids.title}
      aria-describedby={ids.desc}
      onChange={(e) => setText(e.target.value)}
      onBlur={() => {
        if (text !== null && text.trim() !== s.aiModel) void updateSettings({ aiModel: text.trim() }).then(() => loadAi());
        setText(null);
      }}
    />
  );
}
