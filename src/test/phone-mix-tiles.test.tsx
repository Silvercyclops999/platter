/**
 * Home's Made for you tiles on the phone are the printed sleeves alone: the
 * name is on the paper, once, and the tile is named for the reader.
 */
import { render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { backend } from "../lib/backend";
import { DEFAULT_SETTINGS } from "../lib/types";
import { MixArt } from "../pages/MixCard";
import { usePhoneForm } from "../phone/form";
import { HomeMixes } from "../phone/pages/HomeExtras";
import { loadSettings } from "../stores/settings";

beforeEach(async () => {
  usePhoneForm.setState({ phone: true });
  await backend.settingsSet({ ...DEFAULT_SETTINGS, homeMixes: true });
  await loadSettings();
});

afterEach(() => {
  usePhoneForm.setState({ phone: false });
});

describe("Made for you on the phone", () => {
  it("each tile is its sleeve, naming the mix once", async () => {
    render(<HomeMixes />);
    const shelf = await screen.findByRole("region", { name: "Made for you" }, { timeout: 4000 });
    const tiles = await waitFor(() => {
      const t = [...shelf.querySelectorAll<HTMLElement>(".ph-mix-tile")];
      expect(t.length).toBeGreaterThan(0);
      return t;
    });
    for (const tile of tiles) {
      const name = tile.getAttribute("aria-label")!;
      expect(tile.querySelector(".ph-card-tile__title, .ph-card-tile__sub")).toBeNull();
      expect(within(tile).getAllByText(name, { exact: true })).toHaveLength(1);
    }
  });

  it("the sleeve carries the mix's name", () => {
    const mix = { id: "m1", kind: "daily", title: "Daily Mix 1", description: "", color: "#335", art: null, covers: [] } as never;
    const { container } = render(<MixArt mix={mix} />);
    expect(container.querySelector(".mix-art__title")?.textContent).toBe("Daily Mix 1");
  });
});
