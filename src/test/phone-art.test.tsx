/**
 * Covers on the phone: sizes come off a short ladder capped at 1200 px (no
 * more 4096 px covers), the same record at the same size is one URL across
 * the mini player, rows and Home, colour samples ask for a small copy, the
 * cover's tint shows until it loads and it fades in only after decoding,
 * and covers start loading about a screen before they scroll in.
 */
import { act, fireEvent, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Art, tintColour, watchNear } from "../components/Art";
import { artPixelSize, PHONE_ART_MAX, PHONE_ART_SAMPLE, PHONE_ART_STEPS, phoneArtPixelSize } from "../lib/artSize";
import { artSampleUrl, artUrl, backend } from "../lib/backend";
import { usePhoneForm } from "../phone/form";
import phoneCss from "../phone/phone.css?raw";
import parityCss from "../phone/pages/parity.css?raw";
import wrappedCss from "../shell/wrapped.css?raw";
import sleeveCss from "../pages/sleeve.css?raw";
import componentsCss from "../components/components.css?raw";

const size = (url: string | undefined) => Number(new URL(url!, "http://x").searchParams.get("w"));

describe("the phone's art ladder", () => {
  it("asks for at most 3x what shows, on a few fixed steps", () => {
    expect(PHONE_ART_STEPS).toEqual([96, 192, 360, 600, 1200]);
    expect(phoneArtPixelSize(44, 3)).toBe(192);
    expect(phoneArtPixelSize(48, 3)).toBe(192);
    expect(phoneArtPixelSize(56, 3)).toBe(192);
    expect(phoneArtPixelSize(112, 3)).toBe(360);
    expect(phoneArtPixelSize(136, 3)).toBe(600);
    expect(phoneArtPixelSize(200, 2)).toBe(600);
    expect(phoneArtPixelSize(30, 3)).toBe(96);
  });

  it("never asks for more than 1200, however big the cover or the screen", () => {
    expect(phoneArtPixelSize(390, 3)).toBe(PHONE_ART_MAX);
    expect(phoneArtPixelSize(900, 3)).toBe(PHONE_ART_MAX);
    expect(phoneArtPixelSize(1600, 4)).toBe(PHONE_ART_MAX);
    // A 4x screen counts as 3x.
    expect(phoneArtPixelSize(100, 4)).toBe(phoneArtPixelSize(100, 3));
    // The computer's ladder is unchanged (it reached 3840 for a 900 px cover on a 3x screen).
    expect(artPixelSize(900, 3)).toBe(3840);
    expect(artPixelSize(48, 2)).toBe(96);
  });

  describe("URLs", () => {
    beforeEach(() => {
      usePhoneForm.setState({ phone: true });
      vi.stubGlobal("devicePixelRatio", 3);
      // The browser mock hands back pictures without sizes; ask what was asked for.
      vi.spyOn(backend, "artUrl").mockImplementation((p, w, h) => `${p}?w=${w}&h=${h}`);
    });
    afterEach(() => {
      usePhoneForm.setState({ phone: false });
      vi.unstubAllGlobals();
      vi.restoreAllMocks();
    });

    it("the mini player, a row and a Home tile share one cover URL", () => {
      expect(artUrl("/t/1", 44)).toBe(artUrl("/t/1", 48));
      expect(artUrl("/t/1", 56)).toBe(artUrl("/t/1", 48));
      expect(size(artUrl("/t/1", 900))).toBe(1200);
    });

    it("a wide image keeps its shape under the cap", () => {
      const u = new URL(artUrl("/m/1", 1200, 480)!, "http://x");
      expect(Number(u.searchParams.get("w"))).toBe(1200);
      expect(Number(u.searchParams.get("h"))).toBe(480);
    });

    it("colour samples ask for a small copy", () => {
      expect(size(artSampleUrl("/t/1"))).toBe(PHONE_ART_SAMPLE);
      expect(artSampleUrl(null)).toBeUndefined();
    });

    it("the computer keeps its own sizes", () => {
      act(() => usePhoneForm.setState({ phone: false }));
      expect(size(artUrl("/t/1", 48))).toBe(artPixelSize(48, 3));
      expect(size(artSampleUrl("/t/1"))).toBe(artPixelSize(64, 3));
    });
  });
});

describe("the stylesheets this touches", () => {
  it("have no stray characters glued to a rule (a stray `n` once hid the tint placeholder)", () => {
    for (const [name, css] of Object.entries({ phoneCss, wrappedCss, sleeveCss, parityCss, componentsCss })) {
      // A line that starts with something other than a selector, brace or comment, then opens a comment.
      expect(css, name).not.toMatch(/^[^\s/*{}.:#[@>~+&-][^{};]*\/\*/m);
      // A line that is a bare word or two letters on its own.
      expect(css, name).not.toMatch(/^[A-Za-z]{1,2}\s*$/m);
    }
    expect(phoneCss).toMatch(/^\.phone \.art__tint \{/m);
  });
});

describe("a cover loading on the phone", () => {
  beforeEach(() => usePhoneForm.setState({ phone: true }));
  afterEach(() => usePhoneForm.setState({ phone: false }));

  it("server tints become colours", () => {
    expect(tintColour("a1b2c3")).toBe("#a1b2c3");
    expect(tintColour("rgb(1, 2, 3)")).toBe("rgb(1, 2, 3)");
    expect(tintColour("")).toBeNull();
    expect(tintColour(null)).toBeNull();
  });

  it("shows its tint until the image is decoded, then fades in", async () => {
    let decoded!: () => void;
    const decode = vi.fn(() => new Promise<void>((r) => (decoded = r)));
    Object.defineProperty(HTMLImageElement.prototype, "decode", { value: decode, configurable: true });
    try {
      const { container } = render(<Art path="/t/1" size={48} tint="a1b2c3" sleeve={{ title: "Glass Season" }} />);
      const tint = container.querySelector(".art__tint") as HTMLElement;
      expect(tint.style.background).toBe("rgb(161, 178, 195)");
      // The printed sleeve and the skeleton make way for the colour.
      expect(container.querySelector(".art__skeleton, .art__sleeve--pending")).toBeNull();
      fireEvent.load(container.querySelector("img")!);
      expect(decode).toHaveBeenCalled();
      expect(container.querySelector(".art")).not.toHaveClass("is-loaded");
      await act(async () => decoded());
      expect(container.querySelector(".art")).toHaveClass("is-loaded", "is-fading");
      expect(container.querySelector(".art__tint")).toBeNull();
    } finally {
      delete (HTMLImageElement.prototype as { decode?: unknown }).decode;
    }
  });

  it("without a tint, a record keeps its printed sleeve while it loads", () => {
    const { container } = render(<Art path="/t/1" size={48} sleeve={{ title: "Glass Season" }} />);
    expect(container.querySelector(".art__sleeve--pending")).not.toBeNull();
    expect(container.querySelector(".art__tint")).toBeNull();
  });

  it("on the computer the tint is not used", () => {
    act(() => usePhoneForm.setState({ phone: false }));
    const { container } = render(<Art path="/t/1" size={48} tint="a1b2c3" />);
    expect(container.querySelector(".art__tint")).toBeNull();
    expect(container.querySelector(".art__skeleton")).not.toBeNull();
  });
});

describe("loading the next screenful", () => {
  type Entry = { target: Element; isIntersecting: boolean };
  const made: { root: Element | null; margin: string; cb: (e: Entry[]) => void; seen: Element[] }[] = [];
  beforeEach(() => {
    made.length = 0;
    class FakeIO {
      rec: (typeof made)[number];
      constructor(cb: (e: Entry[]) => void, opts: { root: Element | null; rootMargin: string }) {
        this.rec = { root: opts.root, margin: opts.rootMargin, cb, seen: [] };
        made.push(this.rec);
      }
      observe(el: Element) {
        this.rec.seen.push(el);
      }
      unobserve(el: Element) {
        this.rec.seen = this.rec.seen.filter((x) => x !== el);
      }
      disconnect() {}
    }
    vi.stubGlobal("IntersectionObserver", FakeIO);
    usePhoneForm.setState({ phone: true });
  });
  afterEach(() => {
    vi.unstubAllGlobals();
    usePhoneForm.setState({ phone: false });
  });

  it("covers in a list switch to eager about a screen before they show, with one observer per list", () => {
    const { container } = render(
      <div className="ph-page">
        <Art path="/t/1" size={48} />
        <Art path="/t/2" size={48} />
      </div>,
    );
    const page = container.querySelector(".ph-page")!;
    const io = made.find((m) => m.root === page)!;
    expect(made.filter((m) => m.root === page)).toHaveLength(1);
    expect(io.margin).toBe("100%");
    const imgs = [...container.querySelectorAll("img")];
    expect(imgs.map((i) => i.getAttribute("loading"))).toEqual(["lazy", "lazy"]);
    act(() => io.cb([{ target: imgs[0], isIntersecting: true }]));
    expect(imgs.map((i) => i.getAttribute("loading"))).toEqual(["eager", "lazy"]);
    expect(io.seen).toEqual([imgs[1]]);
  });

  it("a shelf's covers wait until the shelf itself is near the page's view", () => {
    const { container } = render(
      <div className="ph-page">
        <div className="ph-shelf__row">
          <Art path="/t/1" size={136} />
        </div>
      </div>,
    );
    const page = container.querySelector(".ph-page")!;
    const shelf = container.querySelector(".ph-shelf__row")!;
    const img = container.querySelector("img")!;
    const pageIo = made.find((m) => m.root === page)!;
    expect(pageIo.seen).toEqual([shelf]);
    expect(made.find((m) => m.root === shelf)).toBeUndefined();
    act(() => pageIo.cb([{ target: shelf, isIntersecting: true }]));
    const shelfIo = made.find((m) => m.root === shelf)!;
    expect(shelfIo.seen).toEqual([img]);
    act(() => shelfIo.cb([{ target: img, isIntersecting: true }]));
    expect(img.getAttribute("loading")).toBe("eager");
  });

  it("a cover that fired and then unmounts doesn't stop another cover watching the same place", () => {
    const el = document.createElement("div");
    document.body.appendChild(el);
    try {
      const first = vi.fn();
      const stopFirst = watchNear(el, first);
      const io = made[made.length - 1];
      io.cb([{ target: el, isIntersecting: true }]);
      expect(first).toHaveBeenCalledOnce();
      const second = vi.fn();
      watchNear(el, second);
      // The first one's effect cleanup runs after it already fired.
      stopFirst();
      expect(io.seen).toContain(el);
      io.cb([{ target: el, isIntersecting: true }]);
      expect(second).toHaveBeenCalledOnce();
      expect(first).toHaveBeenCalledOnce();
    } finally {
      el.remove();
    }
  });

  it("watching stops cleanly", () => {
    const el = document.createElement("div");
    const near = vi.fn();
    const stop = watchNear(el, near);
    stop();
    made.forEach((m) => m.cb([{ target: el, isIntersecting: true }]));
    expect(near).not.toHaveBeenCalled();
  });
});
