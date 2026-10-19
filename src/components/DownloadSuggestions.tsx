/**
 * What your other devices downloaded that this one hasn't: one tap
 * downloads it here too, or Not now stops suggesting it on this device.
 * Downloads stay per device; nothing here downloads by itself.
 */
import { useCallback, useEffect, useRef, useState } from "react";
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
  // Only the latest read lands; an older answer arriving late is dropped.
  const seq = useRef(0);
  const load = useCallback(() => {
    const n = ++seq.current;
    if (!on) {
      setList([]);
      return;
    }
    backend.deviceDownloadSuggestions().then(
      (l) => n === seq.current && setList(l),
      // Keep what's shown; the next change reads again.
      (e) => console.warn("download suggestions", e),
    );
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
      seq.current++;
    };
  }, [load]);
  const download = async (s: DownloadSuggestion) => {
    if (await setDownload({ kind: s.kind, id: s.id, title: s.title }, true)) {
      setList((l) => l.filter((x) => !(x.kind === s.kind && x.id === s.id)));
      toast.info(`Downloading ${s.title}`);
    }
  };
  const dismiss = (s: DownloadSuggestion) => {
    const same = (x: DownloadSuggestion) => x.kind === s.kind && x.id === s.id;
    setList((l) => l.filter((x) => !same(x)));
    backend.deviceDownloadDismiss(s.kind, s.kind === "liked" ? null : s.id).catch((e) => {
      // Not hidden after all: it comes back.
      setList((l) => (l.some(same) ? l : [...l, s].sort((a, b) => b.addedAt - a.addedAt)));
      toast.error(failText("Couldn't hide that suggestion", e));
    });
  };
  return { list, download, dismiss };
}
