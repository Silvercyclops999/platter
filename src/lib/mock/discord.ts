/**
 * Discord status for the mock backend. Nothing here talks to Discord or
 * Imgur. Demo with ?discord=off (a build without a Discord app id) or
 * ?discord=closed (Discord isn't running); ?discord=uploadFailed shows a
 * failed cover upload.
 */
import type { DiscordStatus, InfoKeyState } from "../types";

export type DiscordDemo = "off" | "closed" | "uploadFailed";

export function isDiscordDemo(v: string | null): v is DiscordDemo {
  return v === "off" || v === "closed" || v === "uploadFailed";
}

export class MockDiscord {
  private available: boolean;
  private connected: boolean;
  private uploadFailed: boolean;
  private key: string | null = null;
  private rejected = false;

  constructor(
    demo: DiscordDemo | undefined,
    private readonly onChange: (s: DiscordStatus) => void,
  ) {
    this.available = demo !== "off";
    this.connected = demo !== "closed" && demo !== "off";
    this.uploadFailed = demo === "uploadFailed";
    if (demo === "uploadFailed") this.key = "demo";
  }

  status(): DiscordStatus {
    return {
      available: this.available,
      connected: this.connected,
      imgurKey: { saved: !!this.key, rejected: this.rejected },
      uploadFailed: this.uploadFailed,
    };
  }

  /** Keys starting with "bad" are refused, like a wrong client ID. */
  setKey(key: string | null): InfoKeyState {
    this.key = key?.trim() || null;
    this.rejected = !!this.key && this.key.startsWith("bad");
    this.uploadFailed = false;
    this.onChange(this.status());
    return { saved: !!this.key, rejected: this.rejected };
  }

  // Test hooks.
  setConnected(on: boolean) {
    this.connected = on;
    this.onChange(this.status());
  }
  setAvailable(on: boolean) {
    this.available = on;
    if (!on) this.connected = false;
    this.onChange(this.status());
  }
  failUpload() {
    this.uploadFailed = true;
    this.onChange(this.status());
  }
}
