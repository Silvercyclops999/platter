import { useEffect, useState } from "react";
import { Art } from "../components/Art";
import { Button } from "../components/Button";
import { bringBack, loadDeletedPlaylists, useDeletedPlaylists } from "../stores/playlists";
import { nav } from "../stores/router";
import { goneByDay, loadGone, markGoneSeen, useSafety } from "../stores/safety";
import "../components/smart-editor.css";

/** Songs and playlists that left the server in the last 30 days. */
export function GonePage() {
  const gone = useSafety((s) => s.gone);
  const failed = useSafety((s) => s.goneFailed);
  const playlists = (useDeletedPlaylists((s) => s.list) ?? []).filter((k) => k.gone);
  const [busy, setBusy] = useState<number | null>(null);
  useEffect(() => {
    void Promise.all([loadGone(), loadDeletedPlaylists()]).then(() => markGoneSeen());
  }, []);
  const days = goneByDay(gone ?? []);
  const empty = gone !== null && gone.length === 0 && playlists.length === 0;
  return (
    <div className="page-pad bookmarks" data-testid="gone-page">
      <h1 className="page-title">Gone from your server</h1>
      {failed ? (
        <p className="bookmarks__empty" role="alert">
          Couldn't read the list. Platter will try again next time you open this page.
        </p>
      ) : empty ? (
        <p className="bookmarks__empty" role="status">
          Nothing has left your server. Songs and playlists that go missing show up here for 30 days.
        </p>
      ) : (
        <p className="bookmarks__empty">What left your server in the last 30 days. Click a song to look for it again.</p>
      )}
      {playlists.length > 0 && (
        <section aria-label="Playlists">
          <h2 className="section-title">Playlists</h2>
          <ul className="bookmark-list">
            {playlists.map((k) => (
              <li key={k.id} className="bookmark-row">
                <span className="bookmark-row__open">
                  <span className="bookmark-row__text">
                    <span className="bookmark-row__title truncate">{k.title}</span>
                    <span className="bookmark-row__sub truncate">{k.trackIds.length === 1 ? "1 song" : `${k.trackIds.length} songs`}</span>
                  </span>
                </span>
                <Button
                  size="sm"
                  variant="ghost"
                  disabled={busy !== null}
                  aria-label={`Bring back ${k.title}`}
                  onClick={() => {
                    setBusy(k.id);
                    void bringBack(k.id).finally(() => setBusy(null));
                  }}
                >
                  Bring back
                </Button>
              </li>
            ))}
          </ul>
        </section>
      )}
      {days.map((d) => (
        <section key={d.day} aria-label={d.day}>
          <h2 className="section-title">{d.day}</h2>
          <ul className="bookmark-list">
            {d.songs.map((g) => (
              <li key={g.id} className="bookmark-row">
                <button
                  type="button"
                  className="bookmark-row__open"
                  aria-label={`Look for ${g.title} by ${g.artistTitle}`}
                  onClick={() => nav.go({ name: "search", query: `${g.title} ${g.artistTitle}` })}
                >
                  <Art path={g.thumb} size={48} kind="album" />
                  <span className="bookmark-row__text">
                    <span className="bookmark-row__title truncate">{g.title}</span>
                    <span className="bookmark-row__sub truncate">
                      {g.artistTitle} • {g.albumTitle}
                    </span>
                  </span>
                </button>
              </li>
            ))}
          </ul>
        </section>
      ))}
    </div>
  );
}
