import { User } from "lucide-react";
import { createContext, useContext, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { prefersReducedMotion } from "../design/motion";
import { phoneArtPixelSize } from "../lib/artSize";
import { artUrl, backend } from "../lib/backend";
import { isArtReady, loadArt, markArtReady, readyRungBelow, type ArtLoad } from "./artLoader";
import { usePhoneForm } from "../phone/form";

/**
 * Rendered width (CSS px) of fluid art inside a layout that knows it, such as
 * the virtual grid's columns. Fluid art outside one uses its `requestSize`.
 */
export const FluidArtSize = createContext<number | null>(null);

/**
 * False while a layout (such as a shelf) has this art scrolled far out of
 * view: the box and skeleton stay, but no image is requested or kept decoded.
 */
export const ArtActive = createContext(true);

export interface ArtProps {
  path: string | null | undefined;
  /** Display size in CSS px (the box is square). Use `fluid` to fill the parent width. */
  size?: number;
  fluid?: boolean;
  /** Expected display size in CSS px when fluid and no `FluidArtSize` is provided. */
  requestSize?: number;
  round?: boolean;
  kind?: "album" | "artist" | "playlist";
  alt?: string;
  className?: string;
  onLoad?: (img: HTMLImageElement) => void;
  /** A name whose initials stand in when there's no picture (instead of the blank icon). */
  monogram?: string;
  /** A record with no cover: its title and artist set like a printed sleeve stand in. */
  sleeve?: SleeveText | null;
  /** The cover's colour (the server's tint, "rrggbb" or any CSS colour): on the phone it fills the box until the cover loads. */
  tint?: string | null;
}

/** A tint as a CSS colour: the server sends "rrggbb" without the hash. */
export function tintColour(tint: string | null | undefined): string | null {
  const t = tint?.trim();
  if (!t) return null;
  return /^[0-9a-f]{6}$/i.test(t) ? `#${t}` : t;
}

/**
 * On the phone, how far past the screen covers start loading: about one
 * screen ahead, down a page and along a shelf, so the next screenful is
 * ready when it scrolls in, without asking for a whole long list at once.
 */
export const PREFETCH_MARGIN = "100%";

/** One observer per scrolling box and margin, shared by everything watched in it. */
const observers = new WeakMap<Element, Map<string, IntersectionObserver>>();
const callbacksByMargin = new Map<string, WeakMap<Element, Set<() => void>>>();
const callbacksFor = (margin: string) => {
  let m = callbacksByMargin.get(margin);
  if (!m) callbacksByMargin.set(margin, (m = new WeakMap()));
  return m;
};

/** The box `el` scrolls in: a shelf, a page or a sheet (not `el` itself). */
function scrollBox(el: Element): Element | null {
  return el.parentElement?.closest(".ph-shelf__row, .ph-page, .ph-sheet__body") ?? null;
}

/** Calls `onNear` once, when `el` comes within `margin` of its scrolling box's view. */
function observeNear(el: Element, onNear: () => void, margin: string): () => void {
  const root = scrollBox(el);
  const key = root ?? document.documentElement;
  const nearCallbacks = callbacksFor(margin);
  const byMargin = observers.get(key) ?? new Map<string, IntersectionObserver>();
  observers.set(key, byMargin);
  let io = byMargin.get(margin);
  if (!io) {
    io = new IntersectionObserver(
      (entries) => {
        for (const e of entries) if (e.isIntersecting) for (const cb of [...(nearCallbacks.get(e.target) ?? [])]) cb();
      },
      { root, rootMargin: margin },
    );
    byMargin.set(margin, io);
  }
  const watcher = io;
  const set = nearCallbacks.get(el) ?? new Set();
  nearCallbacks.set(el, set);
  let done = false;
  const stop = () => {
    // Runs once: firing and the effect's cleanup both call it, and by the
    // second call another cover may have started a fresh set for `el`.
    if (done) return;
    done = true;
    set.delete(cb);
    if (!set.size && nearCallbacks.get(el) === set) {
      watcher.unobserve(el);
      nearCallbacks.delete(el);
    }
  };
  const cb = () => {
    stop();
    onNear();
  };
  set.add(cb);
  watcher.observe(el);
  return stop;
}

/**
 * Calls `onNear` once `el` is within about a screen of being seen. A cover
 * on a shelf waits for its shelf to come near the page's view first, so a
 * long Home doesn't fetch every shelf's covers at once.
 */
export function watchNear(el: Element, onNear: () => void, margin: string = PREFETCH_MARGIN): () => void {
  if (typeof IntersectionObserver === "undefined") {
    onNear();
    return () => {};
  }
  const box = scrollBox(el);
  if (box?.matches(".ph-shelf__row")) {
    let inner: (() => void) | null = null;
    const outer = observeNear(
      box,
      () => {
        inner = observeNear(el, onNear, margin);
      },
      margin,
    );
    return () => {
      outer();
      inner?.();
    };
  }
  return observeNear(el, onNear, margin);
}

/** Calls `onSeen` once `el` is on screen (no margin). */
export const watchSeen = (el: Element, onSeen: () => void) => watchNear(el, onSeen, "0px");

/** Whether `el` is on screen now. */
function onScreen(el: Element): boolean {
  if (typeof window === "undefined") return true;
  const r = el.getBoundingClientRect();
  return r.bottom > 0 && r.right > 0 && r.top < window.innerHeight && r.left < window.innerWidth && (r.width > 0 || r.height > 0);
}



/** What a fallback sleeve prints. */
export interface SleeveText {
  title: string;
  artist?: string | null;
}

/** Below this size a sleeve prints only its first letter; from the next, an artist line too. */
export const SLEEVE_LETTER_BELOW = 40;
export const SLEEVE_ARTIST_FROM = 96;

/**
 * The sleeve's flat tone, from its title: a hue for each record, so
 * neighbouring sleeves differ, at one muted lightness so they read as print.
 */
export function sleeveHue(title: string): number {
  let h = 0;
  for (let i = 0; i < title.length; i++) h = (h * 31 + title.charCodeAt(i)) >>> 0;
  return h % 360;
}

/** A typographic sleeve for a record without a cover. Decorative: the row or card names it. */
function FallbackSleeve({ text, size, fluid }: { text: SleeveText; size: number; fluid: boolean }) {
  const title = text.title.trim() || "Untitled";
  const hue = { "--sleeve-h": sleeveHue(title) } as React.CSSProperties;
  // Type scales with the sleeve's own width (container units), so a fluid
  // cover of any size sets its title right.
  if (!fluid && size < SLEEVE_LETTER_BELOW) {
    return (
      <span className="art__fallback art__sleeve art__sleeve--letter" aria-hidden="true" data-testid="fallback-sleeve" style={hue}>
        <span className="art__sleeve-letter">{Array.from(title)[0]?.toLocaleUpperCase()}</span>
      </span>
    );
  }
  const artist = fluid || size >= SLEEVE_ARTIST_FROM ? text.artist?.trim() : null;
  return (
    <span className="art__fallback art__sleeve" aria-hidden="true" data-testid="fallback-sleeve" style={hue}>
      <span className="art__sleeve-title">{title}</span>
      {artist && <span className="art__sleeve-artist">{artist}</span>}
    </span>
  );
}

/** Up to two initials of a name ("Cold Coast" is CC, "Ostra" is O). */
export function initials(name: string): string {
  const words = name
    .replace(/^the\s+/i, "")
    .split(/[\s&+/-]+/)
    .map((w) => w.replace(/[^\p{L}\p{N}]/gu, ""))
    .filter(Boolean);
  return words
    .slice(0, 2)
    .map((w) => w[0]!.toLocaleUpperCase())
    .join("");
}

/** Platter's P, the mark a plain paper sleeve carries when a record or playlist has no cover. */
export const P_MARK_PATH = "M296 876V456A240 240 0 1 1 408 659.02V876Z M616 456A80 80 0 1 0 456 456A80 80 0 1 0 616 456Z";

function PaperSleeve() {
  return (
    <span className="art__fallback art__paper" aria-hidden="true" data-testid="paper-sleeve">
      <svg className="art__paper-mark" viewBox="296 216 480 660">
        <path fillRule="evenodd" d={P_MARK_PATH} />
      </svg>
    </span>
  );
}

/** What stands in when there's no picture, or it failed: a printed sleeve, initials, a paper sleeve, or (for a person) a figure. */
function NoPicture({ sleeve, kind, round, monogram, cssSize, size, fluid }: { sleeve?: SleeveText | null; kind: string; round?: boolean; monogram?: string; cssSize: number; size: number; fluid?: boolean }) {
  if (sleeve && kind === "album" && !round) return <FallbackSleeve text={sleeve} size={cssSize} fluid={!!fluid} />;
  if (monogram && initials(monogram))
    return (
      <span className="art__fallback art__monogram" aria-hidden="true" style={{ fontSize: Math.max(12, Math.round(cssSize * 0.36)) }}>
        {initials(monogram)}
      </span>
    );
  if (kind !== "artist" && !round) return <PaperSleeve />;
  return (
    <span className="art__fallback" aria-hidden="true">
      <User size={fluid ? 40 : Math.max(16, Math.round(size * 0.38))} strokeWidth={1.5} />
    </span>
  );
}

/**
 * Fixed-aspect artwork box: reserves its square before the image arrives (no
 * layout shift), shows a skeleton until loaded, then fades the image in. Art
 * that is already decoded when it mounts (from cache) shows at once, without
 * the fade.
 *
 * Images are requested at the rendered size times the display's pixel ratio,
 * rounded up to a shared step (see lib/artSize), since decoded art is most of
 * the WebView's memory. The phone loads its covers its own way (PhoneArt).
 */
export function Art(props: ArtProps) {
  const phone = usePhoneForm((s) => s.phone);
  return phone ? <PhoneArt {...props} /> : <DeskArt {...props} />;
}

function DeskArt({ path, size = 48, fluid, requestSize, round, kind = "album", alt = "", className = "", onLoad, monogram, sleeve }: ArtProps) {
  const fluidSize = useContext(FluidArtSize);
  const cssSize = fluid ? (fluidSize ?? requestSize ?? 200) : size;
  const active = useContext(ArtActive);
  const wanted = artUrl(path, cssSize);
  const src = active ? wanted : undefined;
  // Track which src loaded/failed so a new src starts fresh without an effect.
  const [loadedSrc, setLoadedSrc] = useState<string | null>(null);
  const [failedSrc, setFailedSrc] = useState<string | null>(null);
  // Set when the image finished loading after it was shown: only then fade.
  const [fadeSrc, setFadeSrc] = useState<string | null>(null);
  const img = useRef<HTMLImageElement>(null);
  // Released art starts over when it comes back: shown at once if the image
  // is still decoded in the cache, otherwise skeleton then fade.
  if (!src && (loadedSrc !== null || fadeSrc !== null)) {
    setLoadedSrc(null);
    setFadeSrc(null);
  }
  const loaded = !!src && loadedSrc === src;
  const failed = !!src && failedSrc === src;

  // Already complete on mount (or on a src change): mark it loaded before the
  // first paint so it neither flashes the skeleton nor fades.
  useLayoutEffect(() => {
    const el = img.current;
    if (src && el?.complete && el.naturalWidth > 0) setLoadedSrc(src);
  }, [src]);

  const style = fluid ? undefined : { width: size, height: size };
  const box = Math.round(cssSize);

  return (
    <span className={`art ${round ? "art--round" : ""} ${loaded ? "is-loaded" : ""} ${loaded && fadeSrc === src ? "is-fading" : ""} ${className}`} style={style}>
      {(!wanted || failed) && <NoPicture sleeve={sleeve} kind={kind} round={round} monogram={monogram} cssSize={cssSize} size={size} fluid={fluid} />}
      {wanted && !failed && !loaded && <span className="art__skeleton skeleton" aria-hidden="true" />}
      {src && !failed && (
        <img
          ref={img}
          src={src}
          width={box}
          height={box}
          alt={alt}
          draggable={false}
          loading="lazy"
          decoding="async"
          onLoad={(e) => {
            if (loadedSrc !== src) {
              setLoadedSrc(src ?? null);
              setFadeSrc(src ?? null);
            }
            onLoad?.(e.currentTarget);
          }}
          onError={() => setFailedSrc(src ?? null)}
        />
      )}
    </span>
  );
}

/**
 * A cover on the phone. Until its size is decoded it shows the cover's own
 * colour (or a record's printed sleeve), or a smaller size of it that is
 * already decoded, but never a smaller size it would have to fetch: one
 * step, then the cover fades in over 150 ms. It asks for the cover about a
 * screen before it scrolls in, through the loader (at most six at once,
 * covers on screen first), and only draws it once decoded, in a box that
 * never changes size.
 */
function PhoneArt({ path, size = 48, fluid, requestSize, round, kind = "album", alt = "", className = "", onLoad, monogram, sleeve, tint }: ArtProps) {
  const fluidSize = useContext(FluidArtSize);
  const cssSize = fluid ? (fluidSize ?? requestSize ?? 200) : size;
  const active = useContext(ArtActive);
  const wanted = artUrl(path, cssSize);
  const px = phoneArtPixelSize(cssSize);
  const [ready, setReady] = useState<{ url: string; fade: boolean } | null>(null);
  const [failedUrl, setFailedUrl] = useState<string | null>(null);
  const box = useRef<HTMLSpanElement>(null);
  // Decoded already (another row, the mini player, a prefetch): shown at once, no fade.
  // Decided when the cover is first asked for, so one that finishes later still fades.
  const instant = useMemo(() => !!wanted && isArtReady(wanted), [wanted]);
  const shownUrl = instant ? wanted : ready?.url === wanted ? wanted : undefined;
  const fading = !instant && !!shownUrl && !!ready?.fade;
  const failed = !!wanted && failedUrl === wanted;

  useEffect(() => {
    const el = box.current;
    if (!wanted || !active || !el || isArtReady(wanted)) return;
    let job: ArtLoad | null = null;
    let live = true;
    const stopSeen = watchSeen(el, () => job?.promote());
    const stopNear = watchNear(el, () => {
      job = loadArt(wanted, { visible: onScreen(el) });
      job.done.then(
        () => {
          markArtReady(wanted, path, px);
          // Under reduced motion it simply appears.
          if (live) setReady({ url: wanted, fade: !prefersReducedMotion() });
        },
        () => live && setFailedUrl(wanted),
      );
    });
    return () => {
      live = false;
      stopNear();
      stopSeen();
      job?.cancel();
    };
  }, [wanted, active, path, px]);

  // Until it shows: a smaller size already decoded, else the cover's colour, else a printed sleeve, else a skeleton.
  // The fade is over (also when the animation never runs, a hidden page): what stood in can go.
  useEffect(() => {
    if (!fading) return;
    const t = setTimeout(() => setReady((r) => (r && r.url === shownUrl ? { ...r, fade: false } : r)), PHONE_ART_FADE_MS + 60);
    return () => clearTimeout(t);
  }, [fading, shownUrl]);

  // What stood in stays under the cover while it fades in.
  const covered = !!shownUrl && !fading;
  const lower = !covered && !failed && path ? readyRungBelow(path, px) : null;
  const lowerUrl = lower ? backend.artUrl(path!, lower, lower) : undefined;
  const fill = tintColour(tint);
  const waiting = !!wanted && !failed && !covered && !lowerUrl;
  const style = fluid ? undefined : { width: size, height: size };
  const side = Math.round(cssSize);

  return (
    <span ref={box} className={`art ${round ? "art--round" : ""} ${shownUrl ? "is-loaded" : ""} ${fading ? "is-fading" : ""} ${className}`} style={style}>
      {(!wanted || failed) && <NoPicture sleeve={sleeve} kind={kind} round={round} monogram={monogram} cssSize={cssSize} size={size} fluid={fluid} />}
      {waiting &&
        (fill ? (
          <span className="art__tint" aria-hidden="true" style={{ background: fill }} />
        ) : sleeve && kind === "album" && !round ? (
          // Printed by CSS from data attributes: a stand-in, not words of the page.
          <span className="art__fallback art__sleeve art__sleeve--pending" aria-hidden="true" style={{ "--sleeve-h": sleeveHue(sleeve.title) } as React.CSSProperties}>
            <span className="art__sleeve-title" data-text={sleeve.title} />
            {sleeve.artist && <span className="art__sleeve-artist" data-text={sleeve.artist} />}
          </span>
        ) : (
          <span className="art__skeleton skeleton" aria-hidden="true" />
        ))}
      {lowerUrl && <img className="art__under" src={lowerUrl} width={side} height={side} alt="" aria-hidden="true" draggable={false} decoding="async" />}
      {shownUrl && (
        <img
          key={shownUrl}
          src={shownUrl}
          width={side}
          height={side}
          alt={alt}
          draggable={false}
          decoding="async"
          onLoad={(e) => onLoad?.(e.currentTarget)}
          onError={() => setFailedUrl(shownUrl)}
        />
      )}
    </span>
  );
}

/** How long a cover takes to fade in on the phone (phone.css). */
export const PHONE_ART_FADE_MS = 150;

/** Start fetching a cover the phone is about to show (the next song's, at now playing's size). */
export function preloadArt(path: string | null | undefined, cssSize: number) {
  const url = artUrl(path, cssSize);
  if (!url || isArtReady(url)) return;
  void loadArt(url, { visible: false }).done.then(
    () => markArtReady(url, path, phoneArtPixelSize(cssSize)),
    () => {},
  );
}
