/**
 * Song info for streaming overlays: the clipboard, or a text file
 * rewritten whenever the song changes.
 */
import { spawn } from "node:child_process";
import { writeFile } from "node:fs/promises";

export type Writer = { file(path: string, text: string): Promise<void>; clipboard(text: string): Promise<void> };

/** The real writer: the file as UTF-8, the clipboard through the OS's own tool. */
export const osWriter: Writer = {
  file: (path, text) => writeFile(path, text, "utf8"),
  clipboard: (text) =>
    new Promise((resolve, reject) => {
      const p = process.platform === "win32" ? spawn("clip") : spawn("pbcopy");
      p.on("error", reject);
      p.on("close", () => resolve());
      p.stdin.end(process.platform === "win32" ? `${text}` : text, "utf8");
    }),
};

/** Writes the text when it changes (one per output target). */
export class InfoOutput {
  private last = new Map<string, string>();

  constructor(private readonly w: Writer = osWriter) {}

  async put(target: "clipboard" | "file", path: string, text: string): Promise<boolean> {
    const key = `${target}:${path}`;
    if (this.last.get(key) === text) return false;
    if (target === "file" && !path.trim()) return false;
    this.last.set(key, text);
    if (target === "file") await this.w.file(path, text);
    else await this.w.clipboard(text);
    return true;
  }

  /** Write again next time even when unchanged (a key just appeared). */
  forget(): void {
    this.last.clear();
  }
}
