import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { mockMatched, mockPlexMissed } from "../lib/mock/fixtures";
import { DEFAULT_SETTINGS } from "../lib/types";
import { AlbumPage } from "../pages/AlbumPage";
import { SettingsPage } from "../pages/SettingsPage";
import { loadSettings } from "../stores/settings";

const mock = backend as MockBackend;
const fx = mock.fx;
const missed = fx.albums.find((a) => mockPlexMissed(fx.albums, a))!;
const probably = fx.albums.find((a) => !mockMatched(a) && !mockPlexMissed(fx.albums, a))!;
const demo = fx.albums.find((a) => a.subformats.includes("demo"))!;
const released = fx.albums.find((a) => mockMatched(a) && !a.subformats.includes("demo"))!;

beforeEach(async () => {
  await backend.settingsSet({ ...DEFAULT_SETTINGS });
  await loadSettings();
});

describe("release status", () => {
  it("the mock has an album of each kind", async () => {
    expect([missed, probably, demo, released].every(Boolean)).toBe(true);
    expect((await backend.albumReleaseStatus(missed.id)).status).toBe("plexMissed");
    expect((await backend.albumReleaseStatus(probably.id)).status).toBe("probablyUnreleased");
    expect((await backend.albumReleaseStatus(demo.id)).status).toBe("taggedUnreleased");
    expect((await backend.albumReleaseStatus(released.id)).status).toBe("released");
  });

  it("a probably unreleased album says why, MusicBrainz included", async () => {
    const user = userEvent.setup();
    render(<AlbumPage id={probably.id} />);
    expect(await screen.findByText("Probably unreleased")).toBeInTheDocument();
    const why = screen.getByRole("button", { name: "Why probably unreleased?" });
    await user.click(why);
    const pop = screen.getByRole("dialog", { name: "Probably unreleased" });
    expect(within(pop).getByText("Plex couldn't match it to a known release.")).toBeInTheDocument();
    expect(within(pop).getByText(/MusicBrainz has no release/)).toBeInTheDocument();
    expect(within(pop).queryByText(/Fix match/)).toBeNull();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog", { name: "Probably unreleased" })).toBeNull();
    expect(why).toHaveFocus();
  });

  it("an album Plex missed says so and points at Fix match", async () => {
    const user = userEvent.setup();
    render(<AlbumPage id={missed.id} />);
    expect(await screen.findByText("Released, Plex missed it")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Why released, plex missed it?" }));
    const pop = screen.getByRole("dialog", { name: "Released, Plex missed it" });
    expect(within(pop).getByText(/MusicBrainz has an official release/)).toBeInTheDocument();
    expect(within(pop).getByText(/Fix match/)).toBeInTheDocument();
  });

  it("tagged and plainly released albums show no label", async () => {
    for (const a of [demo, released]) {
      const { unmount } = render(<AlbumPage id={a.id} />);
      await screen.findByRole("heading", { name: a.title });
      await new Promise((r) => setTimeout(r, 50));
      expect(screen.queryByRole("button", { name: /^Why / })).toBeNull();
      unmount();
    }
  });

  it("with the check off, the album Plex missed goes back to probably unreleased", async () => {
    await backend.settingsSet({ checkUnmatchedMusicbrainz: false });
    await loadSettings();
    const s = await backend.albumReleaseStatus(missed.id);
    expect(s.status).toBe("probablyUnreleased");
    // MusicBrainz says nothing either way; "Why?" says it wasn't asked.
    expect(s.reasons.some((r) => r.includes("MusicBrainz has"))).toBe(false);
    expect(s.reasons).toContain("MusicBrainz not checked.");
    // So does MusicBrainz itself being off.
    await backend.settingsSet({ checkUnmatchedMusicbrainz: true, infoMusicbrainz: false });
    expect((await backend.albumReleaseStatus(missed.id)).status).toBe("probablyUnreleased");
  });

  it("sharing follows it: an album Plex missed gets a public link", async () => {
    expect((await backend.shareInfo("album", missed.id)).public).not.toBe("unreleased");
    expect((await backend.shareInfo("album", probably.id)).public).toBe("unreleased");
  });
});

describe("settings", () => {
  it("checking MusicBrainz is on by default, next to the unmatched row, and the switch turns it off", async () => {
    const user = userEvent.setup();
    render(<SettingsPage />);
    const sw = await screen.findByRole("switch", { name: "Check MusicBrainz for albums Plex couldn't match" });
    expect(sw).toBeChecked();
    const rows = [...document.querySelectorAll("[data-setting]")].map((r) => r.getAttribute("data-setting"));
    expect(rows.indexOf("checkUnmatchedMusicbrainz")).toBe(rows.indexOf("countUnmatchedUnreleased") + 1);
    await user.click(sw);
    expect((await backend.settingsGet()).checkUnmatchedMusicbrainz).toBe(false);
  });

  it("needs MusicBrainz on", async () => {
    await backend.settingsSet({ infoMusicbrainz: false });
    await loadSettings();
    render(<SettingsPage />);
    const sw = await screen.findByRole("switch", { name: "Check MusicBrainz for albums Plex couldn't match" });
    expect(sw).toBeDisabled();
    expect(sw.closest("[data-setting]")).toHaveTextContent("Turn on MusicBrainz under Artist info to use this.");
  });
});
