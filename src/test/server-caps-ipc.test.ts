import { describe, expect, it, vi } from "vitest";

const invoke = vi.fn(async (..._args: unknown[]) => undefined as unknown);
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...a: unknown[]) => invoke(...a),
  convertFileSrc: (p: string) => p,
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));

const { createTauriBackend } = await import("../lib/backend");

describe("backend_caps", () => {
  it("is one command with no arguments", async () => {
    await createTauriBackend().backendCaps();
    expect(invoke).toHaveBeenCalledWith("backend_caps", undefined);
  });
});
