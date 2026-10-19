/** The Platter mark and the wordmark: sizes, the small-size drawing, accessible names, and one drawing shared with the app icon. */
import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { BOWL_AND_STEM, Logo, MARK_VIEWBOX, SPINDLE, Wordmark } from "../shell/Logo";
import appIcon from "../../src-tauri/app-icon.svg?raw";
import desktopIcon from "../../src-tauri/app-icon-desktop.svg?raw";
import lightIcon from "../../src-tauri/app-icon-light.svg?raw";

describe("Logo", () => {
  it("is decorative by default and drawn at the size asked", () => {
    const { container } = render(<Logo size={64} />);
    const svg = container.querySelector("svg")!;
    expect(svg.getAttribute("aria-hidden")).toBe("true");
    expect(svg.getAttribute("width")).toBe("64");
    expect(svg.getAttribute("height")).toBe("64");
    expect(svg.querySelector("[data-part=spindle]")).not.toBeNull();
  });

  it("leaves the spindle out below 20 px, where it would only fill the hole", () => {
    const { container } = render(<Logo size={16} />);
    expect(container.querySelector("[data-part=spindle]")).toBeNull();
    expect(container.querySelector("path")).not.toBeNull();
  });

  it("takes an accessible name when it stands for the app", () => {
    render(<Logo size={24} label="Platter" />);
    expect(screen.getByRole("img", { name: "Platter" })).toBeTruthy();
  });

  it("draws in the look's colours, not fixed ones", () => {
    const { container } = render(<Logo size={28} />);
    expect(container.querySelector("path")!.getAttribute("fill")).toBe("var(--color-accent)");
    expect(container.querySelector("circle")!.getAttribute("fill")).toBe("var(--color-text-primary)");
  });

  it("is the app icon's drawing, with the spindle in the bowl's centre", () => {
    for (const [file, icon] of Object.entries({ appIcon, desktopIcon, lightIcon })) {
      expect(icon, file).toContain(`d="${BOWL_AND_STEM}"`);
      expect(icon, file).toContain(`<circle cx="${SPINDLE.cx}" cy="${SPINDLE.cy}" r="${SPINDLE.r}"`);
    }
    // The bowl is the arc that starts at the stem's top: its centre is the spindle's.
    const [stemX, stemTop] = BOWL_AND_STEM.match(/^M(\d+) \d+V(\d+)A(\d+)/)!.slice(1, 3).map(Number);
    const radius = Number(BOWL_AND_STEM.match(/A(\d+) /)![1]);
    expect(stemTop).toBe(SPINDLE.cy);
    expect(stemX + radius).toBe(SPINDLE.cx);
  });

  it("sits in a square box the height of the mark, centred on it", () => {
    const [x, y, w, h] = MARK_VIEWBOX.split(" ").map(Number);
    const radius = Number(BOWL_AND_STEM.match(/A(\d+) /)![1]);
    const bottom = Number(BOWL_AND_STEM.match(/^M\d+ (\d+)/)![1]);
    const top = SPINDLE.cy - radius;
    expect(w).toBe(h);
    expect([y, y + h]).toEqual([top, bottom]);
    expect(x + w / 2).toBe(SPINDLE.cx);
  });
});

describe("Wordmark", () => {
  it("reads as one name, with the mark and the type hidden inside it", () => {
    render(<Wordmark />);
    const mark = screen.getByRole("img", { name: "Platter" });
    expect(mark.textContent).toBe("Platter");
    expect(mark.querySelector("svg")!.getAttribute("aria-hidden")).toBe("true");
  });
});
