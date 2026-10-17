/**
 * Automix in the browser mock (docs/IPC.md "Automix and song analysis"):
 * playlists with Mix on and their gap picks, and made-up tempos so rows can
 * show "N BPM". Songs of one album in album order can't be mixed, as in the
 * core.
 */
import type { AutomixView, MixStyle, MixedPlaylists, Track } from "../types";

/** A steady made-up tempo per song (some songs have none). */
export function mockBpm(id: string): number | null {
  let h = 0;
  for (const c of id) h = (h * 31 + c.charCodeAt(0)) >>> 0;
  return h % 7 === 0 ? null : 80 + (h % 70);
}

export class MockAutomix {
  mixed: MixedPlaylists = {};

  constructor(
    private readonly track: (id: string) => Track | undefined,
    private readonly changed: () => void,
  ) {}

  list(): MixedPlaylists {
    return structuredClone(this.mixed);
  }

  set(id: string, on: boolean): MixedPlaylists {
    if (on && !this.mixed[id]) this.mixed[id] = { styles: {} };
    if (!on) delete this.mixed[id];
    this.changed();
    return this.list();
  }

  setStyle(id: string, fromId: string, toId: string, style: MixStyle): MixedPlaylists {
    const p = this.mixed[id];
    if (!p) throw { kind: "invalid", message: "Turn on mix for this playlist first." };
    const k = `${fromId}>${toId}`;
    if (style === "auto") delete p.styles[k];
    else p.styles[k] = style;
    this.changed();
    return this.list();
  }

  view(playlistId: string | null, trackIds: string[]): AutomixView {
    const styles = (playlistId && this.mixed[playlistId]?.styles) || {};
    const tracks = trackIds.map((id) => this.track(id));
    const songs = trackIds.map((id) => ({ id, bpm: mockBpm(id), key: null }));
    const gaps = trackIds.slice(1).map((toId, i) => {
      const fromId = trackIds[i];
      const a = tracks[i];
      const b = tracks[i + 1];
      const album =
        !!a && !!b && a.albumId === b.albumId && ((b.discNumber === a.discNumber && b.trackNumber === a.trackNumber + 1) || (b.discNumber === a.discNumber + 1 && b.trackNumber <= 1));
      const bpmA = mockBpm(fromId);
      const bpmB = mockBpm(toId);
      return {
        fromId,
        toId,
        reason: album ? "Songs from one album, in order" : null,
        style: styles[`${fromId}>${toId}`] ?? "auto",
        beatmatched: !album && bpmA != null && bpmB != null && Math.abs(bpmA / bpmB - 1) <= 0.06,
      };
    });
    return { songs, gaps };
  }

  reset() {
    this.mixed = {};
  }
}
