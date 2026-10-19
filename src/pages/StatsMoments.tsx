/**
 * Moments: small stories from a range. The first song, the day one song
 * was played over and over, the artist found that you played most, and a
 * song you came back to after a long break, as ruled rows with the cover.
 * A row plays the song or opens the artist.
 */
import { Art } from "../components/Art";
import { Skeleton } from "../components/Skeleton";
import { formatPlays, shortDate } from "../lib/stats";
import type { MomentSong, StatsMoments } from "../lib/statsTypes";
import { player } from "../stores/player";
import { nav } from "../stores/router";

export interface MomentCard {
  id: "first" | "replayed" | "discovery" | "comeback";
  eyebrow: string;
  title: string;
  sub: string;
  detail: string;
  thumb: string | null;
  round: boolean;
  /** What a click does, when it can do something. */
  action: { label: string; run: () => void } | null;
}

const timeFmt = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" });

/** "Oct 13" in the given year, "Oct 13, 2025" otherwise. */
export function momentDate(day: string, year: string): string {
  const full = shortDate(day);
  return day.startsWith(year) ? full.replace(/, \d{4}$/, "") : full;
}

function play(m: MomentSong): MomentCard["action"] {
  const id = m.song.track?.id;
  return id ? { label: `Play ${m.song.title}`, run: () => void player.playTracks([id], id) } : null;
}

/** The cards a range's moments make, in order; moments that didn't happen are left out. */
export function momentCards(m: StatsMoments, rangeTitle: string, isYear: boolean, year: string): MomentCard[] {
  const out: MomentCard[] = [];
  const date = (d: string) => momentDate(d, year);
  if (m.firstSong) {
    const f = m.firstSong;
    out.push({
      id: "first",
      eyebrow: isYear ? `First song of ${rangeTitle === "This year" ? "this year" : rangeTitle}` : "First song of the range",
      title: f.song.title,
      sub: f.song.artist,
      detail: `${date(f.day)}${f.at != null ? ` at ${timeFmt.format(new Date(f.at))}` : ""}`,
      thumb: f.song.track?.thumb ?? null,
      round: false,
      action: play(f),
    });
  }
  if (m.replayed) {
    const r = m.replayed;
    out.push({
      id: "replayed",
      eyebrow: "On repeat",
      title: r.song.title,
      sub: r.song.artist,
      detail: `${formatPlays(r.song.plays)} on ${date(r.day)}`,
      thumb: r.song.track?.thumb ?? null,
      round: false,
      action: play(r),
    });
  }
  if (m.discovery) {
    const d = m.discovery;
    out.push({
      id: "discovery",
      eyebrow: "Biggest discovery",
      title: d.artist.name,
      sub: `First played ${date(d.day)}`,
      detail: `${formatPlays(d.artist.plays)} since`,
      thumb: d.artist.thumb,
      round: true,
      action: d.artist.artistId ? { label: `Open ${d.artist.name}`, run: () => nav.go({ name: "artist", id: d.artist.artistId! }) } : null,
    });
  }
  if (m.comeback) {
    const c = m.comeback;
    out.push({
      id: "comeback",
      eyebrow: "Welcome back",
      title: c.song.song.title,
      sub: c.song.song.artist,
      detail: `Back after ${c.gapDays.toLocaleString()} days, ${date(c.song.day)}`,
      thumb: c.song.song.track?.thumb ?? null,
      round: false,
      action: play(c.song),
    });
  }
  return out;
}

function Card({ c }: { c: MomentCard }) {
  const body = (
    <>
      <span className={`moment__art ${c.round ? "is-round" : ""}`}>
        <Art path={c.thumb} fluid requestSize={64} round={c.round} kind={c.round ? "artist" : "album"} />
      </span>
      <span className="moment__text">
        <span className="lg-eyebrow moment__eyebrow">{c.eyebrow}</span>
        <span className="moment__title truncate">{c.title}</span>
        <span className="moment__sub truncate">{c.sub}</span>
        <span className="moment__detail lg-num">{c.detail}</span>
      </span>
    </>
  );
  return (
    <li className={`moment moment--${c.id}`} data-testid={`moment-${c.id}`}>
      {c.action ? (
        <button type="button" className="moment__btn" aria-label={`${c.eyebrow}: ${c.title}, ${c.sub}. ${c.detail}. ${c.action.label}`} onClick={c.action.run}>
          {body}
        </button>
      ) : (
        <div className="moment__btn">{body}</div>
      )}
    </li>
  );
}

export function MomentsSection({ moments, rangeTitle, isYear, year, error }: { moments: StatsMoments | undefined; rangeTitle: string; isYear: boolean; year: string; error: boolean }) {
  if (error) return null;
  const cards = moments ? momentCards(moments, rangeTitle, isYear, year) : null;
  if (cards && !cards.length) return null;
  return (
    <section className="stats-section" aria-labelledby="stats-moments" data-shot="moments">
      <div className="lg-head">
        <h2 id="stats-moments" className="lg-head__title stats-section__title">
          Moments
        </h2>
      </div>
      <ul className={`moments moments--n${cards?.length ?? 4}`}>
        {cards
          ? cards.map((c) => <Card key={c.id} c={c} />)
          : [0, 1, 2, 3].map((i) => (
              <li key={i} className="moment" aria-hidden="true">
                <span className="moment__btn">
                  <Skeleton width={64} height={64} radius="row" />
                  <span className="moment__text">
                    <Skeleton width="50%" height={12} />
                    <Skeleton width="80%" height={16} style={{ marginTop: 8 }} />
                    <Skeleton width="60%" height={12} style={{ marginTop: 8 }} />
                  </span>
                </span>
              </li>
            ))}
      </ul>
    </section>
  );
}
