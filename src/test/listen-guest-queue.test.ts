/**
 * Listening along as a guest: the queue here mirrors the host's, so edits
 * go to the host or say why they can't, instead of changing a local copy
 * the next update wipes.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../lib/backend";
import type { MockBackend } from "../lib/mock";
import { dropOnQueue } from "../stores/drag";
import { player } from "../stores/player";
import { NO_SESSION, useSocial } from "../stores/social";
import { useToasts } from "../stores/toasts";

const mock = backend as MockBackend;
const toasts = () => useToasts.getState().toasts.map((t) => t.message).join(" | ");

beforeEach(() => {
  useToasts.setState({ toasts: [] });
  useSocial.setState({ listen: { ...NO_SESSION, role: "guest", status: "live" } });
});
afterEach(() => {
  useSocial.setState({ listen: NO_SESSION });
  vi.restoreAllMocks();
});

describe("a guest's queue edits", () => {
  it("songs dropped on the queue go to the host", async () => {
    const add = vi.spyOn(backend, "listenAdd").mockResolvedValue(undefined);
    const local = vi.spyOn(backend, "queueAdd");
    const t = mock.fx.tracks[0];
    await dropOnQueue({ label: t.title, title: t.title, count: 1, tracks: [t], load: async () => [t] } as never, 0);
    expect(add).toHaveBeenCalledWith([t.id]);
    expect(local).not.toHaveBeenCalled();
    expect(toasts()).toContain("session's queue");
  });

  it("remove and clear say only the host can, and change nothing here", async () => {
    const remove = vi.spyOn(backend, "queueRemove");
    const clear = vi.spyOn(backend, "queueClear");
    await player.queueRemove("q1");
    await player.queueClear();
    expect(remove).not.toHaveBeenCalled();
    expect(clear).not.toHaveBeenCalled();
    expect(toasts()).toContain("Only the host can take songs out");
  });
});
