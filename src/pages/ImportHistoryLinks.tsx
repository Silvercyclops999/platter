/** Where imported listening history shows up: "Played a lot" on the Missing page, and Stats. */
import { Button } from "../components/Button";
import type { ImportReport } from "../lib/importTypes";
import { nav } from "../stores/router";
import { useSettings } from "../stores/settings";
import { showPlayedALot } from "./importShared";

export function ImportHistoryLinks({ r }: { r: ImportReport }) {
  const played = useSettings((s) => s.settings?.showPlayedALot !== false);
  const stats = useSettings((s) => s.settings?.statsPage !== false);
  if (!r.history?.plays || (!played && !stats)) return null;
  return (
    <span className="import-history-links">
      {played && (
        <Button size="sm" variant="ghost" onClick={showPlayedALot}>
          Played a lot, not in your library
        </Button>
      )}
      {stats && (
        <Button size="sm" variant="ghost" onClick={() => nav.go({ name: "stats" })}>
          See your stats
        </Button>
      )}
    </span>
  );
}
