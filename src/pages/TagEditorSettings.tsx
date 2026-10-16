/**
 * Settings, Library, Tag editor: the music folder, the tag check for the
 * whole library, and Plex's library settings (read only).
 */
import { CircleCheck, CircleHelp, CircleX } from "lucide-react";
import { useEffect, useId, useState } from "react";
import { Button } from "../components/Button";
import { backend } from "../lib/backend";
import type { MusicFolderCheck, PlexLibraryCheck } from "../lib/types";
import { errText } from "../stores/plexEdit";
import { useSettings } from "../stores/settings";
import { useTagEdit } from "../stores/tagEdit";
import { toast } from "../stores/toasts";
import "../components/plex-edit.css";
import "../components/tag-edit.css";

/** The line under "Music folder": where it is and whether files were found. */
export function MusicFolderNote() {
  const folder = useSettings((s) => s.settings?.musicFolder ?? "");
  const [check, setCheck] = useState<MusicFolderCheck | null>(null);
  useEffect(() => {
    let live = true;
    setCheck(null);
    if (!folder) return;
    backend.tagEditMusicFolderStatus().then(
      (c) => live && setCheck(c),
      () => live && setCheck(null),
    );
    return () => {
      live = false;
    };
  }, [folder]);
  if (!folder) return <span>Not set. Tag details are view only until it is.</span>;
  return (
    <span role="status" aria-live="polite">
      {folder}
      {check ? `: ${check.message}` : ""}
    </span>
  );
}

function said(c: MusicFolderCheck) {
  if (c.mapping && c.found > 0) toast.info(c.ok ? c.message : `Music folder set. ${c.message}`);
  else toast.error(c.message);
}

/** Choose (system dialog) and Clear. */
export function MusicFolderControl() {
  const folder = useSettings((s) => s.settings?.musicFolder ?? "");
  const [busy, setBusy] = useState(false);
  const run = async (fn: () => Promise<MusicFolderCheck | null | void>) => {
    setBusy(true);
    try {
      const c = await fn();
      if (c) said(c);
    } catch (e) {
      toast.error(errText(e) || "Couldn't check that folder.");
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="te-folder">
      <Button size="sm" disabled={busy} onClick={() => void run(() => backend.tagEditPickMusicFolder())}>
        {folder ? "Change…" : "Choose…"}
      </Button>
      {folder && (
        <Button size="sm" variant="ghost" disabled={busy} onClick={() => void run(() => backend.tagEditClearMusicFolder())}>
          Clear
        </Button>
      )}
    </div>
  );
}

/** A path typed in, for a UNC path the picker can't reach. */
export function MusicFolderPath() {
  const id = useId();
  const [path, setPath] = useState("");
  const [busy, setBusy] = useState(false);
  const use = async () => {
    setBusy(true);
    try {
      said(await backend.tagEditSetMusicFolder(path));
    } catch (e) {
      toast.error(errText(e) || "Couldn't check that folder.");
    } finally {
      setBusy(false);
    }
  };
  return (
    <form
      className="te-folder"
      onSubmit={(e) => {
        e.preventDefault();
        if (path.trim()) void use();
      }}
    >
      <label htmlFor={id} className="sr-only">
        Music folder path
      </label>
      <input id={id} className="pe-input" value={path} placeholder="Or type a path, like \\server\music or /Volumes/Music" onChange={(e) => setPath(e.target.value)} />
      <Button size="sm" type="submit" disabled={busy || !path.trim()}>
        Use this folder
      </Button>
    </form>
  );
}

export function CheckTagsButton() {
  return (
    <Button size="sm" onClick={() => useTagEdit.getState().openCheck({ kind: "library" }, "Library")}>
      Check tags
    </Button>
  );
}

/** Plex's settings for each music library, explained. Read only. */
export function PlexLibrarySettings() {
  const [data, setData] = useState<PlexLibraryCheck[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let live = true;
    backend.tagEditLibrarySettings().then(
      (d) => live && setData(d),
      (e) => live && setError(errText(e) || "Couldn't read the library settings from Plex."),
    );
    return () => {
      live = false;
    };
  }, []);
  if (error) return <p className="te-prefs__help">{error}</p>;
  if (!data) return <p className="te-prefs__help">Reading the library settings from Plex…</p>;
  return (
    <>
      {data.map((s) => (
        <div key={s.id} className="te-prefs-section">
          <div className="te-prefs__help">
            {s.title} ({s.agent})
          </div>
          <ul className="te-prefs">
            {s.items.map((i) => {
              const Icon = i.ok === true ? CircleCheck : i.ok === false ? CircleX : CircleHelp;
              return (
                <li key={i.label}>
                  <Icon size={16} aria-hidden="true" />
                  <div>
                    <div>
                      {i.label}
                      <span className="sr-only">: {i.ok === true ? "on" : i.ok === false ? "off" : "unknown"}</span>
                    </div>
                    <div className="te-prefs__help">{i.help}</div>
                  </div>
                </li>
              );
            })}
          </ul>
        </div>
      ))}
    </>
  );
}

const SHOWN_KEY = "platter.plexLibraryCheckShown";

/** Once, on first connect: say if Plex's library settings will fight the tags. */
export async function firstConnectLibraryCheck(): Promise<void> {
  try {
    if (localStorage.getItem(SHOWN_KEY)) return;
    localStorage.setItem(SHOWN_KEY, "1");
  } catch {
    return;
  }
  try {
    const sections = await backend.tagEditLibrarySettings();
    const off = sections.flatMap((s) => s.items.filter((i) => i.ok === false).map((i) => `${i.label} (${s.title})`));
    if (off.length) toast.error(`Plex library settings need a look: ${off.join(", ")}. Settings, Library has the steps.`);
  } catch {
    // Not connected yet: Settings shows it later.
  }
}
