import { useEffect, useState } from "react";
import { useSettings } from "../stores/settings";
import { useUi } from "../stores/ui";
import { artUrl } from "./backend";
import { NEUTRAL_HEADER, headerColorsFromImageUrl, headerColorsFromServer, type HeaderColors } from "./color";

/**
 * Work out artwork colours: the server's colour at once, then the art's own
 * most vivid colour once the image is sampled (the server's stays when the
 * image can't be read). Returns a cleanup that drops a late answer.
 */
function artColours(serverTint: string | null | undefined, artPath: string | null | undefined, set: (c: HeaderColors | null) => void): () => void {
  let alive = true;
  const fromServer = headerColorsFromServer(serverTint);
  const url = artUrl(artPath, 64);
  if (!url) {
    set(fromServer);
    return () => {};
  }
  headerColorsFromImageUrl(url).then((c) => alive && set(c ?? fromServer));
  return () => {
    alive = false;
  };
}

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
    return artColours(serverTint, artPath, (c) => useUi.getState().setTint(c));
  }, [serverTint, artPath, colours]);
}

/**
 * The same colours for one item that isn't the page (an album's spine on
 * the record shelf). The server's colour shows first, so a spine is never
 * blank while its art is sampled. Null with no colour, or with Artwork
 * colours off.
 */
export function useArtColours(serverTint: string | null | undefined, artPath: string | null | undefined): HeaderColors | null {
  const colours = useSettings((s) => s.settings?.artworkColours !== false);
  const [c, setC] = useState<HeaderColors | null>(() => (colours ? headerColorsFromServer(serverTint) : null));
  useEffect(() => {
    if (!colours) {
      setC(null);
      return;
    }
    setC(headerColorsFromServer(serverTint));
    return artColours(serverTint, artPath, setC);
  }, [serverTint, artPath, colours]);
  return c;
}
