import { describe, expect, it } from "vitest";
import { GLYPHS } from "../scripts/glyphs.mjs";
import { C, esc, keyPic, line, neoBar, svgOf, svgUrl, textWidth } from "../src/render.js";

describe("render", () => {
  it("escapes text", () => {
    expect(esc(`<a & "b">`)).toBe("&lt;a &amp; &quot;b&quot;&gt;");
    expect(svgOf(keyPic({ glyph: "play", label: "Rock & Roll" }))).toContain("Rock &amp; Roll");
  });

  it("short text stays plain; long text fades through a gradient (no masks)", () => {
    expect(line("Hi", 0, 10, { size: 14, max: 100 })).not.toContain("Gradient");
    const long = line("A very long song title indeed, longer than the room", 0, 10, { size: 14, max: 100 });
    expect(long).toContain("linearGradient");
    expect(long).toContain('stop-opacity="0"');
    expect(long).not.toContain("<mask");
    expect(long).not.toContain("clipPath");
    expect(textWidth("WWW", 10)).toBeGreaterThan(textWidth("iii", 10));
    // The same picture is the same text, so keys aren't sent it again.
    expect(line("A very long song title indeed, longer than the room", 0, 10, { size: 14, max: 100 })).toBe(long);
  });

  it("pictures are SVG data at 144 (keys) and 232 x 50 (Neo)", () => {
    expect(svgUrl("").startsWith("data:image/svg+xml;charset=utf8,")).toBe(true);
    expect(svgOf(svgUrl(""))).toContain('viewBox="0 0 144 144"');
    expect(svgOf(neoBar({ cover: null, title: "T", artist: "A", progress: 0.5, paused: false }))).toContain('viewBox="0 0 232 50"');
  });

  it("on keys use the one accent; greyed keys the faint colour", () => {
    expect(svgOf(keyPic({ glyph: "shuffle", on: true }))).toContain(C.accent);
    expect(svgOf(keyPic({ glyph: "shuffle", off: true }))).toContain(C.faint);
  });

  it("every glyph draws", () => {
    for (const [name, g] of Object.entries(GLYPHS)) expect(g("#fff").length, name).toBeGreaterThan(20);
  });
});
