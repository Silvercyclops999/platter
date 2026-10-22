import { render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { backend } from "../lib/backend";
import { DEFAULT_SETTINGS } from "../lib/types";
import { usePhoneForm } from "../phone/form";
import { usePhoneUi } from "../phone/phoneUi";
import { heldStatusBar, holdLightStatusBar, holdStatusBar, mixIsDark, resetStatusBarHolds, STATUS_LAYER } from "../phone/statusBar";
import { LYRICS_TINT, lyricsGround } from "../phone/LyricsScreen";
import parityCss from "../phone/pages/parity.css?raw";
import { LibraryChips } from "../shell/LibraryControls";
import { useRouter } from "../stores/router";
import { loadSettings } from "../stores/settings";
import phoneLibrary from "../phone/pages/Library.tsx?raw";

describe("the phone", () => {
  beforeEach(async () => {
    usePhoneForm.setState({ phone: true });
    usePhoneUi.setState({ tab: "home", nowPlaying: false, lyrics: false, sheets: [] });
    useRouter.getState().reset({ name: "home" });
    await backend.settingsSet({ ...DEFAULT_SETTINGS });
    await loadSettings();
  });
  afterEach(() => {
    usePhoneForm.setState({ phone: false });
    resetStatusBarHolds();
  });

  it("Liked songs fills its column in the Library grid, like the covers beside it", () => {
    expect(phoneLibrary).toMatch(/<LikedCover fluid icon=/);
  });

  it("the status bar follows the view on top: the newest hold wins, and pale lyrics take dark text", () => {
    expect(heldStatusBar()).toBeNull();
    const light = holdLightStatusBar();
    expect(heldStatusBar()).toBe(true);
    const dark = holdStatusBar(false);
    expect(heldStatusBar()).toBe(false);
    dark();
    expect(heldStatusBar()).toBe(true);
    light();
    expect(heldStatusBar()).toBeNull();
    // Lyrics sit over now playing: now playing holding again later doesn't take the clock from them.
    const lyrics = holdStatusBar(false, STATUS_LAYER.lyrics);
    const np = holdStatusBar(true, STATUS_LAYER.nowPlaying);
    expect(heldStatusBar()).toBe(false);
    lyrics.set(true);
    expect(heldStatusBar()).toBe(true);
    lyrics();
    np();
    expect(heldStatusBar()).toBeNull();
    // The lyrics page is the look's page colour washed with the cover's: the model matches the CSS.
    expect(parityCss).toMatch(new RegExp(`\\.phone \\.ph-lyrics \\{[^}]*color-mix\\(in srgb, var\\(--ph-tint\\) ${Math.round(LYRICS_TINT * 100)}%, var\\(--color-bg-base\\)\\) 0`));
    // On Studio's graphite even a beige cover stays dark; on Sleeve's paper a deep red one stays light.
    expect(mixIsDark("#E9DCC0", LYRICS_TINT, "#1A1816")).toBe(true);
    expect(mixIsDark("#7A1F2B", LYRICS_TINT, "#F4F0E8")).toBe(false);
    expect(lyricsGround()).toMatch(/^#|^rgb/);
  });

  it("a hold let go after the holds were reset leaves the newer ones alone", () => {
    const stale = holdStatusBar(true, STATUS_LAYER.lyrics);
    resetStatusBarHolds();
    const fresh = holdStatusBar(false, STATUS_LAYER.nowPlaying);
    stale();
    expect(heldStatusBar()).toBe(false);
    fresh();
    expect(heldStatusBar()).toBeNull();
  });
});

describe("chip rows", () => {
  it("a row that runs out of room fades at its edge instead of cutting a word", () => {
    const sw = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "scrollWidth");
    const cw = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientWidth");
    Object.defineProperty(HTMLElement.prototype, "scrollWidth", { configurable: true, get: () => 400 });
    Object.defineProperty(HTMLElement.prototype, "clientWidth", { configurable: true, get: () => 260 });
    try {
      render(<LibraryChips filter="all" sub={null} kinds={["playlist", "album", "artist"]} downloaded={{ available: true, on: false }} />);
      expect(screen.getByRole("group", { name: "Show in library" }).parentElement).toHaveClass("has-after");
    } finally {
      if (sw) Object.defineProperty(HTMLElement.prototype, "scrollWidth", sw);
      if (cw) Object.defineProperty(HTMLElement.prototype, "clientWidth", cw);
    }
  });
});

