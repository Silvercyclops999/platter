import { AtSign, BookOpen, Camera, ChevronLeft, ChevronRight, Cloud, Disc, ExternalLink, Globe, Play, Radio, Users, X, type LucideIcon } from "lucide-react";
import { useEffect, useId, useState, type KeyboardEvent } from "react";
import { Art } from "../components/Art";
import { Dialog } from "../components/Dialog";
import { IconButton } from "../components/Button";
import { artUrl, backend } from "../lib/backend";
import type { Artist, ArtistInfo, InfoCredit, InfoLink } from "../lib/types";
import { nav } from "../stores/router";
import { useSettings } from "../stores/settings";
import { toast } from "../stores/toasts";
import { aboutFacts, bigNumber, bioParagraphs, bioSegments, galleryPaths, listenersLine, NO_BIO, photoPath, playsLine } from "./artist";
import "./artist-about.css";

/**
 * Outside info for an artist page. Answers from the core's cache at once;
 * reloads when the core says more arrived or a source is switched on or off.
 * Failures leave the page on Plex data only.
 */
export function useArtistInfo(id: string): ArtistInfo | null {
  const [info, setInfo] = useState<ArtistInfo | null>(null);
  const switches = useSettings((s) =>
    s.settings ? [s.settings.infoWikipedia, s.settings.infoLastfm, s.settings.infoFanart, s.settings.infoAudiodb].join() : "",
  );
  useEffect(() => {
    let alive = true;
    const load = () =>
      backend.artistInfo(id).then(
        (i) => alive && setInfo(i),
        () => {},
      );
    void load();
    const un = backend.listen("artistinfo://changed", (e) => {
      if (e.id === id) void load();
    });
    return () => {
      alive = false;
      un.then((f) => f());
    };
  }, [id, switches]);
  return info;
}

const openLink = (url: string, artistId?: string) =>
  (artistId ? backend.openInfoLink(url, artistId) : backend.openInfoLink(url)).catch(() => toast.error("Couldn't open the browser"));

/**
 * About card: one large button with the artist photo filling it behind a dark
 * fade, and over it at the bottom the stats line (Last.fm listeners when
 * known, else plays in the library) and the bio's first three lines, or a
 * note that there's none. Opens the dialog.
 */
export function AboutCard({ artist, about, plays, info }: { artist: Artist; about: string | null; plays: number; info: ArtistInfo | null }) {
  const [open, setOpen] = useState(false);
  const headingId = useId();
  const photo = photoPath(artist, info);
  const stats = info?.listeners != null ? listenersLine(info.listeners) : playsLine(plays);
  return (
    <section className="detail-section artist-about" aria-labelledby={headingId}>
      <h2 id={headingId} className="section-title">
        About
      </h2>
      <button type="button" className="about-card about-card--button" onClick={() => setOpen(true)} aria-label={`About ${artist.title}`}>
        <Art path={photo} fluid requestSize={820} kind="artist" className="about-card__bg" />
        <span className="about-card__shade" aria-hidden="true" />
        <span className="about-card__text">
          <span className="about-card__stats">{stats}</span>
          {/* One run of text, so the clamp shows the most lines. */}
          {about ? (
            <span className="about-card__bio is-clamped">{about.replace(/\s*\n+\s*/g, " ")}</span>
          ) : (
            <span className="about-card__bio about-card__bio--none">{NO_BIO}</span>
          )}
        </span>
      </button>
      <AboutDialog open={open} onClose={() => setOpen(false)} artist={artist} about={about} plays={plays} info={info} />
    </section>
  );
}

function CreditLine({ c }: { c: InfoCredit }) {
  return (
    <li className="about-credits__item" data-source={c.source}>
      {c.url ? (
        <button type="button" className="about-link" onClick={() => void openLink(c.url!)}>
          {c.text}
          <ExternalLink size={12} aria-hidden="true" />
        </button>
      ) : (
        <span>{c.text}</span>
      )}
      {c.licence && (
        <>
          {", "}
          {c.licenceUrl ? (
            <button type="button" className="about-link" onClick={() => void openLink(c.licenceUrl!)}>
              {c.licence}
            </button>
          ) : (
            c.licence
          )}
        </>
      )}
    </li>
  );
}

/** Generic icons per link kind (no brand marks). */
const LINK_ICONS: Record<string, LucideIcon> = {
  website: Globe,
  wikipedia: BookOpen,
  lastfm: Radio,
  instagram: Camera,
  x: AtSign,
  facebook: Users,
  youtube: Play,
  bandcamp: Disc,
  soundcloud: Cloud,
};

/**
 * Photos one at a time in a 16:9 strip, on black so portrait photos sit in
 * the middle. Arrows on each side (and the arrow keys) step through them.
 */
export function AboutGallery({ paths, name }: { paths: string[]; name: string }) {
  const [i, setI] = useState(0);
  const n = paths.length;
  // The list can change when more info arrives: stay in range.
  const at = n ? Math.min(i, n - 1) : 0;
  const step = (d: number) => setI((at + d + n) % n);
  const onKey = (e: KeyboardEvent) => {
    if (n < 2 || (e.key !== "ArrowLeft" && e.key !== "ArrowRight")) return;
    e.preventDefault();
    step(e.key === "ArrowRight" ? 1 : -1);
  };
  if (!n)
    return (
      <div className="about-gallery about-gallery--empty" aria-hidden="true">
        <Art path={null} fluid requestSize={640} kind="artist" />
      </div>
    );
  return (
    <div
      className="about-gallery"
      role="group"
      aria-roledescription="gallery"
      aria-label={n > 1 ? `Photos of ${name}, ${at + 1} of ${n}` : `Photo of ${name}`}
      tabIndex={n > 1 ? 0 : -1}
      onKeyDown={onKey}
    >
      <Art key={paths[at]} path={paths[at]} fluid requestSize={1280} kind="artist" className="about-gallery__img" />
      {n > 1 && (
        <>
          <button type="button" className="about-gallery__arrow about-gallery__arrow--prev" aria-label="Previous photo" onClick={() => step(-1)}>
            <ChevronLeft size={22} aria-hidden="true" />
          </button>
          <button type="button" className="about-gallery__arrow about-gallery__arrow--next" aria-label="Next photo" onClick={() => step(1)}>
            <ChevronRight size={22} aria-hidden="true" />
          </button>
          <span className="about-gallery__count" aria-hidden="true">
            {at + 1} / {n}
          </span>
        </>
      )}
    </div>
  );
}

function Stat({ value, label }: { value: string; label: string }) {
  return (
    <div className="about-stat">
      <span className="about-stat__value">{value}</span>
      <span className="about-stat__label">{label}</span>
    </div>
  );
}

/**
 * A centred modal: the photo gallery on top, then a narrow column of stats,
 * facts and links beside the full bio (library artists it names are linked),
 * tags and where each piece came from. Everything scrolls together. Closes
 * with ×, Esc or a click outside.
 */
export function AboutDialog({
  open,
  onClose,
  artist,
  about,
  plays,
  info,
}: {
  open: boolean;
  onClose: () => void;
  artist: Artist;
  about: string | null;
  plays: number;
  info: ArtistInfo | null;
}) {
  const titleId = useId();
  const facts = aboutFacts(info);
  const members = info?.members ?? [];
  const tags = info?.tags ?? [];
  const credits = info?.credits ?? [];
  const links: InfoLink[] = info?.links ?? [];
  // Names are found in the core's bio; Plex's own (shown before info arrives) has none.
  const bioLinks = info?.bio && about === info.bio.trim() ? (info.bioLinks ?? []) : [];
  const logo = info?.logo ? artUrl(info.logo, 240, 80) : undefined;
  const go = (id: string) => {
    onClose();
    nav.go({ name: "artist", id });
  };
  // Each linked name only at its first mention in the whole bio.
  const used = new Set<string>();
  return (
    <Dialog open={open} onClose={onClose} label={`About ${artist.title}`} className="about-dialog">
      {/* Stays in the corner while the dialog scrolls. */}
      <div className="about-dialog__closebar">
        <IconButton className="about-dialog__close" label="Close" icon={<X size={20} />} onClick={onClose} />
      </div>
      <AboutGallery paths={galleryPaths(artist, info)} name={artist.title} />

      <div className="about-dialog__body">
        <header className="about-dialog__head">
          {logo && <img className="about-dialog__logo" src={logo} alt="" draggable={false} />}
          <h2 id={titleId} className="about-dialog__title">
            {artist.title}
          </h2>
        </header>

        <div className="about-dialog__cols">
          <aside className="about-side" aria-label="Stats and links">
            {info?.listeners != null && <Stat value={bigNumber(info.listeners)} label={info.listeners === 1 ? "listener on Last.fm" : "listeners on Last.fm"} />}
            {info?.scrobbles != null && <Stat value={bigNumber(info.scrobbles)} label="plays on Last.fm" />}
            <Stat value={bigNumber(plays)} label={plays === 1 ? "play in your library" : "plays in your library"} />

            {(facts.length > 0 || members.length > 0) && (
              <dl className="about-facts">
                {facts.map((f) => (
                  <div key={f.label} className="about-facts__row">
                    <dt>{f.label}</dt>
                    <dd>{f.value}</dd>
                  </div>
                ))}
                {members.length > 0 && (
                  <div className="about-facts__row">
                    <dt>Members</dt>
                    <dd>
                      <ul className="about-members">
                        {members.map((m) => (
                          <li key={m.name}>
                            {m.artistId ? (
                              <button type="button" className="about-link about-link--plain" onClick={() => go(m.artistId!)}>
                                {m.name}
                              </button>
                            ) : (
                              m.name
                            )}
                          </li>
                        ))}
                      </ul>
                    </dd>
                  </div>
                )}
              </dl>
            )}

            {links.length > 0 && (
              <ul className="about-links" aria-label="Links">
                {links.map((l) => {
                  const Icon = LINK_ICONS[l.kind] ?? Globe;
                  return (
                    <li key={l.url}>
                      <button type="button" className="about-links__item" onClick={() => void openLink(l.url, artist.id)}>
                        <Icon size={16} aria-hidden="true" />
                        <span>{l.label}</span>
                      </button>
                    </li>
                  );
                })}
              </ul>
            )}
          </aside>

          <div className="about-main">
            <div className="about-dialog__bio">
              {about ? (
                bioParagraphs(about).map((p, i) => (
                  <p key={i}>
                    {bioSegments(p, bioLinks, used).map((seg, j) =>
                      seg.artistId ? (
                        <button key={j} type="button" className="about-bio-link" onClick={() => go(seg.artistId!)}>
                          {seg.text}
                        </button>
                      ) : (
                        seg.text
                      ),
                    )}
                  </p>
                ))
              ) : (
                <p className="about-dialog__none">{NO_BIO}</p>
              )}
            </div>

            {tags.length > 0 && (
              <ul className="about-tags" aria-label="Tags">
                {tags.map((t) => (
                  <li key={t} className="about-tags__tag">
                    {t}
                  </li>
                ))}
              </ul>
            )}

            {credits.length > 0 && (
              <ul className="about-credits" aria-label="Sources">
                {credits.map((c) => (
                  <CreditLine key={c.source} c={c} />
                ))}
              </ul>
            )}
          </div>
        </div>
      </div>
    </Dialog>
  );
}
