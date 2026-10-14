import { fireEvent, render } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { useShortcuts } from "../shell/useShortcuts";
import { nav, useRouter } from "../stores/router";
import { useUi } from "../stores/ui";

const route = () => useRouter.getState().entries[useRouter.getState().index].route;

describe("router", () => {
  beforeEach(() => useRouter.getState().reset());

  it("navigates, goes back and forward", () => {
    nav.go({ name: "album", id: "1" });
    nav.go({ name: "artist", id: "2" });
    expect(route()).toEqual({ name: "artist", id: "2" });
    nav.back();
    expect(route()).toEqual({ name: "album", id: "1" });
    nav.back();
    expect(route()).toEqual({ name: "home" });
    nav.back(); // no-op at the start
    expect(route()).toEqual({ name: "home" });
    nav.forward();
    nav.forward();
    expect(route()).toEqual({ name: "artist", id: "2" });
    nav.forward(); // no-op at the end
    expect(route()).toEqual({ name: "artist", id: "2" });
  });

  it("drops forward history when navigating from the middle", () => {
    nav.go({ name: "album", id: "1" });
    nav.go({ name: "album", id: "2" });
    nav.back();
    nav.go({ name: "library", tab: "artists" });
    expect(useRouter.getState().entries.map((e) => e.route.name)).toEqual(["home", "album", "library"]);
    nav.forward();
    expect(route()).toEqual({ name: "library", tab: "artists" });
  });

  it("does not stack duplicates of the current route", () => {
    nav.go({ name: "album", id: "1" });
    nav.go({ name: "album", id: "1" });
    expect(useRouter.getState().entries).toHaveLength(2);
  });

  it("replace() edits the current entry without adding history", () => {
    nav.go({ name: "search" });
    useRouter.getState().replace({ name: "search", query: "lan" });
    expect(useRouter.getState().entries).toHaveLength(2);
    expect(route()).toEqual({ name: "search", query: "lan" });
  });
});

function ShortcutHost() {
  useShortcuts();
  return null;
}

describe("navigation shortcuts", () => {
  beforeEach(() => {
    useRouter.getState().reset();
    useUi.setState({ paletteOpen: false, searchFocusTick: 0 });
  });

  it("Alt+Left / Alt+Right go back and forward", () => {
    render(<ShortcutHost />);
    nav.go({ name: "album", id: "1" });
    fireEvent.keyDown(window, { key: "ArrowLeft", altKey: true });
    expect(route()).toEqual({ name: "home" });
    fireEvent.keyDown(window, { key: "ArrowRight", altKey: true });
    expect(route()).toEqual({ name: "album", id: "1" });
  });

  it("mouse back/forward buttons navigate", () => {
    render(<ShortcutHost />);
    nav.go({ name: "album", id: "1" });
    fireEvent.mouseUp(window, { button: 3 });
    expect(route()).toEqual({ name: "home" });
    fireEvent.mouseUp(window, { button: 4 });
    expect(route()).toEqual({ name: "album", id: "1" });
  });

  it("Ctrl+K toggles the command palette and Ctrl+L asks for search focus", () => {
    render(<ShortcutHost />);
    fireEvent.keyDown(window, { key: "k", ctrlKey: true, metaKey: true });
    expect(useUi.getState().paletteOpen).toBe(true);
    fireEvent.keyDown(window, { key: "l", ctrlKey: true, metaKey: true });
    expect(useUi.getState().paletteOpen).toBe(false);
    expect(useUi.getState().searchFocusTick).toBe(1);
  });
});
