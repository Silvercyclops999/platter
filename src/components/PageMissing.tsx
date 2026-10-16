import type { AppError } from "../lib/types";
import { nav, useCanGoBack } from "../stores/router";
import { Button } from "./Button";

type What = "album" | "artist" | "playlist";

/**
 * A page whose album, artist or playlist couldn't be loaded. One that isn't
 * here reads "This album isn't in your library" with a way back; other
 * errors keep their message. Never shows a raw id.
 */
export function PageMissing({ what, error, missing }: { what: What; error: AppError; missing?: string }) {
  const canGoBack = useCanGoBack();
  const back = () => (canGoBack ? nav.back() : nav.go({ name: "home" }));
  if (error.kind !== "notFound") {
    return <p className="page-pad page-error">Couldn't load this {what}: {error.message}</p>;
  }
  return (
    <div className="page-pad page-error page-missing" role="status">
      <p className="page-missing__text">{missing ?? `This ${what} isn't in your library`}</p>
      <Button onClick={back}>{canGoBack ? "Back" : "Go Home"}</Button>
    </div>
  );
}
