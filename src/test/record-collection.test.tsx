/**
 * The record collection look on Search and now playing: genres as crate
 * dividers with the spines of their records, "From" in one plain line, and
 * the cover's colour kept to a fifth of the room.
 */
import { render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import phoneCss from "../phone/phone.css?raw";
import identityCss from "../design/identity.css?raw";
import { backend } from "../lib/backend";
import { DEFAULT_SETTINGS, type Album } from "../lib/types";
import { usePhoneForm } from "../phone/form";
import { NP_TOP_TINT } from "../phone/NowPlaying";
import { PhoneSearch, spineColour } from "../phone/pages/Search";
import { loadSettings } from "../stores/settings";

beforeEach(async () => {
  usePhoneForm.setState({ phone: true });
  await backend.settingsSet({ ...DEFAULT_SETTINGS });
  await loadSettings();
});

afterEach(() => {
  usePhoneForm.setState({ phone: false });
});

describe("Browse the crates", () => {
  it("each genre is a divider: its name on the tab, how many records, and their spines; no covers", async () => {
    const { crates } = await backend.crates("genre");
    render(<PhoneSearch />);
    const section = (await screen.findByRole("heading", { name: "Browse the crates" }, { timeout: 4000 })).closest("section")!;
    const dividers = await waitFor(() => {
      const d = within(section).getAllByTestId("crate-divider");
      expect(d.length).toBeGreaterThan(0);
      return d;
    });
    const first = crates[0];
    expect(dividers[0].querySelector(".ph-divider__tab")?.textContent).toBe(first.name);
    expect(dividers[0].querySelector(".ph-divider__count")?.textContent).toBe(`${first.count} record${first.count === 1 ? "" : "s"}`);
    expect(dividers[0].querySelectorAll(".ph-divider__spines > span").length).toBe(Math.min(5, first.albums.length));
    expect(section.querySelector(".art, img")).toBeNull();
  });

  it("a spine takes the cover's colour, or a cloth colour that stays the same for the record", () => {
    const album = (over: Partial<Album>) => ({ id: "a1", title: "x", artistTitle: "y", tint: null, ...over }) as Album;
    expect(spineColour(album({ id: "a9" }))).toBe(spineColour(album({ id: "a9" })));
    expect(spineColour(album({ id: "a9" }))).toMatch(/^#[0-9a-f]{6}$/);
    expect(spineColour(album({ tint: "#c0392b" }))).not.toBe(spineColour(album({ id: "zz" })));
  });

  it("no tilted covers on the phone", () => {
    expect(phoneCss).not.toMatch(/transform:\s*rotate\(1\dX?deg\)/);
    expect(phoneCss).not.toContain("ph-browse__art");
  });
});

describe("now playing", () => {
  it("the cover's colour is a fifth at most, at the top and in the pool", () => {
    expect(NP_TOP_TINT).toBeLessThanOrEqual(0.2);
    const rule = phoneCss.match(/\n\.ph-np \{([^}]*)\}/)?.[1] ?? "";
    const shares = [...rule.matchAll(/var\(--ph-tint\) (\d+)%/g)].map((m) => Number(m[1]));
    expect(shares.length).toBeGreaterThan(2);
    for (const n of shares) expect(n).toBeLessThanOrEqual(20);
  });

  it("the expanded view on the computer: a pool of 20% at most, and no blurred art", () => {
    const pools = [...identityCss.matchAll(/\.fw--artwork \{([^}]*)\}/g)].map((m) => m[1]);
    expect(pools.length).toBeGreaterThan(0);
    for (const p of pools) for (const [, n] of p.matchAll(/var\(--fw-tint[^)]*\) (\d+)%/g)) expect(Number(n)).toBeLessThanOrEqual(20);
    expect(identityCss).toMatch(/\.fw--artwork :is\(\.fw__bg, \.fw__shade\) \{ display: none; \}/);
  });

  it("the miniplayer never takes the cover's colour", () => {
    const rule = identityCss.match(/\.mini\.is-tinted \{([^}]*)\}/)?.[1] ?? "";
    expect(rule).not.toContain("--mini-tint");
  });
});
