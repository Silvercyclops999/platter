import { act, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Dialog } from "../components/Dialog";
import { TrackList, TrackRow } from "../components/TrackRow";
import { resetTypeAhead } from "../components/listKeys";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { DEFAULT_SETTINGS } from "../lib/types";
import { PowerHints } from "../shell/PowerHints";
import { ShortcutsHelp } from "../shell/ShortcutsHelp";
import { collectHints, hintLabels, overlaps, placeHint, resetPowerKeys, usePowerHints, type Box } from "../shell/powerKeys";
import { useShortcuts } from "../shell/useShortcuts";
import { useSettings } from "../stores/settings";
import { useUi } from "../stores/ui";
import { body } from "./rows";

const mock = backend as MockBackend;

function Keys() {
  useShortcuts();
  return null;
}

const VIEW = { width: 1000, height: 700 };

/** Lay elements out from data-box="left,top,width,height" (jsdom has no layout). */
function fakeLayout() {
  return vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
    const b = (this.closest("[data-box]") === this ? this.dataset.box : undefined)?.split(",").map(Number);
    const [left, top, width, height] = b ?? [0, 0, 0, 0];
    return { left, top, width, height, right: left + width, bottom: top + height, x: left, y: top, toJSON: () => ({}) } as DOMRect;
  });
}

function power(on: boolean, hintKey: "f" | ";" = "f") {
  act(() => useSettings.setState({ settings: { ...DEFAULT_SETTINGS, powerKeys: on, hintKey } }));
}

beforeEach(() => {
  resetPowerKeys();
  useUi.setState({ paletteOpen: false, menu: null, shortcutsOpen: false });
  Object.defineProperty(window, "innerWidth", { value: VIEW.width, configurable: true });
  Object.defineProperty(window, "innerHeight", { value: VIEW.height, configurable: true });
});
afterEach(() => {
  vi.restoreAllMocks();
  resetTypeAhead();
  resetPowerKeys();
});

describe("hint labels and placement", () => {
  it("gives distinct two-letter labels, home row first", () => {
    const l = hintLabels(40);
    expect(l).toHaveLength(40);
    expect(new Set(l).size).toBe(40);
    expect(l.every((x) => /^[a-z]{2}$/.test(x))).toBe(true);
    expect(l.slice(0, 3)).toEqual(["aa", "as", "ad"]);
    expect(hintLabels(10_000)).toHaveLength(26 * 26);
  });

  it("never puts a label on the thing it names", () => {
    const size = { width: 26, height: 18 };
    const targets: Box[] = [
      { left: 300, top: 300, width: 120, height: 32 }, // room on the left
      { left: 0, top: 300, width: 400, height: 40 }, // at the left edge: above
      { left: 0, top: 0, width: 300, height: 40 }, // top-left corner: below
      { left: 0, top: 0, width: 960, height: 700 }, // full height: to the right
    ];
    for (const t of targets) {
      const p = placeHint(t, size, VIEW)!;
      expect(overlaps({ ...p, ...size }, t), JSON.stringify(t)).toBe(false);
    }
    // Filling the window: no room for a label anywhere, so no hint.
    expect(placeHint({ left: 0, top: 0, width: 1000, height: 700 }, size, VIEW)).toBeNull();
    expect(placeHint(targets[0], size, VIEW)).toEqual({ left: 300 - 26 - 2, top: 307 });
    expect(placeHint(targets[1], size, VIEW)!.top).toBe(300 - 18 - 2);
    expect(placeHint(targets[2], size, VIEW)!.top).toBe(42);
    expect(placeHint(targets[3], size, VIEW)!.left).toBe(962);
  });

  it("labels only what's in view, and only the open dialog's controls", () => {
    fakeLayout();
    render(
      <div>
        <button data-box="100,100,80,30">In view</button>
        <button data-box="100,900,80,30">Below the window</button>
        <button data-box="0,0,0,0">Hidden</button>
        <button data-box="100,200,80,30" disabled>
          Off
        </button>
        <div aria-hidden="true">
          <button data-box="100,300,80,30">Behind</button>
        </div>
      </div>,
    );
    expect(collectHints().map((h) => h.el.textContent)).toEqual(["In view"]);
    render(
      <div role="dialog" aria-modal="true">
        <button data-box="300,300,80,30">In the dialog</button>
      </div>,
    );
    expect(collectHints().map((h) => h.el.textContent)).toEqual(["In the dialog"]);
  });
});

describe("jump hints", () => {
  function Page({ onA, onB }: { onA: () => void; onB: () => void }) {
    return (
      <>
        <Keys />
        <PowerHints />
        <button data-box="100,100,80,30" onClick={onA}>
          First
        </button>
        <button data-box="100,200,80,30" onClick={onB}>
          Second
        </button>
        <input data-box="100,300,200,30" aria-label="Field" />
      </>
    );
  }

  it("the hint key shows them; typing a label clicks that thing", () => {
    fakeLayout();
    power(true);
    const [onA, onB] = [vi.fn(), vi.fn()];
    render(<Page onA={onA} onB={onB} />);
    fireEvent.keyDown(window, { key: "f" });
    const shown = screen.getByTestId("power-hints").querySelectorAll("[data-hint]");
    expect([...shown].map((h) => h.getAttribute("data-hint"))).toEqual(["aa", "as", "ad"]);
    fireEvent.keyDown(window, { key: "a" });
    // Narrowed but still all three (they all start with a).
    expect(usePowerHints.getState().typed).toBe("a");
    fireEvent.keyDown(window, { key: "s" });
    expect(onB).toHaveBeenCalledTimes(1);
    expect(onA).not.toHaveBeenCalled();
    expect(screen.queryByTestId("power-hints")).toBeNull();
  });

  it("a field's hint focuses it; Esc and a wrong letter hide them", () => {
    fakeLayout();
    power(true);
    render(<Page onA={vi.fn()} onB={vi.fn()} />);
    fireEvent.keyDown(window, { key: "f" });
    fireEvent.keyDown(window, { key: "a" });
    fireEvent.keyDown(window, { key: "d" });
    expect(screen.getByRole("textbox", { name: "Field" })).toHaveFocus();
    (document.activeElement as HTMLElement).blur();
    fireEvent.keyDown(window, { key: "f" });
    expect(screen.getByTestId("power-hints")).toBeInTheDocument();
    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByTestId("power-hints")).toBeNull();
    fireEvent.keyDown(window, { key: "f" });
    fireEvent.keyDown(window, { key: "z" });
    expect(screen.queryByTestId("power-hints")).toBeNull();
  });

  it("follows the chosen hint key, and does nothing while power mode is off", () => {
    fakeLayout();
    power(false);
    render(<Page onA={vi.fn()} onB={vi.fn()} />);
    fireEvent.keyDown(window, { key: "f" });
    expect(screen.queryByTestId("power-hints")).toBeNull();
    power(true, ";");
    fireEvent.keyDown(window, { key: "f" });
    expect(screen.queryByTestId("power-hints")).toBeNull();
    fireEvent.keyDown(window, { key: ";" });
    expect(screen.getByTestId("power-hints")).toBeInTheDocument();
  });

  it("leaves typing in a field alone", () => {
    fakeLayout();
    power(true);
    render(<Page onA={vi.fn()} onB={vi.fn()} />);
    const field = screen.getByRole("textbox", { name: "Field" });
    field.focus();
    fireEvent.keyDown(field, { key: "f" });
    expect(screen.queryByTestId("power-hints")).toBeNull();
  });
});

describe("list keys", () => {
  const [a, b, c] = mock.fx.tracks;
  function List({ onPlay }: { onPlay: (id: string) => void }) {
    return (
      <>
        <Keys />
        <TrackList label="Tracks">
          {[a, b, c].map((t, i) => (
            <TrackRow key={t.id} track={t} index={i + 1} onPlay={() => onPlay(t.id)} />
          ))}
        </TrackList>
      </>
    );
  }
  const rows = () => body(screen.getAllByRole("row"));

  it("j and k move the selection, g g and G jump to the ends, Enter plays", async () => {
    power(true);
    const user = userEvent.setup();
    const onPlay = vi.fn();
    render(<List onPlay={onPlay} />);
    // Nothing focused: j starts at the first row.
    await user.keyboard("j");
    expect(rows()[0]).toHaveFocus();
    expect(rows()[0]).toHaveAttribute("data-selected");
    await user.keyboard("j");
    expect(rows()[1]).toHaveFocus();
    expect(rows()[1]).toHaveAttribute("data-selected");
    await user.keyboard("k");
    expect(rows()[0]).toHaveFocus();
    await user.keyboard("G");
    expect(rows()[2]).toHaveFocus();
    expect(rows()[2]).toHaveAttribute("data-selected");
    await user.keyboard("gg");
    expect(rows()[0]).toHaveFocus();
    await user.keyboard("{Enter}");
    expect(onPlay).toHaveBeenCalledWith(a.id);
  });

  it("with power mode off, j is type-ahead as before", async () => {
    power(false);
    const user = userEvent.setup();
    render(<List onPlay={vi.fn()} />);
    rows()[0].focus();
    await user.keyboard("j");
    // No title starts with j in the fixtures' first three, so focus stays.
    const jTitle = [b, c].find((t) => t.title.toLowerCase().startsWith("j"));
    expect(document.activeElement).toBe(jTitle ? rows()[[a, b, c].indexOf(jTitle)] : rows()[0]);
  });

  it("leave the page behind a modal dialog alone", () => {
    power(true);
    const onPlay = vi.fn();
    render(
      <>
        <List onPlay={onPlay} />
        <Dialog open onClose={() => {}} label="Rename">
          <input aria-label="Name" />
        </Dialog>
      </>,
    );
    const tick = useUi.getState().searchFocusTick;
    fireEvent.keyDown(document.body, { key: "j" });
    fireEvent.keyDown(document.body, { key: "/" });
    expect(rows().some((r) => r.hasAttribute("data-selected"))).toBe(false);
    expect(useUi.getState().searchFocusTick).toBe(tick);
  });

  it("/ focuses search", () => {
    power(true);
    render(<Keys />);
    const tick = useUi.getState().searchFocusTick;
    fireEvent.keyDown(window, { key: "/" });
    expect(useUi.getState().searchFocusTick).toBe(tick + 1);
  });
});

describe("shortcuts window", () => {
  it("lists the power mode keys with the chosen hint key", () => {
    power(false, ";");
    act(() => useUi.setState({ shortcutsOpen: true }));
    render(<ShortcutsHelp />);
    const dialog = screen.getByRole("dialog", { name: "Keyboard shortcuts" });
    expect(dialog).toHaveTextContent("Turn on Keyboard power mode in Settings to use these.");
    const hints = dialog.querySelector('[data-shortcut="powerHints"]')!;
    expect([...hints.querySelectorAll("kbd")].map((k) => k.textContent)).toEqual([";"]);
    const top = dialog.querySelector('[data-shortcut="powerTop"]')!;
    expect([...top.querySelectorAll("kbd")].map((k) => k.textContent)).toEqual(["G", "G"]);
  });
});
