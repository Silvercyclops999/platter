/**
 * What your other devices downloaded that this one hasn't: one tap
 * downloads it here too, or Not now stops suggesting it on this device.
 * Downloads stay per device; nothing here downloads by itself.
 */
import { useCallback, useEffect, useState } from "react";
import { backend } from "../lib/backend";
import type { DownloadSuggestion } from "../lib/types";
import { setDownload } from "../stores/downloads";
import { useSettings } from "../stores/settings";
import { toast } from "../stores/toasts";
import { failText } from "../lib/errorText";

/** "On Studio PC", "On Studio PC and Kitchen iPad". */
export function suggestionLine(s: DownloadSuggestion): string {
  const d = s.devices;
  const who = d.length <= 2 ? d.join(" and ") : `${d.slice(0, -1).join(", ")} and ${d[d.length - 1]}`;
  return `Downloaded on ${who}`;
}

/** The suggestions, kept current while shown. Empty while the setting is off. */
export function useDownloadSuggestions() {
  const on = useSettings((s) => s.settings?.syncDownloads !== false && s.settings?.deviceSync !== false);
  const [list, setList] = useState<DownloadSuggestion[]>([]);
  const load = useCallback(() => {
    if (!on) {
      setList([]);
      return;
    }
    backend.deviceDownloadSuggestions().then(setList, () => setList([]));
  }, [on]);
  useEffect(() => {
    load();
    const offStore = backend.listen("syncstore://changed", (c) => {
      if (c.namespaces.includes("downloadIntent")) load();
    });
    const offFiles = backend.listen("downloads://changed", () => load());
    return () => {
      void offStore.then((u) => u());
      void offFiles.then((u) => u());
    };
  }, [load]);
  const download = async (s: DownloadSuggestion) => {
    if (await setDownload({ kind: s.kind, id: s.id, title: s.title }, true)) {
      setList((l) => l.filter((x) => !(x.kind === s.kind && x.id === s.id)));
      toast.info(`Downloading ${s.title}`);
    }
  };
  const dismiss = (s: DownloadSuggestion) => {
    setList((l) => l.filter((x) => !(x.kind === s.kind && x.id === s.id)));
    backend.deviceDownloadDismiss(s.kind, s.kind === "liked" ? null : s.id).catch((e) => toast.error(failText("Couldn't hide that suggestion", e)));
  };
  return { list, download, dismiss };
}
