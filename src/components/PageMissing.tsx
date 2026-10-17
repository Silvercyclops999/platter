import type { AppError } from "../lib/types";
import { nav, useCanGoBack } from "../stores/router";
import { Button } from "./Button";
import { LoadError } from "./LoadError";

/**
 * A page whose album, artist, playlist (or mix, Blend...) couldn't be
 * loaded. One that isn't here reads "This album isn't in your library" with
 * a way back; other errors show the shared load error with Try again. Never
 * shows a raw id.
 */
export function PageMissing({ what, error, missing, onRetry }: { what: string; error: AppError; missing?: string; onRetry?: () => void }) {
  const canGoBack = useCanGoBack();
  const back = () => (canGoBack ? nav.back() : nav.go({ name: "home" }));
  if (error.kind !== "notFound") return <LoadError what={`this ${what}`} error={error} onRetry={onRetry} />;
  return (
    <div className="page-pad page-error page-missing" role="status">
      <p className="page-missing__text">{missing ?? `This ${what} isn't in your library`}</p>
      <Button onClick={back}>{canGoBack ? "Back" : "Go home"}</Button>
    </div>
  );
}
