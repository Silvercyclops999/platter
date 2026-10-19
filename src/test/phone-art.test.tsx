/**
 * Covers on the phone: sizes come off a short ladder capped at 1200 px (no
 * more 4096 px covers), the same record at the same size is one URL across
 * the mini player, rows and Home, colour samples ask for a small copy, the
 * cover's tint shows until it loads and it fades in only after decoding,
 * and covers start loading about a screen before they scroll in.
 */
import { act, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Art, PHONE_ART_FADE_MS, preloadArt, tintColour, watchNear } from "../components/Art";
import { artLoadState, isArtReady, loadArt, markArtReady, MAX_ART_LOADS, resetArtLoader } from "../components/artLoader";
import { NP_ART_CSS } from "../phone/NowPlaying";
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

/** An Image that loads only when the test says so, and records what it was asked for. */
const images: { src: string; decode: () => void; fail: () => void }[] = [];
class FakeImage {
  decoding = "";
  onload: (() => void) | null = null;
  onerror: (() => void) | null = null;
  private done!: () => void;
  private bad!: (e: unknown) => void;
  private p = new Promise<void>((res, rej) => {
    this.done = res;
    this.bad = rej;
  });
  set src(v: string) {
    images.push({ src: v, decode: () => this.done(), fail: () => {
        this.bad(new Error("x"));
        this.onerror?.();
      } });
  }
  decode() {
    return this.p;
  }
}
const sized = () => vi.spyOn(backend, "artUrl").mockImplementation((p, w, h) => `${p}?w=${w}&h=${h}`);

describe("a cover loading on the phone", () => {
  beforeEach(() => {
    images.length = 0;
    resetArtLoader();
    vi.stubGlobal("Image", FakeImage);
    vi.stubGlobal("devicePixelRatio", 3);
    sized();
    usePhoneForm.setState({ phone: true });
  });
  afterEach(() => {
    usePhoneForm.setState({ phone: false });
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    resetArtLoader();
  });

  it("server tints become colours", () => {
    expect(tintColour("a1b2c3")).toBe("#a1b2c3");
    expect(tintColour("rgb(1, 2, 3)")).toBe("rgb(1, 2, 3)");
    expect(tintColour("")).toBeNull();
    expect(tintColour(null)).toBeNull();
  });

  it("goes straight from its tint to the cover, decoded off screen first, with one fade", async () => {
    const { container } = render(<Art path="/t/1" size={48} tint="a1b2c3" sleeve={{ title: "Glass Season" }} />);
    const tint = container.querySelector(".art__tint") as HTMLElement;
    expect(tint.style.background).toBe("rgb(161, 178, 195)");
    expect(container.querySelector(".art__skeleton, .art__sleeve--pending, img")).toBeNull();
    expect(images.map((i) => i.src)).toEqual(["/t/1?w=192&h=192"]);
    await act(async () => images[0].decode());
    const art = container.querySelector(".art")!;
    expect(art).toHaveClass("is-loaded", "is-fading");
    const img = container.querySelector("img")!;
    expect(img.getAttribute("src")).toBe("/t/1?w=192&h=192");
    expect(img.getAttribute("decoding")).toBe("async");
    // The colour stays under it until the fade ends.
    expect(container.querySelector(".art__tint")).not.toBeNull();
    await act(() => new Promise((r) => setTimeout(r, PHONE_ART_FADE_MS + 80)));
    expect(art).not.toHaveClass("is-fading");
    expect(container.querySelector(".art__tint")).toBeNull();
  });

  it("a cover already decoded shows at once, with no fade and no new request", () => {
    markArtReady("/t/1?w=192&h=192", "/t/1", 192);
    const { container } = render(<Art path="/t/1" size={48} tint="a1b2c3" />);
    expect(container.querySelector(".art")).toHaveClass("is-loaded");
    expect(container.querySelector(".art")).not.toHaveClass("is-fading");
    expect(images).toHaveLength(0);
  });

  it("shows a smaller size only when it is decoded already and the right one isn't", async () => {
    markArtReady("/t/1?w=192&h=192", "/t/1", 192);
    const { container } = render(<Art path="/t/1" size={136} tint="a1b2c3" />);
    const under = container.querySelector("img.art__under")!;
    expect(under.getAttribute("src")).toBe("/t/1?w=192&h=192");
    expect(container.querySelector(".art__tint")).toBeNull();
    expect(images.map((i) => i.src)).toEqual(["/t/1?w=600&h=600"]);
    await act(async () => images[0].decode());
    await act(() => new Promise((r) => setTimeout(r, PHONE_ART_FADE_MS + 80)));
    expect(container.querySelector("img.art__under")).toBeNull();
    // Nothing decoded for another picture: its colour, never a fetched small step.
    const other = render(<Art path="/t/2" size={136} tint="a1b2c3" />);
    expect(other.container.querySelector("img")).toBeNull();
    expect(other.container.querySelector(".art__tint")).not.toBeNull();
  });

  it("without a tint, a record keeps its printed sleeve while it loads", () => {
    const { container } = render(<Art path="/t/1" size={48} sleeve={{ title: "Glass Season" }} />);
    expect(container.querySelector(".art__sleeve--pending")).not.toBeNull();
    expect(container.querySelector(".art__tint")).toBeNull();
  });

  it("a cover that fails shows the record's printed sleeve", async () => {
    const { container } = render(<Art path="/t/1" size={48} sleeve={{ title: "Glass Season" }} />);
    await act(async () => images[0].fail());
    expect(container.querySelector('[data-testid="fallback-sleeve"]')).not.toBeNull();
  });

  it("on the computer the tint is not used", () => {
    act(() => usePhoneForm.setState({ phone: false }));
    const { container } = render(<Art path="/t/1" size={48} tint="a1b2c3" />);
    expect(container.querySelector(".art__tint")).toBeNull();
    expect(container.querySelector(".art__skeleton")).not.toBeNull();
  });

  it("loads at most six at once, covers on screen first", async () => {
    const jobs = Array.from({ length: 10 }, (_, i) => loadArt(`/a/${i}`, { visible: false }));
    expect(artLoadState()).toEqual({ running: MAX_ART_LOADS, queued: 4 });
    const seen = loadArt("/a/seen", { visible: true });
    // A queued one that scrolled on screen moves up too, and a cancelled one never starts.
    jobs[8].promote();
    jobs[6].cancel();
    await act(async () => images[0].decode());
    await act(async () => images[1].decode());
    expect(images.slice(6).map((i) => i.src)).toEqual(["/a/8", "/a/seen"]);
    await act(async () => images[2].decode());
    expect(images.map((i) => i.src)).not.toContain("/a/6");
    expect(images[8].src).toBe("/a/7");
    void seen;
  });

  it("now playing's next cover is fetched ahead, at now playing's size", async () => {
    preloadArt("/t/next", NP_ART_CSS);
    expect(images.map((i) => i.src)).toEqual(["/t/next?w=1200&h=1200"]);
    await act(async () => images[0].decode());
    expect(isArtReady("/t/next?w=1200&h=1200")).toBe(true);
  });
});

describe("loading the next screenful", () => {
  type Entry = { target: Element; isIntersecting: boolean };
  const made: { root: Element | null; margin: string; cb: (e: Entry[]) => void; seen: Element[] }[] = [];
  beforeEach(() => {
    made.length = 0;
    images.length = 0;
    resetArtLoader();
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
    vi.stubGlobal("Image", FakeImage);
    usePhoneForm.setState({ phone: true });
  });
  afterEach(() => {
    vi.unstubAllGlobals();
    usePhoneForm.setState({ phone: false });
    resetArtLoader();
  });

  it("covers in a list are asked for about a screen before they show, with one observer per list", () => {
    const { container } = render(
      <div className="ph-page">
        <Art path="/t/1" size={48} />
        <Art path="/t/2" size={48} />
      </div>,
    );
    const page = container.querySelector(".ph-page")!;
    const io = made.find((m) => m.root === page && m.margin === "100%")!;
    expect(made.filter((m) => m.root === page && m.margin === "100%")).toHaveLength(1);
    const arts = [...container.querySelectorAll(".art")];
    expect(images).toHaveLength(0);
    act(() => io.cb([{ target: arts[0], isIntersecting: true }]));
    expect(images).toHaveLength(1);
    expect(io.seen).toEqual([arts[1]]);
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
    const art = container.querySelector(".art")!;
    const pageIo = made.find((m) => m.root === page && m.margin === "100%")!;
    expect(pageIo.seen).toEqual([shelf]);
    expect(made.find((m) => m.root === shelf && m.margin === "100%")).toBeUndefined();
    act(() => pageIo.cb([{ target: shelf, isIntersecting: true }]));
    const shelfIo = made.find((m) => m.root === shelf && m.margin === "100%")!;
    expect(shelfIo.seen).toEqual([art]);
    act(() => shelfIo.cb([{ target: art, isIntersecting: true }]));
    expect(images).toHaveLength(1);
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
