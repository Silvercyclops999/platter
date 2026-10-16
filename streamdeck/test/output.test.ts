import { describe, expect, it } from "vitest";
import { InfoOutput } from "../src/output.js";

describe("song info output", () => {
  it("writes when the text changes, to the file or the clipboard", async () => {
    const wrote: string[] = [];
    const o = new InfoOutput({ file: async (p, t) => void wrote.push(`file ${p} ${t}`), clipboard: async (t) => void wrote.push(`clip ${t}`) });
    expect(await o.put("file", "/tmp/now.txt", "A - B")).toBe(true);
    expect(await o.put("file", "/tmp/now.txt", "A - B")).toBe(false);
    expect(await o.put("file", "/tmp/now.txt", "C - D")).toBe(true);
    expect(await o.put("clipboard", "", "C - D")).toBe(true);
    expect(await o.put("file", " ", "x"), "no path, no file").toBe(false);
    o.forget();
    expect(await o.put("file", "/tmp/now.txt", "C - D")).toBe(true);
    expect(wrote).toEqual(["file /tmp/now.txt A - B", "file /tmp/now.txt C - D", "clip C - D", "file /tmp/now.txt C - D"]);
  });
});
