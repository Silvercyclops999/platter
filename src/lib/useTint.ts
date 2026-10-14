import { useEffect } from "react";
import { useUi } from "../stores/ui";
import { artUrl } from "./backend";
import { tintFromImageUrl, tintFromServer } from "./color";

/**
 * Tint the page header from artwork: the server's tint when present,
 * otherwise a dominant color sampled from the image. The color transitions
 * smoothly (registered `--tint` custom property in shell.css).
 */
export function useArtTint(serverTint: string | null | undefined, artPath: string | null | undefined) {
  useEffect(() => {
    let alive = true;
    const fromServer = tintFromServer(serverTint);
    if (fromServer) {
      useUi.getState().setTint(fromServer);
      return;
    }
    const url = artUrl(artPath, 64);
    if (!url) {
      useUi.getState().setTint(null);
      return;
    }
    tintFromImageUrl(url).then((c) => alive && useUi.getState().setTint(c));
    return () => {
      alive = false;
    };
  }, [serverTint, artPath]);
}
