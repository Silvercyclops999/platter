/**
 * Change the mood, on the phone: the moods the next songs can lean toward,
 * from your own library. Picking one puts eight songs up next; the queue
 * carries on as it was after them.
 */
import { useState } from "react";
import type { Track, TurnChoice } from "../lib/types";
import { turnTo } from "../stores/turn";
import { SheetFrame, tap } from "./bits";
import { phone } from "./phoneUi";

export { useTurnChoices } from "../stores/turn";

export function TurnSheet({ track, choices }: { track: Track; choices: TurnChoice[] }) {
  const [busy, setBusy] = useState<string | null>(null);
  return (
    <SheetFrame title="Change the mood" subtitle={<>The next 8 songs lean this way, from your library.</>} onClose={phone.pop} dimBack>
      <ul className="ph-rows" aria-label={`Moods to turn ${track.title} toward`}>
        {choices.map((c) => (
          <li key={c.mood}>
            <button
              type="button"
              className="ph-row ph-row--text"
              disabled={busy !== null}
              onClick={() => {
                tap("selection");
                setBusy(c.mood);
                void turnTo(c.mood).then(() => {
                  setBusy(null);
                  phone.pop();
                });
              }}
            >
              {c.mood}
            </button>
          </li>
        ))}
      </ul>
    </SheetFrame>
  );
}
