import { useEffect, useState } from "react";
import { Select } from "../components/Select";
import { backend } from "../lib/backend";
import type { LibraryServers } from "../lib/types";
import { useApp } from "../stores/app";
import { updateSettings, useSettings } from "../stores/settings";

export const ALL_SERVERS = "all";

/** The servers the library has music from; reloads when the library or the libraries setting changes. */
export function useLibraryServers(): LibraryServers | null {
  const version = useApp((s) => s.libraryVersion);
  const libraries = useSettings((s) => s.settings?.libraries);
  const [servers, setServers] = useState<LibraryServers | null>(null);
  useEffect(() => {
    let alive = true;
    backend.libraryServers().then(
      (v) => alive && setServers(v),
      () => alive && setServers(null),
    );
    return () => {
      alive = false;
    };
  }, [version, libraries]);
  return servers;
}

/**
 * The sidebar header's server switcher: All servers (the same album once) or
 * one server's music. Only there with more than one server included.
 */
export function ServerSwitcher() {
  const servers = useLibraryServers();
  const [picked, setPicked] = useState<string | null>(null);
  if (!servers || servers.servers.length < 2) return null;
  const value = picked ?? servers.view;
  const options = [{ value: ALL_SERVERS, label: "All servers" }, ...servers.servers.map((s) => ({ value: s.tag, label: s.name }))];
  return (
    <div className="lib-servers">
      <Select
        value={value}
        options={options}
        label="Show music from"
        onChange={(v) => {
          setPicked(v);
          // A refused change is put back (updateSettings says why).
          void updateSettings({ serverView: v }).then(() => {
            if (useSettings.getState().settings?.serverView !== v) setPicked(null);
          });
        }}
      />
    </div>
  );
}
