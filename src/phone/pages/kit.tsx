/**
 * Pieces the phone's other pages are built from: the page frame (back bar,
 * title, a line under it), pills, a two-up grid of tiles, rows, a list of
 * links, the play row, and a frame that makes a computer page's own
 * buttons phone-sized where a page is simple enough to borrow.
 */
import { ChevronRight, Pause, Play, Shuffle } from "lucide-react";
import { Suspense, useRef, type ReactNode } from "react";
import { Art } from "../../components/Art";
import { useArtistPhoto } from "../../lib/artistPhotos";
import { contextShuffleMode, shuffleLabel, smartOffered } from "../../lib/shuffleMode";
import type { PlayContextKind } from "../../lib/types";
import { player, usePlayer } from "../../stores/player";
import { nav, type Route } from "../../stores/router";
import { useSettings } from "../../stores/settings";
import { BackBar, PageLoading } from "./common";
import "./parity.css";

/** A page opened from another: Back, the title, an optional line under it, then the page. */
export function PageFrame({
  name,
  title,
  kicker,
  lede,
  right,
  className = "",
  children,
}: {
  /** The route it shows (kept on the element for tests and screenshots). */
  name: Route["name"];
  title: ReactNode;
  kicker?: ReactNode;
  lede?: ReactNode;
  right?: ReactNode;
  className?: string;
  children?: ReactNode;
}) {
  return (
    <div className={`ph-page ph-x ${className}`} data-phone-page={name}>
      <BackBar title={typeof title === "string" ? title : undefined} after={72} right={right} cover />
      {kicker && <p className="ph-eyebrow ph-x__kicker">{kicker}</p>}
      <h1 className="ph-title ph-title--page ph-x__title">{title}</h1>
      {lede && <p className="ph-x__lede">{lede}</p>}
      {children}
    </div>
  );
}

/**
 * A computer page that already works in one column: shown in the phone's
 * margins, with its buttons, chips and fields at a finger's size and its
 * desktop-only furniture (column headers, hover tools) put away.
 */
export function Borrowed({ name, children }: { name: Route["name"]; children: ReactNode }) {
  return (
    <div className="ph-page ph-wrap" data-phone-page={name}>
      <BackBar />
      <Suspense fallback={<PageLoading />}>{children}</Suspense>
    </div>
  );
}

/** Pills that act like tabs: one is on. */
export function Pills<T extends string | number>({
  label,
  items,
  value,
  onChange,
}: {
  label: string;
  items: readonly { id: T; label: string }[];
  value: T;
  onChange: (v: T) => void;
}) {
  return (
    <div className="ph-pills ph-pills--scroll ph-x__pills" role="group" aria-label={label}>
      {items.map((i) => (
        <button key={String(i.id)} type="button" className={`ph-chip ${value === i.id ? "is-on" : ""}`} aria-pressed={value === i.id} onClick={() => onChange(i.id)}>
          {i.label}
        </button>
      ))}
    </div>
  );
}

/** Two tiles across (three for round ones). */
export function TileGrid({ label, round, children }: { label: string; round?: boolean; children: ReactNode }) {
  return (
    <ul className={`ph-lib-grid ph-x__grid ${round ? "ph-lib-grid--round" : ""}`} aria-label={label}>
      {children}
    </ul>
  );
}

/** A record, artist or mix in a grid; `extra` sits under its words (a Get button). */
export function Tile({
  title,
  sub,
  thumb,
  art,
  round,
  kind = "album",
  sleeveArtist,
  onOpen,
  extra,
  label,
}: {
  title: string;
  sub?: ReactNode;
  thumb?: string | null;
  /** Art of its own (a mix's), in place of `thumb`. */
  art?: ReactNode;
  round?: boolean;
  kind?: "album" | "artist" | "playlist";
  sleeveArtist?: string;
  onOpen?: () => void;
  extra?: ReactNode;
  label?: string;
}) {
  const face = art ?? (
    <Art
      path={thumb ?? null}
      fluid
      requestSize={240}
      round={round}
      kind={kind}
      monogram={kind === "artist" ? title : undefined}
      sleeve={kind === "album" ? { title, artist: sleeveArtist ?? (typeof sub === "string" ? sub : "") } : undefined}
    />
  );
  const words = (
    <>
      <span className="ph-x__tile-art">{face}</span>
      <span className="ph-card-tile__title">{title}</span>
      {sub && <span className="ph-card-tile__sub">{sub}</span>}
    </>
  );
  return (
    <li className="ph-x__tile" aria-label={label}>
      {onOpen ? (
        <button type="button" className={`ph-card-tile ph-card-tile--fluid ${round ? "is-round" : ""}`} onClick={onOpen}>
          {words}
        </button>
      ) : (
        <div className={`ph-card-tile ph-card-tile--fluid ${round ? "is-round" : ""}`}>{words}</div>
      )}
      {extra && <div className="ph-get ph-x__tile-extra">{extra}</div>}
    </li>
  );
}

/** A row: art, two lines and something at its end. */
export function Row({
  art,
  title,
  sub,
  onOpen,
  end,
  label,
  className = "",
}: {
  art?: ReactNode;
  title: ReactNode;
  sub?: ReactNode;
  onOpen?: () => void;
  end?: ReactNode;
  label?: string;
  className?: string;
}) {
  const body = (
    <>
      {art}
      <span className="ph-row__text">
        <span className="ph-row__title">{title}</span>
        {sub && <span className="ph-row__sub">{sub}</span>}
      </span>
    </>
  );
  return (
    <li className={`ph-x__row ${className}`} aria-label={label}>
      {onOpen ? (
        <button type="button" className="ph-row" onClick={onOpen}>
          {body}
        </button>
      ) : (
        <div className="ph-row">{body}</div>
      )}
      {end && <div className="ph-x__row-end ph-get">{end}</div>}
    </li>
  );
}

export interface LinkItem {
  title: string;
  sub?: string;
  route?: Route;
  run?: () => void;
  icon?: ReactNode;
}

/** A card of links, each a full-width row with a chevron. */
export function LinkList({ label, items }: { label: string; items: LinkItem[] }) {
  if (!items.length) return null;
  return (
    <ul className="ph-set-list ph-x__links" aria-label={label}>
      {items.map((l) => (
        <li key={l.title}>
          <button type="button" className="ph-set-link" onClick={() => (l.run ? l.run() : l.route && nav.go(l.route))}>
            {l.icon && <span className="ph-x__link-icon" aria-hidden="true">{l.icon}</span>}
            <span className="ph-set-link__text">
              <span className="ph-set-link__title">{l.title}</span>
              {l.sub && <span className="ph-set-link__sub">{l.sub}</span>}
            </span>
            <ChevronRight size={18} aria-hidden="true" className="ph-set-link__chev" />
          </button>
        </li>
      ))}
    </ul>
  );
}

/** The page's play row: things on the left, shuffle and a large play on the right. */
export function PlayRow({
  name,
  playing,
  onPlay,
  onShuffle,
  shuffleOn,
  disabled,
  left,
}: {
  name: string;
  playing: boolean;
  onPlay: () => void;
  onShuffle?: () => void;
  /** Shuffle as a switch (a mix keeps it): shown lit while on. */
  shuffleOn?: boolean;
  disabled?: boolean;
  left?: ReactNode;
}) {
  return (
    <div className="ph-actions">
      <div className="ph-actions__left">{left}</div>
      <div className="ph-actions__right">
        {onShuffle && (
          <button
            type="button"
            className={`ph-icon-btn ${shuffleOn ? "is-on" : ""}`}
            aria-label={`Shuffle ${name}`}
            aria-pressed={shuffleOn}
            disabled={disabled}
            onClick={onShuffle}
          >
            <Shuffle size={24} aria-hidden="true" />
          </button>
        )}
        <button type="button" className="ph-play ph-play--page" aria-label={playing ? `Pause ${name}` : `Play ${name}`} disabled={disabled} onClick={onPlay}>
          {playing ? <Pause size={28} fill="currentColor" aria-hidden="true" /> : <Play size={28} fill="currentColor" aria-hidden="true" />}
        </button>
      </div>
    </div>
  );
}

/** An empty or switched-off page: a heading, a line, maybe a way on. */
export function Empty({ title, text, action }: { title: string; text?: ReactNode; action?: ReactNode }) {
  return (
    <div className="ph-empty" role="status">
      <p className="ph-empty__title">{title}</p>
      {text && <p className="ph-empty__text">{text}</p>}
      {action}
    </div>
  );
}

/** A button that goes somewhere, in the phone's pill style. */
export function GoPill({ route, children, primary }: { route: Route; children: ReactNode; primary?: boolean }) {
  return (
    <button type="button" className={`ph-pill ${primary ? "is-primary" : ""}`} onClick={() => nav.go(route)}>
      {children}
    </button>
  );
}

/** Ghost rows while a list loads. */
export function GhostRows({ n = 5 }: { n?: number }) {
  return (
    <ul className="ph-rows" aria-busy="true" aria-label="Loading">
      {Array.from({ length: n }, (_, i) => (
        <li key={i} className="ph-lib-row--ghost" />
      ))}
    </ul>
  );
}

/** An artist outside the library: their photo once it's found (as the row comes into view), a monogram until then. */
export function CataloguePhoto({ mbid, name, given, size, fluid }: { mbid: string | null; name: string; given?: string | null; size?: number; fluid?: boolean }) {
  const box = useRef<HTMLSpanElement>(null);
  const { photo } = useArtistPhoto(box, mbid, name, given);
  return (
    <span ref={box} className="ph-x__photo">
      <Art path={photo} size={size} fluid={fluid} requestSize={fluid ? 240 : undefined} round kind="artist" monogram={name} />
    </span>
  );
}

/** A centred cover over the page's name, like an album's. */
export function Hero({ art, children }: { art: ReactNode; children?: ReactNode }) {
  return (
    <header className="ph-album__head ph-x__hero">
      <div className="ph-album__cover">{art}</div>
      {children}
    </header>
  );
}

/** The first `step` of a long list, and a button for the next lot, so long pages stay quick. */
export function ShowMore({ total, shown, onMore, noun }: { total: number; shown: number; onMore: () => void; noun: string }) {
  if (shown >= total) return null;
  return (
    <button type="button" className="ph-pill ph-pill--wide" onClick={onMore}>
      Show more {noun} ({(total - shown).toLocaleString()} left)
    </button>
  );
}

/**
 * A page's shuffle switch for its context, as on the computer: a tap turns
 * shuffle on or off (smart shuffle too where offered) and it stays lit while
 * on; the play button then plays in that order. It never starts playback.
 */
export function PhoneShuffleToggle({ id, kind, name }: { id: string; kind: PlayContextKind; name: string }) {
  const mode = usePlayer((st) => contextShuffleMode(st.state, id));
  const smartAllowed = useSettings((st) => !!st.settings?.smartShuffle);
  const label = shuffleLabel(mode, (smartAllowed && smartOffered(kind)) || mode === "smart", name);
  return (
    <button
      type="button"
      className={`ph-icon-btn ${mode !== "off" ? "is-on" : ""}`}
      aria-label={label}
      aria-pressed={mode !== "off"}
      onClick={() => void player.cycleContextShuffle(id, kind, smartAllowed)}
    >
      <Shuffle size={24} aria-hidden="true" />
    </button>
  );
}
