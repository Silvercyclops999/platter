import { describe, expect, it } from "vitest";
import { offlineLeftOutMessage, offlineSkipped } from "../lib/offline";

describe("offline notices name the device they're on", () => {
  it("says this computer on a computer and this phone on a phone", () => {
    expect(offlineSkipped()).toContain("aren't on this computer.");
    expect(offlineSkipped(true)).toContain("aren't on this phone.");
    expect(offlineSkipped(true)).not.toContain("computer");
    expect(offlineLeftOutMessage(1, true)).toBe("Skipped 1 song that isn't on this phone. They play again when Platter can reach your server.");
  });
});
