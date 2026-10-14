import { fireEvent, render } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Art } from "../components/Art";
import { Shelf, ShelfItem } from "../components/Shelf";
import { nextArtRange, rangeWithin, type ShelfMetrics } from "../lib/shelfWindow";

// 12 cards of 200px in a 1000px row: five on screen at a time.
const m = (scrollLeft: number, count = 12): ShelfMetrics => ({ scrollLeft, viewport: 1000, itemWidth: 200, count });

describe("shelf art window", () => {
  it("covers the visible cards plus the load margin", () => {
    expect(rangeWithin(m(0), 0)).toEqual({ start: 0, end: 5 });
    expect(rangeWithin(m(100), 0)).toEqual({ start: 0, end: 6 });
    // Half a viewport (500px) each side, clipped to the row.
    expect(nextArtRange(null, m(0))).toEqual({ start: 0, end: 8 });
    expect(nextArtRange(null, m(1400))).toEqual({ start: 4, end: 12 });
  });

  it("gives every card art when there is no layout to measure", () => {
    expect(rangeWithin({ scrollLeft: 0, viewport: 0, itemWidth: 0, count: 7 }, 0)).toEqual({ start: 0, end: 7 });
    expect(nextArtRange(null, m(0, 0))).toEqual({ start: 0, end: 0 });
  });

  it("keeps art that is still within the keep margin", () => {
    const first = nextArtRange(null, m(0));
    // Scrolled one viewport on: cards 0-2 are within 1.5 viewports, so stay.
    expect(nextArtRange(first, m(1000))).toEqual({ start: 0, end: 12 });
  });

  it("releases art that scrolled far away", () => {
    const long = (s: number) => m(s, 60);
    let r = nextArtRange(null, long(0));
    expect(r).toEqual({ start: 0, end: 8 });
    // Step along the row the way scroll events arrive.
    for (let s = 0; s <= 6000; s += 100) r = nextArtRange(r, long(s));
    // On screen: cards 30-34. Near: 27-37. Kept: from 1.5 viewports back (22).
    expect(r.start).toBeGreaterThanOrEqual(22);
    expect(r.start).toBeLessThanOrEqual(27);
    expect(r.end).toBe(38);
    // A jump far away drops everything that was kept.
    expect(nextArtRange(r, long(10000))).toEqual(rangeWithin(long(10000), 500));
  });

  it("doesn't flicker when the row moves back and forth near an edge", () => {
    const long = (s: number) => m(s, 60);
    let r = nextArtRange(null, long(2000));
    const before = r;
    for (const s of [2100, 1950, 2150, 1900, 2000]) r = nextArtRange(r, long(s));
    expect(r.start).toBeLessThanOrEqual(before.start);
    expect(r.end).toBeGreaterThanOrEqual(before.end);
  });
});

describe("Shelf", () => {
  afterEach(() => vi.restoreAllMocks());

  const shelf = () => (
    <Shelf title="Recently added">
      {Array.from({ length: 12 }, (_, i) => (
        <ShelfItem key={i}>
          <div className="card__art">
            <Art path={`/art/${i}`} fluid />
          </div>
        </ShelfItem>
      ))}
    </Shelf>
  );
  const withSrc = (c: HTMLElement) => [...c.querySelectorAll(".shelf__row > *")].map((el) => !!el.querySelector("img"));

  it("requests art for every card when nothing can be measured", () => {
    const { container } = render(shelf());
    expect(withSrc(container).every(Boolean)).toBe(true);
    // Boxes and skeletons stay either way.
    expect(container.querySelectorAll(".art")).toHaveLength(12);
  });

  it("only gives art to cards near the visible part of the row", () => {
    vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockImplementation(function (this: HTMLElement) {
      return this.classList.contains("shelf__row") ? 1000 : 0;
    });
    vi.spyOn(HTMLElement.prototype, "scrollWidth", "get").mockReturnValue(2400);
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
      const w = this.classList.contains("shelf__item") ? 200 : this.classList.contains("card__art") ? 176 : 0;
      return { width: w, height: w, top: 0, left: 0, right: w, bottom: w, x: 0, y: 0, toJSON: () => ({}) } as DOMRect;
    });
    const { container } = render(shelf());
    const row = container.querySelector<HTMLElement>(".shelf__row")!;
    expect(withSrc(container)).toEqual([...Array(8).fill(true), ...Array(4).fill(false)]);
    // Unloaded cards keep their box and show a skeleton.
    expect(container.querySelectorAll(".art")).toHaveLength(12);
    expect(container.querySelectorAll(".shelf__row > *")[11].querySelector(".art__skeleton")).not.toBeNull();
    // Art is requested at the measured width (176 css px), not a guess.
    expect(container.querySelector("img")!.getAttribute("width")).toBe("176");

    row.scrollLeft = 1400;
    fireEvent.scroll(row);
    expect(withSrc(container).every(Boolean)).toBe(true);
  });
});
