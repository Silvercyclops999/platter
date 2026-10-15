/**
 * Settings for the Discogs import: the personal token (checked with
 * Discogs, then kept in the keychain; it never comes back to the UI) and
 * the Import button.
 */
import { useEffect, useState } from "react";
import { Button } from "../components/Button";
import { Skeleton } from "../components/Skeleton";
import { backend } from "../lib/backend";
import { plural } from "../lib/format";
import type { DiscogsStatus } from "../lib/types";
import { ensurePhysical, usePhysical } from "../stores/physical";
import { nav } from "../stores/router";
import { toast } from "../stores/toasts";
import { errText } from "./catalogue";
import { importDiscogs } from "./ShelfPage";

export const DISCOGS_TOKEN_DESCRIPTION =
  "Bring in the records in your Discogs collection. Make a personal access token in your Discogs developer settings and paste it here; it's kept in this computer's keychain. Only the token and your user name go to Discogs.";

/** The Discogs status, loaded on first use. */
export function useDiscogs(): DiscogsStatus | null {
  const st = usePhysical((s) => s.discogs);
  useEffect(() => {
    ensurePhysical();
    void usePhysical.getState().loadDiscogs();
  }, []);
  return st;
}

/** The note under the token row. */
export function discogsNote(st: DiscogsStatus | null): string | undefined {
  if (!st?.saved) return undefined;
  const r = st.report;
  const who = st.username ? `Saved for ${st.username}.` : "Saved.";
  return r ? `${who} Last read ${plural(r.total, "record")}, ${r.matched} in your library.` : who;
}

export function DiscogsTokenControl({ ids, st }: { ids: { title: string; desc: string }; st: DiscogsStatus | null }) {
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  if (!st) return <Skeleton width={160} height={32} radius="pill" />;
  const set = async (token: string | null) => {
    setBusy(true);
    try {
      usePhysical.setState({ discogs: await backend.discogsTokenSet(token) });
      setText("");
      if (token) toast.info("Discogs token saved");
    } catch (e) {
      toast.error(errText(e) || "Couldn't save the token");
    } finally {
      setBusy(false);
    }
  };
  if (st.saved)
    return (
      <Button size="sm" disabled={busy} onClick={() => void set(null)}>
        Remove token
      </Button>
    );
  return (
    <form
      className="info-key"
      onSubmit={(e) => {
        e.preventDefault();
        if (text.trim()) void set(text.trim());
      }}
    >
      <input
        type="password"
        className="info-key__field"
        value={text}
        onChange={(e) => setText(e.target.value)}
        placeholder="Personal access token"
        autoComplete="off"
        spellCheck={false}
        aria-label="Discogs personal access token"
        aria-describedby={ids.desc}
      />
      <Button size="sm" type="submit" disabled={busy || !text.trim()}>
        {busy ? "Checking…" : "Save token"}
      </Button>
    </form>
  );
}

export function DiscogsImportControl({ st }: { st: DiscogsStatus | null }) {
  if (!st?.saved) return null;
  return (
    <span className="shelf-buttons">
      <Button size="sm" disabled={st.importing} onClick={() => void importDiscogs()}>
        {st.importing ? "Reading…" : st.report ? "Read again" : "Import collection"}
      </Button>
      {st.report && (
        <Button size="sm" variant="ghost" onClick={() => nav.go({ name: "shelf" })}>
          See what it found
        </Button>
      )}
    </span>
  );
}
