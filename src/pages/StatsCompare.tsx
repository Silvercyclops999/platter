/**
 * Compare with a friend: taste overlap, artists in common and the friend's
 * songs you haven't played. Only a summary is ever shared (top names with
 * weights), never the history; unreleased songs stay out unless you both
 * agree. Shown while Compare stats with friends is on.
 */
import { UsersRound } from "lucide-react";
import { useState } from "react";
import { Art } from "../components/Art";
import { TextLink } from "../components/ArtistLinks";
import { Switch } from "../components/Switch";
import { TrackList, TrackRow } from "../components/TrackRow";
import { backend } from "../lib/backend";
import type { CompareSummary } from "../lib/statsTypes";
import { useLoad } from "../lib/useLoad";
import { player } from "../stores/player";
import { nav } from "../stores/router";

function CompareView({ friend, unreleasedOk }: { friend: CompareSummary; unreleasedOk: boolean }) {
  const { data: r, error } = useLoad(() => backend.statsCompare(friend, unreleasedOk), [friend.name, unreleasedOk]);
  if (error) return <p className="page-error">Couldn't compare: {error.message}</p>;
  if (!r) return <p className="stats-muted">Comparing…</p>;
  const name = r.friend ?? "your friend";
  const playable = r.theirSongsNew.map((s) => s.track?.id).filter((x): x is string => !!x);
  return (
    <div className="compare" data-testid="compare-view">
      <div className="compare__overlap">
        <div className="compare__ring" style={{ ["--overlap" as string]: `${r.overlap}%` }} aria-hidden="true">
          <span>{r.overlap}%</span>
        </div>
        <p className="compare__overlap-text">
          <strong>{r.overlap}% taste overlap</strong> with {name}, from your top artists over the last six months.
        </p>
      </div>
      <div className="compare__cols">
        <section aria-labelledby="compare-common">
          <h3 id="compare-common" className="compare__h">
            Artists you both love
          </h3>
          {r.commonArtists.length === 0 ? (
            <p className="stats-muted">No artists in common yet.</p>
          ) : (
            <ol className="stats-list">
              {r.commonArtists.slice(0, 8).map((a, i) => (
                <li key={a.name} className="stats-row">
                  <span className="stats-row__num">{i + 1}</span>
                  <span className="stats-row__art">
                    <Art path={a.thumb} size={40} round kind="artist" />
                  </span>
                  <span className="stats-row__text">
                    {a.artistId ? (
                      <TextLink className="stats-row__title truncate" onActivate={() => nav.go({ name: "artist", id: a.artistId! })}>
                        {a.name}
                      </TextLink>
                    ) : (
                      <span className="stats-row__title truncate">{a.name}</span>
                    )}
                    <span className="compare__bars" aria-label={`You ${Math.round(a.mine * 100)}%, ${name} ${Math.round(a.theirs * 100)}%`}>
                      <span className="compare__bar compare__bar--mine" style={{ width: `${Math.min(100, a.mine * 300)}%` }} />
                      <span className="compare__bar compare__bar--theirs" style={{ width: `${Math.min(100, a.theirs * 300)}%` }} />
                    </span>
                  </span>
                </li>
              ))}
            </ol>
          )}
        </section>
        <section aria-labelledby="compare-new">
          <h3 id="compare-new" className="compare__h">
            {name} loves, you haven't played
          </h3>
          {r.theirSongsNew.length === 0 ? (
            <p className="stats-muted">You've played everything on their list.</p>
          ) : (
            <TrackList label={`Songs ${name} loves`} header={false}>
              {r.theirSongsNew.slice(0, 10).map((s, i) =>
                s.track ? (
                  <TrackRow key={`${s.artist}|${s.title}`} track={s.track} index={i + 1} showArt showAlbum={false} compact onPlay={() => void player.playTracks(playable, s.track!.id)} />
                ) : (
                  <div key={`${s.artist}|${s.title}`} className="stats-row stats-row--gone" role="row">
                    <span className="stats-row__art stats-row__art--none" role="gridcell" aria-hidden="true" />
                    <span className="stats-row__text" role="gridcell">
                      <span className="stats-row__title truncate">{s.title}</span>
                      <span className="stats-row__sub truncate">{s.artist} · not in your library</span>
                    </span>
                  </div>
                ),
              )}
            </TrackList>
          )}
        </section>
      </div>
    </div>
  );
}

export function CompareSection() {
  const { data: friends, error } = useLoad(() => backend.statsFriends(), []);
  const [picked, setPicked] = useState<string | null>(null);
  const [unreleasedOk, setUnreleasedOk] = useState(false);
  const friend = friends?.find((f) => f.name === picked) ?? friends?.[0] ?? null;
  return (
    <section className="stats-section" aria-labelledby="stats-compare">
      <h2 id="stats-compare" className="stats-section__title">
        Compare with a friend
      </h2>
      {error ? (
        <p className="page-error">Couldn't load your friends' stats: {error.message}</p>
      ) : !friends ? null : friends.length === 0 || !friend ? (
        <div className="compare__empty">
          <UsersRound size={28} strokeWidth={1.5} aria-hidden="true" />
          <p>When a Platter friend shares their stats with you, you can compare here. Only a summary is shared: top artists and songs, never your history.</p>
        </div>
      ) : (
        <>
          <div className="compare__pick">
            {friends.length > 1 && (
              <div className="chips" role="group" aria-label="Friend">
                {friends.map((f) => (
                  <button key={f.name ?? ""} type="button" aria-pressed={f === friend} className={`chip ${f === friend ? "is-selected" : ""}`} onClick={() => setPicked(f.name)}>
                    {f.name ?? "A friend"}
                  </button>
                ))}
              </div>
            )}
            <label className="compare__opt">
              <span id="compare-unreleased">Include unreleased songs (only when {friend.name ?? "they"} agrees too)</span>
              <Switch checked={unreleasedOk} onChange={setUnreleasedOk} labelledBy="compare-unreleased" />
            </label>
          </div>
          <CompareView friend={friend} unreleasedOk={unreleasedOk} />
        </>
      )}
    </section>
  );
}
