/**
 * Identify by sound settings: the progress note, the user's own AcoustID
 * key (kept in the keychain) and which songs to identify.
 */
import { failText } from "../lib/errorText";
import { useEffect, useState } from "react";
import { Button } from "../components/Button";
import type { SelectOption } from "../components/Select";
import { Skeleton } from "../components/Skeleton";
import { backend } from "../lib/backend";
import type { SoundIdScope, SoundIdStatus } from "../lib/types";
import { toast } from "../stores/toasts";
import "./artist-info-settings.css";

export const SOUND_ID_SCOPE_OPTIONS = [
  { value: "untagged", label: "Untagged only" },
  { value: "all", label: "All songs" },
] as const satisfies SelectOption<SoundIdScope>[];

/** What the sentence under the switch says Platter sends. Kept in one place so the row and tests agree. */
export const SOUND_ID_DESCRIPTION =
  "Makes an audio fingerprint of each song on this computer from its first two minutes and looks it up on AcoustID, a free service, to find untagged or mislabelled files. Only the fingerprint and the song's length are sent: no file, name, tag or account details. Unreleased songs only get a \"sounds like\" hint and are never renamed or matched to a release. This device only.";

/** The status, loaded when the page opens and kept current. */
export function useSoundIdStatus() {
  const [status, setStatus] = useState<SoundIdStatus | null>(null);
  useEffect(() => {
    let alive = true;
    const load = () =>
      backend.soundIdStatus().then(
        (s) => alive && setStatus(s),
        () => {},
      );
    void load();
    const un = backend.listen("soundid://changed", () => void load());
    return () => {
      alive = false;
      void un.then((f) => f());
    };
  }, []);
  const setKey = async (key: string | null) => {
    try {
      const st = await backend.soundIdKeySet(key);
      setStatus(st);
      return st;
    } catch (e) {
      toast.error(failText(key ? "Couldn't save the AcoustID key" : "Couldn't remove the AcoustID key", e));
      return null;
    }
  };
  return { status, setKey };
}

const n = (v: number) => v.toLocaleString();

/** The note under the switch: no key, a refused key, or progress. */
export function soundIdNote(status: SoundIdStatus | null, on: boolean | undefined): { text: string; error: boolean } | undefined {
  if (!status) return undefined;
  if (!status.configured) return { text: "This copy of Platter has no AcoustID key. Add your own below to use it.", error: !!on };
  if (status.keyRefused) return { text: "AcoustID refused the key. Add another one below.", error: true };
  if (!on) return undefined;
  if (status.paused) return { text: "AcoustID or your server can't be reached right now. Platter tries again in a few minutes.", error: false };
  if (status.total === 0) return { text: "No songs to identify.", error: false };
  return { text: `Identified ${n(status.identified)} of ${n(status.done)} songs checked (${n(status.total)} in all).`, error: false };
}

/** A key field until the user saves one, then Remove. */
export function AcoustIdKeyControl({
  ids,
  status,
  setKey,
}: {
  ids: { title: string; desc: string };
  status: SoundIdStatus | null;
  setKey: (key: string | null) => Promise<unknown>;
}) {
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  if (!status) return <Skeleton width={160} height={32} radius="pill" />;
  if (status.userKey) {
    return (
      <Button size="sm" onClick={() => void setKey(null)}>
        Remove key
      </Button>
    );
  }
  const save = async () => {
    if (!text.trim()) return;
    setBusy(true);
    const saved = await setKey(text.trim());
    setBusy(false);
    // A failed save keeps the key in the field to try again.
    if (saved) setText("");
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
        placeholder="Application key"
        autoComplete="off"
        spellCheck={false}
        aria-label="AcoustID application key"
        aria-describedby={ids.desc}
      />
      <Button size="sm" type="submit" disabled={busy || !text.trim()}>
        Save key
      </Button>
    </form>
  );
}
