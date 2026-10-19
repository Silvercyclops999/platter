/** The Platter mark and the wordmark: sizes, the small-size drawing and accessible names. */
import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { Logo, Wordmark } from "../shell/Logo";

describe("Logo", () => {
  it("is decorative by default and drawn at the size asked", () => {
    const { container } = render(<Logo size={64} />);
    const svg = container.querySelector("svg")!;
    expect(svg.getAttribute("aria-hidden")).toBe("true");
    expect(svg.getAttribute("width")).toBe("64");
    expect(svg.getAttribute("height")).toBe("64");
    expect(svg.querySelector("[data-part=spindle]")).not.toBeNull();
  });

  it("leaves the spindle out below 20 px, where it would only blur", () => {
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
});

describe("Wordmark", () => {
  it("reads as one name, with the mark and the type hidden inside it", () => {
    render(<Wordmark />);
    const mark = screen.getByRole("img", { name: "Platter" });
    expect(mark.textContent).toBe("Platter");
    expect(mark.querySelector("svg")!.getAttribute("aria-hidden")).toBe("true");
  });
});
