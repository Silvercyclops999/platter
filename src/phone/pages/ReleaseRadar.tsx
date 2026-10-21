/**
 * Release radar on the phone: new albums, EPs and singles from artists you
 * follow and play most, newest first, each with where it stands or Get
 * (Lidarr, the same one-click Get as everywhere else).
 */
import { RefreshCw } from "lucide-react";
import { useState } from "react";
import { Art } from "../../components/Art";
import { backend } from "../../lib/backend";
import { RADAR_FILTERS, filterRadar, radarDate, radarKind, type RadarFilter } from "../../lib/radar";
import { timeAgo } from "../../lib/relativeTime";
import type { RadarItem } from "../../lib/types";
import { GetButton } from "../../pages/GetButton";
import { useRadar } from "../../stores/radar";
import { nav } from "../../stores/router";
import { toast } from "../../stores/toasts";
import { errText } from "../../pages/catalogue";
import { updateSettings } from "../../stores/settings";
import { Empty, GhostRows, PageFrame, Pills, Row } from "./kit";

function open(item: RadarItem) {
  if (item.libraryAlbumId) nav.go({ name: "album", id: item.libraryAlbumId });
  else if (item.artistId) nav.go({ name: "artist", id: item.artistId });
}

export function PhoneReleaseRadar() {
  const { page, reload } = useRadar();
  const [filter, setFilter] = useState<RadarFilter>("all");
  const [checking, setChecking] = useState(false);
  const busy = checking || !!page?.checking;
  const check = async () => {
    setChecking(true);
    try {
      await backend.releaseRadarCheck();
    } catch (e) {
      toast.error(errText(e) || "Couldn't look for new releases. Try again later");
    } finally {
      setChecking(false);
    }
  };
  const items = page ? filterRadar(page.items, filter) : [];
  const who = page?.artists === 1 ? "1 artist" : `${page?.artists ?? 0} artists`;
  return (
    <PageFrame
      name="releaseRadar"
      title="Release radar"
      lede={page?.shown ? `New releases from the ${who} you follow and play most.${page.checkedAt ? ` Checked ${timeAgo(page.checkedAt).toLowerCase()}.` : ""}` : undefined}
      right={
        page?.shown ? (
          <button type="button" className="ph-icon-btn" aria-label={busy ? "Checking" : "Check now"} disabled={busy} onClick={() => void check()}>
            <RefreshCw size={20} aria-hidden="true" className={busy ? "ph-spin" : undefined} />
          </button>
        ) : undefined
      }
    >
      {!page ? (
        <GhostRows />
      ) : !page.shown ? (
        <Empty
          title="Release radar is off"
          text="Turn it on to see new releases from the artists you follow and play most. Platter looks them up on MusicBrainz about once a day."
          action={
            <button type="button" className="ph-pill is-primary" onClick={() => void updateSettings({ releaseRadar: true }).then(reload)}>
              Turn on
            </button>
          }
        />
      ) : page.items.length === 0 ? (
        <Empty
          title={page.checkedAt ? "Nothing new lately" : "No releases yet"}
          text={page.artists === 0 ? "Follow artists, or play some music, and their new releases show up here." : "Platter looks for new releases about once a day."}
        />
      ) : (
        <>
          <Pills label="Show" items={RADAR_FILTERS} value={filter} onChange={setFilter} />
          {items.length === 0 ? (
            <Empty title="Nothing of this kind lately" />
          ) : (
            <ul className="ph-rows" aria-label="New releases">
              {items.map((item) => (
                <Row
                  key={item.releaseGroupId}
                  className="ph-x__row--get"
                  label={`${item.title} by ${item.artist}`}
                  art={<Art path={item.thumb} size={64} kind="album" sleeve={{ title: item.title, artist: item.artist }} />}
                  title={
                    <>
                      {item.title} {item.new && <span className="ph-x__tag is-accent">New</span>}
                    </>
                  }
                  sub={`${item.artist} · ${radarKind(item.kind)} · ${radarDate(item.date)}`}
                  onOpen={item.libraryAlbumId || item.artistId ? () => open(item) : undefined}
                  end={
                    <GetButton
                      album={{ releaseGroupId: item.releaseGroupId, title: item.title, artist: item.artist, artistId: item.artistId }}
                      known={item.request}
                      libraryAlbumId={item.libraryAlbumId}
                      refusal={item.refusal}
                      lidarrReady={page.lidarrReady}
                      onChanged={reload}
                      compact
                    />
                  }
                />
              ))}
            </ul>
          )}
        </>
      )}
    </PageFrame>
  );
}
