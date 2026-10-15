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
