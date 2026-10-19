/**
 * Everything played on this phone, by day: one row per listening session
 * and where it came from. A row opens its record, artist or playlist; the
 * chevron folds out the songs that were played.
 */
import { ChevronDown } from "lucide-react";
import { useState } from "react";
import { Art } from "../../components/Art";
import { LikedCover } from "../../components/LikedCover";
import { backend } from "../../lib/backend";
import type { RecentsEntry } from "../../lib/types";
import { useLoad } from "../../lib/useLoad";
import { entryKey, entryLines, groupByDay } from "../../pages/recents";
import { player, usePlayer } from "../../stores/player";
import { PageFailed, SongRow } from "./common";
import { Empty, GhostRows, PageFrame } from "./kit";
import { recentInfo } from "./You";

function EntryRow({ e }: { e: RecentsEntry }) {
  const [open, setOpen] = useState(false);
  const info = recentInfo(e);
  const { title, sub } = entryLines(e);
  const ids = [...e.tracks].reverse().map((t) => t.id);
  const name = title ?? info.title;
  return (
    <li className="ph-recent">
      <div className="ph-x__row">
        <button type="button" className="ph-row" onClick={info.open}>
          {info.liked ? <LikedCover size={48} /> : <Art path={info.thumb} size={48} round={info.round} kind={info.round ? "artist" : "album"} monogram={info.round ? info.title : undefined} />}
          <span className="ph-row__text">
            <span className="ph-row__title">{name}</span>
            <span className="ph-row__sub">{sub}</span>
          </span>
        </button>
        <button type="button" className={`ph-icon-btn ph-recent__fold ${open ? "is-open" : ""}`} aria-expanded={open} aria-label={`${open ? "Hide" : "Show"} songs for ${name}`} onClick={() => setOpen(!open)}>
          <ChevronDown size={22} aria-hidden="true" />
        </button>
      </div>
      {open && (
        <ul className="ph-songs ph-recent__songs" aria-label={`Songs from ${name}`}>
          {e.tracks.map((t, i) => (
            <SongRow key={`${i}:${t.id}`} track={t} art={false} index={i + 1} onPlay={() => void player.playTracks(ids, t.id)} />
          ))}
        </ul>
      )}
    </li>
  );
}

export function PhoneRecents() {
  const trackId = usePlayer((s) => s.state?.track?.id ?? null);
  const { data, error, retry } = useLoad(() => backend.recents(), [trackId]);
  const groups = data ? groupByDay(data) : [];
  return (
    <PageFrame name="recents" title="Recents" lede="What you played here, by day and listening session.">
      {error && !data ? (
        <PageFailed what="your recents" retry={retry} />
      ) : !data ? (
        <GhostRows n={6} />
      ) : groups.length === 0 ? (
        <Empty title="Nothing played yet" text="Albums, playlists and songs you play here show up in this list." />
      ) : (
        groups.map((g) => (
          <section key={g.label} aria-label={g.label}>
            <h2 className="ph-x__day">{g.label}</h2>
            <ul className="ph-rows">
              {g.entries.map((e) => (
                <EntryRow key={entryKey(e)} e={e} />
              ))}
            </ul>
          </section>
        ))
      )}
    </PageFrame>
  );
}
