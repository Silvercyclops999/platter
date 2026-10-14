import { ExternalLink, X } from "lucide-react";
import { useEffect, useId, useState } from "react";
import { Art } from "../components/Art";
import { Dialog } from "../components/Dialog";
import { IconButton } from "../components/Button";
import { artUrl, backend } from "../lib/backend";
import type { Artist, ArtistInfo, InfoCredit } from "../lib/types";
import { nav } from "../stores/router";
import { useSettings } from "../stores/settings";
import { toast } from "../stores/toasts";
import { aboutFacts, bioParagraphs, listenersLine, photoPath, playsLine } from "./artist";
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

const openLink = (url: string) => backend.openInfoLink(url).catch(() => toast.error("Couldn't open the browser"));

/**
 * About card: one large button with the artist photo filling it behind a dark
 * fade, and over it at the bottom the stats line (Last.fm listeners when
 * known, else plays in the library) and the bio's first three lines. Opens
 * the dialog.
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
          {about && <span className="about-card__bio is-clamped">{about.replace(/\s*\n+\s*/g, " ")}</span>}
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

/** The full bio, facts, members, tags and where each piece came from. */
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
  const logo = info?.logo ? artUrl(info.logo, 240, 80) : undefined;
  return (
    <Dialog open={open} onClose={onClose} label={`About ${artist.title}`} className="about-dialog">
      <header className="about-dialog__head">
        <div className="about-dialog__titles">
          {logo && <img className="about-dialog__logo" src={logo} alt="" draggable={false} />}
          <h2 id={titleId} className="about-dialog__title">
            {artist.title}
          </h2>
          <p className="about-dialog__stats">
            <span>{playsLine(plays)}</span>
            {info?.listeners != null && <span>{listenersLine(info.listeners)}</span>}
          </p>
        </div>
        <IconButton label="Close" icon={<X size={20} />} onClick={onClose} />
      </header>

      {about && (
        <div className="about-dialog__bio">
          {bioParagraphs(about).map((p, i) => (
            <p key={i}>{p}</p>
          ))}
        </div>
      )}

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
                        <button
                          type="button"
                          className="about-link about-link--plain"
                          onClick={() => {
                            onClose();
                            nav.go({ name: "artist", id: m.artistId! });
                          }}
                        >
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
    </Dialog>
  );
}
