import { useEffect } from "react";
import { useSettings } from "../stores/settings";
import { useUi } from "../stores/ui";
import { artUrl } from "./backend";
import { NEUTRAL_HEADER, headerColorsFromImageUrl, headerColorsFromServer } from "./color";

/**
 * Colour the page header from artwork: the art's most vivid colour, sampled
 * from the image. The server's colour is the fallback when the image can't be
 * read. The change eases over the long motion band (registered `--tint` and
 * `--tint-shade` in shell.css). With the Artwork colours setting off, every
 * header gets the same neutral tone.
 */
export function useArtTint(serverTint: string | null | undefined, artPath: string | null | undefined) {
  const colours = useSettings((s) => s.settings?.artworkColours !== false);
  useEffect(() => {
    if (!colours) {
      useUi.getState().setTint(NEUTRAL_HEADER);
      return;
    }
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
  }, [serverTint, artPath, colours]);
}
