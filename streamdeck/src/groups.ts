/**
 * Big art: Play / Pause keys side by side share one large cover. A key
 * either says where it sits (size, column, row) or finds its neighbours.
 */
import type { Slice } from "./render.js";

export interface Spot {
  column: number;
  row: number;
}

/**
 * The square block of keys (4, then 3, then 2 a side) this key belongs to,
 * all of them Play / Pause keys on the same device. Of the blocks that fit,
 * the one starting highest, then leftmost, so neighbours agree.
 */
export function autoSlice(me: Spot, all: Spot[]): Slice | null {
  const has = new Set(all.map((s) => `${s.column},${s.row}`));
  has.add(`${me.column},${me.row}`);
  for (const n of [4, 3, 2]) {
    for (let r0 = me.row - n + 1; r0 <= me.row; r0++) {
      for (let c0 = me.column - n + 1; c0 <= me.column; c0++) {
        if (r0 < 0 || c0 < 0) continue;
        let full = true;
        for (let r = r0; r < r0 + n && full; r++) for (let c = c0; c < c0 + n && full; c++) full = has.has(`${c},${r}`);
        if (full) return { size: n, col: me.column - c0, row: me.row - r0 };
      }
    }
  }
  return null;
}

/** A key's slice from its settings: manual position, auto, or none. */
export function sliceFor(st: { group: "off" | "auto" | "manual"; groupSize: number; groupCol: number; groupRow: number }, me: Spot | null, all: Spot[]): Slice | null {
  if (st.group === "manual") {
    const n = st.groupSize;
    return { size: n, col: Math.min(n, st.groupCol) - 1, row: Math.min(n, st.groupRow) - 1 };
  }
  if (st.group === "auto" && me) return autoSlice(me, all);
  return null;
}
