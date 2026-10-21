import { act, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { Art } from "../components/Art";
import { IconButton } from "../components/Button";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS } from "../lib/types";
import { usePhoneForm } from "../phone/form";
import { PhoneShell } from "../phone/PhoneShell";
import { usePhoneUi } from "../phone/phoneUi";
import { heldStatusBar, holdLightStatusBar, holdStatusBar, mixIsDark, resetStatusBarHolds, STATUS_LAYER } from "../phone/statusBar";
import { LYRICS_TINT, lyricsGround } from "../phone/LyricsScreen";
import parityCss from "../phone/pages/parity.css?raw";
import { LibraryChips } from "../shell/LibraryControls";
import { useRouter } from "../stores/router";
import { loadSettings } from "../stores/settings";
import identityCss from "../design/identity.css?raw";
import phoneCss from "../phone/phone.css?raw";
import phoneLibrary from "../phone/pages/Library.tsx?raw";
import miniPlayer from "../phone/MiniPlayer.tsx?raw";

const mock = backend as unknown as MockBackend;

/** Every share of the cover's colour a rule mixes in, as numbers. */
function tintShares(rule: string, name: string): number[] {
  return [...rule.matchAll(new RegExp(`var\\(--${name}\\)\\s+(\\d+)%`, "g"))].map((m) => Number(m[1]));
}

describe("the sleeve header", () => {
  it("the album panel takes at most a tenth of the cover's colour, on the computer and the phone", () => {
    const desk = identityCss.match(/:root\[data-identity\] \.detail-header \{([^}]*)\}/)?.[1] ?? "";
    expect(tintShares(desk, "tint").length).toBeGreaterThan(0);
    expect(Math.max(...tintShares(desk, "tint"))).toBeLessThanOrEqual(10);
    const phone = phoneCss.match(/\.ph-album \{([^}]*)\}/)?.[1] ?? "";
    expect(Math.max(...tintShares(phone, "ph-tint"))).toBeLessThanOrEqual(10);
  });

  it("Play and Shuffle carry their word for the pill while the button keeps its name", () => {
    render(<IconButton label="Play Glass Season" variant="accent" size="xl" text="Play" icon={<span />} tooltip={false} />);
    const btn = screen.getByRole("button", { name: "Play Glass Season" });
    expect(btn).toHaveClass("icon-btn--text");
    expect(within(btn).getByText("Play")).toHaveAttribute("aria-hidden", "true");
  });
});

describe("missing art", () => {
  it("a record or playlist with no cover is a paper sleeve with the P, not a glyph", () => {
    for (const kind of ["album", "playlist"] as const) {
      const { container, unmount } = render(<Art path={null} kind={kind} size={160} />);
      expect(container.querySelector('[data-testid="paper-sleeve"] path')).not.toBeNull();
      expect(container.querySelector(".lucide")).toBeNull();
      unmount();
    }
  });

  it("a person still gets a figure, and a titled record its printed sleeve", () => {
    const person = render(<Art path={null} kind="artist" round size={160} />);
    expect(person.container.querySelector('[data-testid="paper-sleeve"]')).toBeNull();
    person.unmount();
    const titled = render(<Art path={null} kind="album" size={160} sleeve={{ title: "Glass Season" }} />);
    expect(titled.getByTestId("fallback-sleeve")).toBeInTheDocument();
  });
});

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

  it("an album opens on its sleeve: Play first, and songs by the album's artist skip the artist line", async () => {
    const album = mock.fx.albums[0];
    render(<PhoneShell />);
    act(() => useRouter.getState().navigate({ name: "album", id: album.id }));
    await screen.findByRole("heading", { name: album.title });
    const page = document.querySelector(".ph-album")!;
    expect(page.querySelector(".ph-album__sleeve .record-disc")).not.toBeNull();
    const first = page.querySelector(".ph-actions__left")!.querySelector("button")!;
    expect(first).toHaveAccessibleName(`Play ${album.title}`);
    expect(within(first).getByText("Play")).toBeInTheDocument();
    const songs = within(page as HTMLElement).getByRole("list", { name: "Songs" });
    const subs = [...songs.querySelectorAll(".ph-row__sub")];
    expect(subs.some((s) => s.textContent === album.artistTitle)).toBe(false);
    // The emptied line holds only its marks, so phone.css hides it rather than leaving a blank line.
    const bare = subs.filter((s) => !s.textContent);
    expect(bare.length).toBeGreaterThan(0);
    expect(bare.every((s) => s.matches(".is-marks-only:has(> .ph-glyphs:empty)"))).toBe(true);
    expect(phoneCss).toMatch(/\.ph-row__sub\.is-marks-only:has\(> \.ph-glyphs:empty\) \{ display: none; \}/);
  });

  it("the mini player is the look's own panel with the progress line on top, never the cover's colour", () => {
    expect(miniPlayer).not.toMatch(/--ph-tint/);
    const mini = phoneCss.match(/^\.ph-mini \{([^}]*)\}/m)?.[1] ?? "";
    expect(mini).toMatch(/background:\s*var\(--color-bg-raised2\)/);
    const line = phoneCss.match(/\.ph-mini__progress \{([^}]*)\}/)?.[1] ?? "";
    expect(line).toMatch(/top:\s*0/);
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

describe("the record beside the sleeve", () => {
  it("follows the cover's size in the banner and minimal headers, and leaves round covers alone", () => {
    expect(identityCss).toMatch(/\[data-look-header-style="banner"\] \.detail-header__art--record \{ --sleeve: 120px; \}/);
    expect(identityCss).toMatch(/\[data-look-header-style="minimal"\] \.detail-header__art--record \{ --sleeve: 96px; \}/);
    expect(identityCss).toMatch(/\[data-look-cover-shape="vinyl"\]\) \.detail-header__record \{ display: none; \}/);
  });
});
