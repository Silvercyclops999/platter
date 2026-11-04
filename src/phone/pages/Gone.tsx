/** Songs and playlists that left the server lately, on the phone. */
import { useEffect, useState } from "react";
import { Art } from "../../components/Art";
import { bringBack, loadDeletedPlaylists, useDeletedPlaylists } from "../../stores/playlists";
import { nav } from "../../stores/router";
import { goneByDay, loadGone, markGoneSeen, useSafety } from "../../stores/safety";
import { Empty, PageFrame, Row } from "./kit";

export function PhoneGone() {
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
    <PageFrame
      name="gone"
      title="Gone from your server"
      lede={empty || failed ? undefined : "What left your server in the last 30 days. Tap a song to look for it again."}
    >
      {failed && <Empty title="Couldn't read the list" text="Platter will try again next time you open this page." />}
      {empty && !failed && <Empty title="Nothing has left your server" text="Songs and playlists that go missing from your server show up here for 30 days." />}
      {playlists.length > 0 && (
        <section className="ph-x__section" aria-label="Playlists">
          <h2 className="ph-section-title">Playlists</h2>
          <ul className="ph-rows">
            {playlists.map((k) => (
              <Row
                key={k.id}
                title={k.title}
                sub={k.trackIds.length === 1 ? "1 song" : `${k.trackIds.length} songs`}
                end={
                  <button
                    type="button"
                    className="ph-pill"
                    disabled={busy !== null}
                    aria-label={`Bring back ${k.title}`}
                    onClick={() => {
                      setBusy(k.id);
                      void bringBack(k.id).finally(() => setBusy(null));
                    }}
                  >
                    Bring back
                  </button>
                }
              />
            ))}
          </ul>
        </section>
      )}
      {days.map((d) => (
        <section key={d.day} className="ph-x__section" aria-label={d.day}>
          <h2 className="ph-section-title">{d.day}</h2>
          <ul className="ph-rows">
            {d.songs.map((g) => (
              <Row
                key={g.id}
                art={<Art path={g.thumb} size={48} kind="album" />}
                title={g.title}
                sub={`${g.artistTitle} · ${g.albumTitle}`}
                label={`Look for ${g.title} by ${g.artistTitle}`}
                onOpen={() => nav.go({ name: "search", query: `${g.title} ${g.artistTitle}` })}
              />
            ))}
          </ul>
        </section>
      ))}
    </PageFrame>
  );
}
