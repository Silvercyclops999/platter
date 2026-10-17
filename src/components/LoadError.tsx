import { RotateCw } from "lucide-react";
import { capReason, errorMessage } from "../lib/errorText";
import { nav, useCanGoBack } from "../stores/router";
import { Button } from "./Button";

/** Under a skeleton that's been up too long: says so, with Try again. */
export function TakingLong({ what, onRetry }: { what: string; onRetry: () => void }) {
  return (
    <div className="load-error load-error--inline" role="status">
      <p className="load-error__why">Still loading {what}. This is taking longer than usual.</p>
      <div className="load-error__actions">
        <Button icon={<RotateCw size={14} />} onClick={onRetry}>
          Try again
        </Button>
      </div>
    </div>
  );
}

/**
 * A page that couldn't load: what failed, the reason (short), Try again, and
 * a way back. Every page that can fail to load shows this one.
 */
export function LoadError({
  what,
  error,
  onRetry,
  inline = false,
}: {
  /** "this album", "Liked Songs", "Home". */
  what: string;
  error?: unknown;
  onRetry?: () => void;
  /** Inside a page section (no top padding, no Back). */
  inline?: boolean;
}) {
  const canGoBack = useCanGoBack();
  const back = () => (canGoBack ? nav.back() : nav.go({ name: "home" }));
  const why = errorMessage(error);
  return (
    <div className={`load-error ${inline ? "load-error--inline" : "page-pad page-error"}`} role="alert">
      <p className="load-error__title">Couldn't load {what}</p>
      {why && <p className="load-error__why">{capReason(why, 160)}</p>}
      <div className="load-error__actions">
        {onRetry && (
          <Button icon={<RotateCw size={14} />} onClick={onRetry}>
            Try again
          </Button>
        )}
        {!inline && (
          <Button variant="ghost" onClick={back}>
            {canGoBack ? "Back" : "Go home"}
          </Button>
        )}
      </div>
    </div>
  );
}
