/**
 * Small things that make the phone feel like a phone app: dragging search
 * results puts the keyboard away, a cover's colour runs up behind the clock
 * until the page scrolls, the search bar shows focus without a web form's
 * ring, and a long playlist opens at once.
 */
import { render } from "@testing-library/react";
import { useRef } from "react";
import { afterEach, describe, expect, it } from "vitest";
import { useKeyboardAwayOnDrag, KEYBOARD_DRAG_PX } from "../phone/gestures";
import phoneCss from "../phone/phone.css?raw";

afterEach(() => {
  document.body.innerHTML = "";
});

function touch(type: string, x: number, y: number, target: EventTarget) {
  const e = new Event(type, { bubbles: true, cancelable: true });
  Object.defineProperty(e, "touches", { value: type === "touchend" ? [] : [{ clientX: x, clientY: y }] });
  target.dispatchEvent(e);
}

function Field({ on }: { on: boolean }) {
  const ref = useRef<HTMLInputElement>(null);
  useKeyboardAwayOnDrag(on, ref);
  return (
    <div>
      <input ref={ref} aria-label="Search" />
      <ul data-testid="list">
        <li>row</li>
      </ul>
    </div>
  );
}

describe("keyboard away on drag", () => {
  it("puts the keyboard away once the list is dragged up or down", () => {
    const { getByLabelText, getByTestId } = render(<Field on />);
    const input = getByLabelText("Search") as HTMLInputElement;
    input.focus();
    const list = getByTestId("list");
    touch("touchstart", 100, 400, list);
    touch("touchmove", 100, 400 - (KEYBOARD_DRAG_PX - 2), list);
    expect(document.activeElement).toBe(input);
    touch("touchmove", 102, 400 - KEYBOARD_DRAG_PX - 4, list);
    expect(document.activeElement).not.toBe(input);
  });

  it("keeps it up for a sideways drag (the chips) or a drag that starts in the field", () => {
    const { getByLabelText, getByTestId } = render(<Field on />);
    const input = getByLabelText("Search") as HTMLInputElement;
    input.focus();
    const list = getByTestId("list");
    touch("touchstart", 100, 400, list);
    touch("touchmove", 160, 410, list);
    expect(document.activeElement).toBe(input);
    touch("touchstart", 100, 400, input);
    touch("touchmove", 100, 340, input);
    expect(document.activeElement).toBe(input);
  });

  it("listens only while the field is focused", () => {
    const { getByLabelText, getByTestId } = render(<Field on={false} />);
    const input = getByLabelText("Search") as HTMLInputElement;
    input.focus();
    const list = getByTestId("list");
    touch("touchstart", 100, 400, list);
    touch("touchmove", 100, 300, list);
    expect(document.activeElement).toBe(input);
  });
});

describe("phone.css", () => {
  it("lets the page show behind the clock until it scrolls", () => {
    expect(phoneCss).toMatch(/\.ph-stack:not\(\[data-moving\]\):has\(\.ph-layer:not\(\[aria-hidden\]\) \.ph-page:not\(\.is-scrolled\) > \.ph-backbar\)::after \{ opacity: 0; \}/);
  });

  it("gives the search bar no focus ring", () => {
    expect(phoneCss).toMatch(/\.ph-search__bar \.ph-field:focus-within \{ box-shadow: none; \}/);
  });
});

describe("long lists", () => {
  it("a playlist of hundreds of songs draws only the rows near the screen", async () => {
    const { act, screen } = await import("@testing-library/react");
    const { vi } = await import("vitest");
    const { backend } = await import("../lib/backend");
    const { usePhoneForm } = await import("../phone/form");
    const { PhonePlaylist } = await import("../phone/pages/Playlist");
    act(() => usePhoneForm.setState({ phone: true }));
    const real = await backend.playlistPage("500");
    const base = real.tracks[0];
    const tracks = Array.from({ length: 1200 }, (_, i) => ({ ...base, id: `big-${i}`, title: `Song ${i}` }));
    vi.spyOn(backend, "playlistPage").mockResolvedValue({ ...real, tracks, itemIds: tracks.map((_, i) => `item-${i}`) });
    render(
      <div className="ph-page" style={{ height: 800, overflow: "auto" }}>
        <PhonePlaylist id="500" />
      </div>,
    );
    expect(await screen.findByText("Song 0", undefined, { timeout: 6000 })).toBeInTheDocument();
    const drawn = document.querySelectorAll(".ph-songs .ph-song").length;
    expect(drawn).toBeGreaterThan(5);
    expect(drawn).toBeLessThan(100);
    expect(screen.getByText(/1,200 songs/)).toBeInTheDocument();
    vi.restoreAllMocks();
    act(() => usePhoneForm.setState({ phone: false }));
  });
});
