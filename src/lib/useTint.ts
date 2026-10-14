import { useEffect } from "react";
import { useUi } from "../stores/ui";
import { artUrl } from "./backend";
import { headerColorsFromImageUrl, headerColorsFromServer } from "./color";

/**
 * Colour the page header from artwork: the art's most vivid colour, sampled
 * from the image. The server's colour is the fallback when the image can't be
 * read. The change eases over the long motion band (registered `--tint` and
 * `--tint-shade` in shell.css).
 */
export function useArtTint(serverTint: string | null | undefined, artPath: string | null | undefined) {
  useEffect(() => {
    let alive = true;
    const fromServer = headerColorsFromServer(serverTint);
    const url = artUrl(artPath, 64);
    if (!url) {
      useUi.getState().setTint(fromServer);
      return;
    }
    headerColorsFromImageUrl(url).then((c) => alive && useUi.getState().setTint(c ?? fromServer));
    return () => {
      alive = false;
    };
  }, [serverTint, artPath]);
}
