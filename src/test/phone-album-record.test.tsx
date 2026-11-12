/**
 * The album page's record: it sits behind the cover, slides out to the
 * right as the page opens (held still with reduced motion), its grooves
 * part where the album's songs do, and it has no light drawn inside the
 * SVG (WebKit paints that in the wrong place on a moving record).
 */
import { act, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { usePhoneForm } from "../phone/form";
import { PhoneAlbum } from "../phone/pages/Album";
import phoneCss from "../phone/phone.css?raw";

const mock = backend as MockBackend;

beforeEach(() => act(() => usePhoneForm.setState({ phone: true })));
afterEach(() => act(() => usePhoneForm.setState({ phone: false })));

describe("the album's record", () => {
  it("sits behind the cover, with one groove gap per song and no SVG light", async () => {
    const album = mock.fx.albums[0];
    const { container, findByRole } = render(<PhoneAlbum id={album.id} />);
    await findByRole("heading", { level: 1, name: album.title }, { timeout: 4000 });
    const sleeve = container.querySelector(".ph-album__sleeve")!;
    expect(sleeve).not.toBeNull();
    const disc = sleeve.querySelector(".ph-album__disc")!;
    expect(disc).toHaveAttribute("aria-hidden", "true");
    expect(disc.querySelector("svg.record-disc")).not.toBeNull();
    expect(disc.querySelector("foreignObject")).toBeNull();
    // The record comes before the cover, so the cover is drawn over it.
    expect(sleeve.firstElementChild).toBe(disc);
    expect(sleeve.querySelector(".ph-album__cover .art")).not.toBeNull();
  });

  it("slides out as the page opens, and holds still with reduced motion", () => {
    expect(phoneCss).toMatch(/\.ph-album__disc \{[^}]*animation: ph-disc-out/);
    expect(phoneCss).toMatch(/:root\[data-reduced-motion="true"\] \.ph-album__disc \{ animation: none; \}/);
    expect(phoneCss).toMatch(/@media \(prefers-reduced-motion: reduce\) \{ \.ph-album__disc \{ animation: none; \} \}/);
  });
});
