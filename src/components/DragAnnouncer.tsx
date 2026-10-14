import { useEffect } from "react";
import { connectDragAutoScroll, useDrag } from "../stores/drag";

/**
 * Screen reader text for drags ("Dragging 3 songs", the target under the
 * pointer, the result), and lists that scroll while a drag nears their edge.
 */
export function DragAnnouncer() {
  const message = useDrag((s) => s.message);
  useEffect(() => connectDragAutoScroll(), []);
  return (
    <div className="sr-only" aria-live="polite" aria-atomic="true" data-testid="drag-status">
      {message}
    </div>
  );
}
