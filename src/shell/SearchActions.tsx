import { ListEnd } from "lucide-react";
import { Tooltip } from "../components/Tooltip";
import type { SearchHit } from "../lib/types";
import { hitInfo, queueHit } from "./searchModel";

/**
 * The hover "Add to queue" button on a song result, in the search dropdown
 * and on the results page. It leaves the dropdown and the typed text as they
 * are; the usual "Added to queue" toast confirms it.
 */
export function QueueButton({ hit, className }: { hit: SearchHit; className: string }) {
  const title = hitInfo(hit)?.title ?? "";
  return (
    <Tooltip label="Add to queue">
      <button
        type="button"
        tabIndex={-1}
        className={className}
        aria-label={`Add ${title} to queue`}
        onClick={(e) => {
          e.stopPropagation();
          void queueHit(hit);
        }}
        onDoubleClick={(e) => e.stopPropagation()}
      >
        <ListEnd size={18} aria-hidden="true" />
      </button>
    </Tooltip>
  );
}
