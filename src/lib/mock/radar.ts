/**
 * The release radar in the browser mock: invented new releases for fixture
 * artists, as if MusicBrainz had them. `?radar=empty` starts with nothing
 * found yet. Test hooks: `setEmpty(on)`, `reset()`.
 */
import type { AutoArtist, LidarrRequest, RadarItem, RadarPage, Settings } from "../types";
import type { Fixtures } from "./fixtures";
import { coverThumb } from "./lidarr";

const DAY = 86400;

const ymd = (unix: number) => new Date(unix * 1000).toISOString().slice(0, 10);

export interface RadarHooks {
  settings: () => Settings;
  setSettings: (patch: Partial<Settings>) => void;
  requests: () => LidarrRequest[];
  lidarrReady: () => boolean;
  emit: () => void;
}

export class MockRadar {
  private empty = false;
  private checkedAt: number | null = null;
  private auto: AutoArtist[] = [];

  constructor(
    private fx: Fixtures,
    private hooks: RadarHooks,
    empty = false,
  ) {
    this.empty = empty;
    this.reset(empty);
  }

  reset(empty = false) {
    this.empty = empty;
    this.auto = [];
    this.checkedAt = empty ? null : Math.floor(Date.now() / 1000) - 5 * 3600;
  }

  setEmpty(on: boolean) {
    this.empty = on;
  }

  /** What the faked MusicBrainz knows: new releases by followed and most played fixture artists. */
  private found(now: number): Omit<RadarItem, "request" | "auto" | "thumb">[] {
    if (this.empty) return [];
    const [a0, a1, a2, a3] = this.fx.artists;
    const own = this.fx.albums.find((al) => al.artistId === a2.id);
    const build = [...this.fx.albums].reverse().find((al) => al.artistId === a3.id);
    const item = (n: number, title: string, artist: { id: string; title: string }, kind: string, ago: number, extra: Partial<RadarItem> = {}) => ({
      releaseGroupId: `5e0c0c55-0000-4000-8000-${String(n).padStart(12, "0")}`,
      title,
      artist: artist.title,
      artistId: artist.id,
      kind,
      date: ymd(now - ago * DAY),
      libraryAlbumId: null,
      refusal: null,
      new: false,
      ...extra,
    });
    return [
      item(1, "Glass Weather", a0, "album", -12),
      item(2, "Paper Moon", a0, "album", 3, { new: true }),
      item(3, "Harbor Lights", a2, "ep", 9, { new: true }),
      item(4, "Northern Rooms", a1, "single", 18),
      ...(own ? [item(5, own.title, a2, "album", 27, { libraryAlbumId: own.id })] : []),
      ...(build
        ? [
            item(6, build.title, a3, "album", 41, {
              refusal: `It has the same title as "${build.title}", an unreleased album already in your library. The two could get mixed together, so it can't be requested.`,
            }),
          ]
        : []),
      item(7, "Static Bloom", a1, "album", 63),
    ];
  }

  page(): RadarPage {
    const s = this.hooks.settings();
    const lidarrReady = this.hooks.lidarrReady();
    if (!s.releaseRadar) return { shown: false, items: [], checkedAt: this.checkedAt, checking: false, lidarrReady, artists: 3 };
    const now = Math.floor(Date.now() / 1000);
    const requests = this.hooks.requests();
    const items = this.found(now)
      .map((f) => {
        const r = requests.find((x) => x.releaseGroupId === f.releaseGroupId);
        return { ...f, thumb: coverThumb(f.releaseGroupId), request: r?.state ?? null, auto: !!r?.auto };
      })
      .sort((a, b) => b.date.localeCompare(a.date) || a.title.localeCompare(b.title));
    return { shown: true, items, checkedAt: this.checkedAt, checking: false, lidarrReady, artists: 3 };
  }

  check(): RadarPage {
    if (!this.hooks.settings().releaseRadar) return this.page();
    this.checkedAt = Math.floor(Date.now() / 1000);
    this.empty = false;
    this.hooks.emit();
    return this.page();
  }

  autoArtists(): AutoArtist[] {
    return this.auto.map((a) => ({ ...a }));
  }

  setAuto(id: string, name: string, on: boolean): AutoArtist[] {
    if (!id.trim() || !name.trim()) throw { kind: "invalid", message: "Not an artist." };
    const had = this.auto.some((a) => a.id === id);
    if (on && !had) this.auto.push({ id, name: name.trim(), enabledAt: Math.floor(Date.now() / 1000) });
    if (!on && had) this.auto = this.auto.filter((a) => a.id !== id);
    if (on && !this.hooks.settings().autoRequest) this.hooks.setSettings({ autoRequest: true });
    this.hooks.emit();
    return this.autoArtists();
  }
}
