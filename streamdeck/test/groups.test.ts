import { describe, expect, it } from "vitest";
import { autoSlice, sliceFor } from "../src/groups.js";

const grid = (cols: number, rows: number, c0 = 0, r0 = 0) => Array.from({ length: cols * rows }, (_, i) => ({ column: c0 + (i % cols), row: r0 + Math.floor(i / cols) }));

describe("big art groups", () => {
  it("finds a 2 x 2 block and each key's place in it", () => {
    const keys = grid(2, 2, 1, 0);
    expect(autoSlice({ column: 1, row: 0 }, keys)).toEqual({ size: 2, col: 0, row: 0 });
    expect(autoSlice({ column: 2, row: 1 }, keys)).toEqual({ size: 2, col: 1, row: 1 });
  });

  it("prefers the biggest square, 3 x 3 on a 5 x 3 deck", () => {
    const keys = grid(3, 3);
    expect(autoSlice({ column: 1, row: 1 }, keys)).toEqual({ size: 3, col: 1, row: 1 });
    expect(autoSlice({ column: 2, row: 2 }, keys)).toEqual({ size: 3, col: 2, row: 2 });
  });

  it("a key on its own, or in a row, has no group", () => {
    expect(autoSlice({ column: 0, row: 0 }, [])).toBeNull();
    expect(autoSlice({ column: 0, row: 0 }, grid(4, 1))).toBeNull();
  });

  it("neighbours agree on one block (top-left first)", () => {
    const keys = grid(3, 2);
    expect(autoSlice({ column: 1, row: 0 }, keys)).toEqual({ size: 2, col: 1, row: 0 });
    expect(autoSlice({ column: 0, row: 0 }, keys)).toEqual({ size: 2, col: 0, row: 0 });
  });

  it("settings: off, by hand or found", () => {
    const st = { group: "manual" as const, groupSize: 3, groupCol: 2, groupRow: 3 };
    expect(sliceFor(st, null, [])).toEqual({ size: 3, col: 1, row: 2 });
    expect(sliceFor({ ...st, group: "off" }, { column: 0, row: 0 }, grid(2, 2))).toBeNull();
    expect(sliceFor({ ...st, group: "auto" }, { column: 0, row: 0 }, grid(2, 2))).toEqual({ size: 2, col: 0, row: 0 });
  });
});
