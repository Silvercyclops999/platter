import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS, type AlbumCredits } from "../lib/types";
import { songCredits } from "../lib/useAlbumCredits";
import { AlbumPage } from "../pages/AlbumPage";
import { albumCreditGroups, releaseDateFor } from "../pages/album";
import { withAlbumCredits } from "../shell/nowPlaying";
import { useRouter } from "../stores/router";
import { loadSettings } from "../stores/settings";

const mock = backend as MockBackend;

beforeEach(async () => {
  await backend.settingsSet({ ...DEFAULT_SETTINGS });
  await loadSettings();
});

/** An album the mock has credits for, and one it has no match for. */
async function albums() {
  let found: string | null = null;
  let none: string | null = null;
  for (const a of mock.fx.albums) {
    const c = await backend.albumCredits(a.id);
    if (c.state === "found" && !found) found = a.id;
    if (c.state === "none" && !none) none = a.id;
    if (found && none) break;
  }
  return { found: found!, none: none! };
}

const credits = (p: Partial<AlbumCredits>): AlbumCredits => ({
  albumId: "1",
  state: "found",
  released: null,
  editionDate: null,
  label: null,
  credits: [],
  tracks: [],
  url: null,
  ...p,
});

describe("album credit rules", () => {
  it("groups people by role in a fixed order, leaving out featured artists", () => {
    const g = albumCreditGroups(
      credits({
        credits: [
          { name: "Writer One", artistId: null, roles: ["writer"] },
          { name: "Prod", artistId: "9", roles: ["producer", "mixer"] },
          { name: "Guest", artistId: null, roles: ["featured"] },
        ],
      }),
    );
    expect(g.map((x) => [x.label, x.people.map((p) => p.name)])).toEqual([
      ["Produced by", ["Prod"]],
      ["Written by", ["Writer One"]],
      ["Mixed by", ["Prod"]],
    ]);
    expect(albumCreditGroups(credits({ state: "pending", credits: [{ name: "X", artistId: null, roles: ["producer"] }] }))).toEqual([]);
  });

  it("shows MusicBrainz's original date when it has the full day, else Plex's", () => {
    expect(releaseDateFor("2004-03-01", credits({ released: "1997-05-21" }))).toBe("1997-05-21");
    expect(releaseDateFor("2004-03-01", credits({ released: "1997" }))).toBe("2004-03-01");
    expect(releaseDateFor("2004-03-01", null)).toBe("2004-03-01");
    expect(releaseDateFor(null, null)).toBeNull();
  });

  it("a song's credits merge into the now-playing card by name", () => {
    const c = credits({
      tracks: [
        {
          trackId: "t1",
          credits: [
            { name: "Main Act", artistId: "1", roles: ["featured", "writer"] },
            { name: "Prod", artistId: null, roles: ["producer", "mixer"] },
            { name: "Guest", artistId: "5", roles: ["featured"] },
          ],
        },
      ],
    });
    const rows = withAlbumCredits([{ name: "Main Act", id: "1", roles: ["Main artist"] }], songCredits(c, "t1"));
    expect(rows).toEqual([
      { name: "Main Act", id: "1", roles: ["Main artist", "Writer"] },
      { name: "Prod", id: null, roles: ["Producer", "Mixing"] },
      { name: "Guest", id: "5", roles: ["Featured"] },
    ]);
    expect(songCredits(c, "other")).toEqual([]);
  });
});

describe("album page credits", () => {
  it("lists producers and writers under the tracklist, with library artists linked and the source", async () => {
    const user = userEvent.setup();
    const open = vi.spyOn(backend, "openInfoLink");
    const { found } = await albums();
    const answer = await backend.albumCredits(found);
    render(<AlbumPage id={found} />);
    const section = await screen.findByRole("region", { name: "Credits" });
    expect(within(section).getByText("Produced by")).toBeInTheDocument();
    expect(within(section).getByText("Written by")).toBeInTheDocument();
    expect(within(section).getByText("Mastered by")).toBeInTheDocument();
    await user.click(within(section).getByRole("button", { name: "Credits from MusicBrainz" }));
    expect(open).toHaveBeenCalledWith(expect.stringMatching(/^https:\/\/musicbrainz\.org\/release\//));
    open.mockRestore();
    // The original release date is the one under the tracklist.
    const day = new Date(`${answer.released}T00:00:00Z`).toLocaleDateString(undefined, { year: "numeric", month: "long", day: "numeric", timeZone: "UTC" });
    expect(screen.getByText(day)).toHaveClass("album-footer__date");
    // The album's artist wrote it, and links to their page.
    const album = mock.fx.albums.find((a) => a.id === found)!;
    await user.click(within(section).getByRole("button", { name: album.artistTitle }));
    const r = useRouter.getState();
    expect(r.entries[r.index].route).toEqual({ name: "artist", id: album.artistId });
  });

  it("no match, or MusicBrainz off, shows no credits", async () => {
    const { found, none } = await albums();
    const a = render(<AlbumPage id={none} />);
    await screen.findByRole("heading", { level: 1 });
    await vi.waitFor(() => expect(screen.getAllByRole("row").length).toBeGreaterThan(0));
    expect(screen.queryByRole("region", { name: "Credits" })).toBeNull();
    a.unmount();

    await backend.settingsSet({ infoMusicbrainz: false });
    await loadSettings();
    render(<AlbumPage id={found} />);
    await screen.findByRole("heading", { level: 1 });
    expect(screen.queryByRole("region", { name: "Credits" })).toBeNull();
  });

  it("reloads when the core says the lookup finished", async () => {
    const { found } = await albums();
    const spy = vi.spyOn(backend, "albumCredits");
    render(<AlbumPage id={found} />);
    await screen.findByRole("region", { name: "Credits" });
    const before = spy.mock.calls.length;
    mock.emit("albumcredits://changed", { id: found });
    await vi.waitFor(() => expect(spy.mock.calls.length).toBe(before + 1));
    spy.mockRestore();
  });
});
