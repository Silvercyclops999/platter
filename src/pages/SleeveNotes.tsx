/**
 * The album page's Sleeve notes tab: the release's facts on the left like
 * the back of a sleeve, the credits on the right, then song by song. Where
 * each fact came from shows on hover or focus (and is always read out);
 * people open their credit page.
 */
import { ExternalLink } from "lucide-react";
import { Fragment, useId, type ReactNode } from "react";

import { Button } from "../components/Button";
import { Skeleton } from "../components/Skeleton";
import { backend } from "../lib/backend";
import { ROLE_LABEL, SOURCE_MARK, STUDIO_LABEL, factRows, formatSleeveDate, roleLabel, sourcesText } from "../lib/sleeve";
import type { SleeveLookups, SleeveNotes, SleevePerson, SleeveRoleGroup, SleeveSource } from "../lib/types";
import { useSleeveNotes } from "../lib/useSleeveNotes";
import { nav } from "../stores/router";
import { toast } from "../stores/toasts";
import "./sleeve.css";

/** Opens Settings where a row is. */
const openSetting = (row: string) => nav.go({ name: "settings", row });

/**
 * Small printed marks naming where a fact came from. They stay out of sight
 * until the fact around them (a `.src-row`) is hovered or focused, then float
 * over its end like a tag; screen readers always hear them.
 */
export function SourceMarks({ sources }: { sources: SleeveSource[] }) {
  if (!sources.length) return null;
  const text = `From ${sourcesText(sources)}`;
  return (
    <span className="src-marks">
      <span className="sr-only">{text}</span>
      {sources.map((s) => (
        <span key={s} className={`src-mark src-mark--${s}`} aria-hidden="true">
          {SOURCE_MARK[s]}
        </span>
      ))}
    </span>
  );
}

/** A credited person: opens their credit page. */
export function PersonLink({ p, onOpen }: { p: SleevePerson; onOpen?: () => void }) {
  return (
    <button
      type="button"
      className="link sleeve-person"
      onClick={() => {
        onOpen?.();
        nav.go({ name: "credits", person: p.name });
      }}
    >
      {p.name}
    </button>
  );
}

/** "Produced by Ada Moss, Bo Kettle" lines, one per role. */
export function RoleLines({ groups, onOpen, marks = true }: { groups: SleeveRoleGroup[]; onOpen?: () => void; marks?: boolean }) {
  return (
    <dl className="sleeve-roles">
      {groups.map((g) => (
        <div key={g.role} className="sleeve-roles__row">
          <dt>{roleLabel(g.role)}</dt>
          <dd>
            <ul className="sleeve-roles__people">
              {g.people.map((p) => (
                <li key={p.key} className="src-row">
                  <PersonLink p={p} onOpen={onOpen} />
                  {p.details.length > 0 && <span className="sleeve-person__detail">{p.details.join(", ")}</span>}
                  {marks && <SourceMarks sources={p.sources} />}
                </li>
              ))}
            </ul>
          </dd>
        </div>
      ))}
    </dl>
  );
}

/** What to do when nothing is known, or how to learn more. */
export function SleeveHint({ lookups, empty }: { lookups: SleeveLookups; empty: boolean }) {
  if (lookups.unreleased) {
    return empty ? <p className="sleeve-hint">Unreleased music is never looked up online. Credits in the files' own tags show here.</p> : null;
  }
  return (
    <div className="sleeve-hint">
      {!lookups.online ? (
        <>
          <p>Turn on online lookups to find credits on MusicBrainz{lookups.discogs ? " and Discogs" : ""}.</p>
          <Button size="sm" variant="secondary" onClick={() => openSetting("sleeveLookups")}>
            Turn on online lookups
          </Button>
        </>
      ) : !lookups.musicbrainz ? (
        <>
          <p>MusicBrainz is off in Settings, so its credits aren't looked up.</p>
          <Button size="sm" variant="secondary" onClick={() => nav.go({ name: "settings", page: "artistInfo" })}>
            Turn on MusicBrainz
          </Button>
        </>
      ) : !lookups.discogs ? (
        <>
          <p>Add a Discogs token to also read credits from Discogs.</p>
          <Button size="sm" variant="ghost" onClick={() => nav.go({ name: "settings", page: "discogs" })}>
            Add a Discogs token
          </Button>
        </>
      ) : empty ? (
        <p>MusicBrainz and Discogs don't know this release yet. Credit tags in your files (producer, composer, performer) show here too.</p>
      ) : null}
    </div>
  );
}

function SleeveSkeleton() {
  return (
    <div className="sleeve" aria-busy="true" aria-label="Loading sleeve notes">
      <div className="sleeve__liner">
        {Array.from({ length: 5 }, (_, i) => (
          <Skeleton key={i} width={`${70 - i * 8}%`} height={14} style={{ margin: "10px 0" }} />
        ))}
      </div>
      <div className="sleeve__credits">
        {Array.from({ length: 4 }, (_, i) => (
          <Skeleton key={i} width={`${60 - i * 6}%`} height={14} style={{ margin: "10px 0" }} />
        ))}
      </div>
    </div>
  );
}

const openLink = (url: string) => void backend.openInfoLink(url).catch(() => toast.error("Couldn't open the browser"));

function isEmpty(n: SleeveNotes): boolean {
  return (
    factRows(n).filter((r) => r.key !== "status").length === 0 &&
    !n.notes &&
    n.studios.length === 0 &&
    n.groups.length === 0 &&
    n.tracks.every((t) => t.groups.length === 0 && !t.recorded)
  );
}

/** The songs that have credits or a recording date of their own. */
export function songsWithCredits(n: SleeveNotes) {
  return n.tracks.filter((t) => t.groups.length > 0 || t.recorded);
}

/** Song by song: each song's own credits under its number and title (the album page's tab, and the phone's album page). */
export function SongBySong({ n }: { n: SleeveNotes }) {
  const songs = songsWithCredits(n);
  const discs = new Set(n.tracks.map((t) => t.disc)).size > 1;
  return (
    <ol className="sleeve-songs">
      {songs.map((t, i) => (
        <Fragment key={t.trackId}>
          {discs && (i === 0 || songs[i - 1].disc !== t.disc) && (
            <li className="sleeve-songs__disc" aria-hidden="true">
              Disc {t.disc}
            </li>
          )}
          <li className="sleeve-song">
            <span className="sleeve-song__n">{t.number || ""}</span>
            <div className="sleeve-song__body">
              <span className="sleeve-song__title">{t.title}</span>
              {t.groups.map((g) => (
                <p key={g.role} className="sleeve-song__line src-row">
                  <span className="sleeve-song__role">{ROLE_LABEL[g.role] ?? roleLabel(g.role)}</span>{" "}
                  {g.people.map((p, j) => (
                    <Fragment key={p.key}>
                      {j > 0 && ", "}
                      <PersonLink p={p} />
                      {p.details.length > 0 && <span className="sleeve-person__detail">{p.details.join(", ")}</span>}
                    </Fragment>
                  ))}
                  <SourceMarks sources={[...new Set(g.people.flatMap((p) => p.sources))]} />
                </p>
              ))}
              {t.recorded && (
                <p className="sleeve-song__line sleeve-muted src-row" tabIndex={0}>
                  Recorded {formatSleeveDate(t.recorded.value)} <SourceMarks sources={t.recorded.sources} />
                </p>
              )}
            </div>
          </li>
        </Fragment>
      ))}
    </ol>
  );
}

/** Whether the release side has anything to print. */
export function linerHasFacts(n: SleeveNotes): boolean {
  return factRows(n).length > 0 || n.studios.length > 0 || !!n.notes;
}

/**
 * The release side of the sleeve: its facts set on rules like the back of a
 * record, the studios, other editions and the release's own notes, on the
 * printed panel. The album page's tab shows it whole; the phone shows the
 * same panel `compact` (the facts and the notes, a few lines of them) under
 * now playing and on its album page, with its own heading and a link.
 */
export function SleeveLiner({ n, compact = false, heading = "The release", action }: { n: SleeveNotes; compact?: boolean; heading?: string; action?: ReactNode }) {
  const id = useId();
  const rows = factRows(n);
  return (
    <section className={`sleeve__liner ${compact ? "sleeve__liner--compact" : ""}`} aria-labelledby={id}>
      <div className="sleeve__liner-head">
        <h2 id={id} className="sleeve__heading">
          {heading}
        </h2>
        {action}
      </div>
      {rows.length > 0 && (
        <dl className="sleeve-facts">
          {rows.map((r) => (
            <div key={r.key} className="sleeve-facts__row src-row">
              <dt>{r.label}</dt>
              <dd>
                <span className="sleeve-facts__value" tabIndex={compact ? undefined : 0}>
                  {r.text}
                </span>
                <SourceMarks sources={r.fact.sources} />
              </dd>
            </div>
          ))}
        </dl>
      )}
      {n.studios.length > 0 && (
        <dl className="sleeve-facts sleeve-facts--studios">
          {n.studios.map((s) => (
            <div key={`${s.role}-${s.name}`} className="sleeve-facts__row src-row">
              <dt>{STUDIO_LABEL[s.role] ?? s.role}</dt>
              <dd>
                <span className="sleeve-facts__value" tabIndex={compact ? undefined : 0}>
                  {s.name}
                </span>
                <SourceMarks sources={s.sources} />
              </dd>
            </div>
          ))}
        </dl>
      )}
      {n.editionNotes.length > 0 && (
        <ul className="sleeve-editions" aria-label="Other editions">
          {n.editionNotes.map((e) => (
            <li key={e.value} className="src-row" tabIndex={0}>
              {e.value} <SourceMarks sources={e.sources} />
            </li>
          ))}
        </ul>
      )}
      {n.notes && (
        <figure className="sleeve-note">
          <blockquote>{n.notes.value}</blockquote>
          <figcaption className="src-row" tabIndex={compact ? undefined : 0}>
            Release notes <SourceMarks sources={n.notes.sources} />
          </figcaption>
        </figure>
      )}
    </section>
  );
}

/** The tab's panel. Asks for the notes only while shown. */
export function SleeveNotesPanel({ albumId, title }: { albumId: string; title: string }) {
  const { data, loading, failed } = useSleeveNotes(albumId);
  if (loading && !data) return <SleeveSkeleton />;
  if (failed || !data) {
    return (
      <div className="empty-state sleeve-empty" role="status">
        <h2 className="empty-state__title">Couldn't read the sleeve notes</h2>
        <p className="empty-state__text">Try again in a moment.</p>
      </div>
    );
  }
  const n = data;
  if (isEmpty(n)) {
    return (
      <div className="empty-state sleeve-empty" role="status">
        <h2 className="empty-state__title">{n.looking ? "Looking for credits" : "No sleeve notes yet"}</h2>
        <p className="empty-state__text">
          {n.looking ? `Checking what's known about ${title}.` : `Nothing is known about who made ${title} yet.`}
        </p>
        {!n.looking && <SleeveHint lookups={n.lookups} empty />}
      </div>
    );
  }
  const songs = songsWithCredits(n);
  return (
    <div className="sleeve">

      <SleeveLiner n={n} />

      <section className="sleeve__credits" aria-labelledby="sleeve-credits-title">
        <h2 id="sleeve-credits-title" className="sleeve__heading">
          Credits
        </h2>
        {n.groups.length > 0 ? <RoleLines groups={n.groups} /> : <p className="sleeve-muted">No album credits yet.</p>}
        {songs.length > 0 && (
          <>
            <h3 className="sleeve__subheading">Song by song</h3>
            <SongBySong n={n} />
          </>
        )}
      </section>

      <footer className="sleeve__footer">
        {n.looking && (
          <p className="sleeve-muted" role="status">
            Looking for more credits…
          </p>
        )}
        <p className="sleeve-legend">
          Point at a fact to see where it came from: <span className="src-mark src-mark--tags">Tags</span> your files,{" "}
          <span className="src-mark src-mark--plex">Plex</span>, <span className="src-mark src-mark--musicbrainz">MB</span> MusicBrainz,{" "}
          <span className="src-mark src-mark--discogs">Discogs</span>.
        </p>
        <div className="sleeve-links">
          {n.links.map((l) => (
            <Button key={l.source} size="sm" variant="ghost" onClick={() => openLink(l.url)}>
              <ExternalLink size={14} aria-hidden="true" /> Open on {l.source === "musicbrainz" ? "MusicBrainz" : "Discogs"}
            </Button>
          ))}
        </div>
        {n.links.some((l) => l.source === "discogs") && <p className="sleeve-legend">Data provided by Discogs.</p>}
        {!n.looking && <SleeveHint lookups={n.lookups} empty={false} />}
      </footer>
    </div>
  );
}
