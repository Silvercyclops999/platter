import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { toast, toastExpiry, useToasts } from "../stores/toasts";

const toasts = () => useToasts.getState().toasts;

describe("toasts", () => {
  beforeEach(() => {
    toastExpiry.on = true;
    vi.useFakeTimers();
    useToasts.setState({ toasts: [] });
  });
  afterEach(() => {
    toastExpiry.on = false;
    vi.useRealTimers();
  });

  it("shows the same text once however often it's pushed", () => {
    const a = toast.info("No unreleased songs are waiting for lyrics.");
    const b = toast.info("No unreleased songs are waiting for lyrics.");
    const c = toast.info("No unreleased songs are waiting for lyrics.");
    expect(toasts()).toHaveLength(1);
    expect(b).toBe(a);
    expect(c).toBe(a);
  });

  it("a repeat restarts the timer", () => {
    toast.info("Copied to clipboard");
    vi.advanceTimersByTime(3000);
    toast.info("Copied to clipboard");
    vi.advanceTimersByTime(3000);
    expect(toasts()).toHaveLength(1);
    vi.advanceTimersByTime(1001);
    expect(toasts()).toHaveLength(0);
  });

  it("a repeat keeps its place and takes the new kind", () => {
    toast.info("One");
    toast.info("Two");
    toast.error("One");
    expect(toasts().map((t) => [t.message, t.kind])).toEqual([
      ["One", "error"],
      ["Two", "info"],
    ]);
  });

  it("different texts still stack, three at most", () => {
    for (const m of ["A", "B", "C", "D"]) toast.info(m);
    expect(toasts().map((t) => t.message)).toEqual(["B", "C", "D"]);
  });

  it("dismissing one stops its timer from touching a later toast", () => {
    const id = toast.info("Gone");
    useToasts.getState().dismiss(id);
    toast.info("Gone");
    expect(toasts()).toHaveLength(1);
    vi.advanceTimersByTime(3999);
    expect(toasts()).toHaveLength(1);
  });
});

describe("failure wording", () => {
  it("names what failed, adds the reason, and cuts a long reason short", async () => {
    const { failText, TOAST_REASON_MAX } = await import("../lib/errorText");
    expect(failText("Couldn't save the playlist", { message: "Offline" })).toBe("Couldn't save the playlist: Offline");
    expect(failText("Couldn't save the playlist", {})).toBe("Couldn't save the playlist");
    const long = failText("Couldn't save the playlist", { message: "word ".repeat(60) });
    expect(long.length).toBeLessThanOrEqual("Couldn't save the playlist: ".length + TOAST_REASON_MAX);
    expect(long.endsWith("…")).toBe(true);
  });

  it("an Undo toast carries the Undo button", async () => {
    const { toast, useToasts } = await import("../stores/toasts");
    let undone = false;
    toast.undo("Deleted Road Trip", () => (undone = true));
    const t = useToasts.getState().toasts.find((x) => x.message === "Deleted Road Trip")!;
    expect(t.action?.label).toBe("Undo");
    t.action!.run();
    expect(undone).toBe(true);
    useToasts.getState().dismiss(t.id);
  });
});
