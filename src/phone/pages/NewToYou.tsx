/**
 * New to you on the phone: artists like the ones you play most that aren't
 * in your library, three across, each with why and Not interested.
 */
import { X } from "lucide-react";
import { dismissArtist } from "../../pages/catalogue";
import { useNewToYou } from "../../pages/NewToYou";
import { nav } from "../../stores/router";
import { useSettings } from "../../stores/settings";
import { CataloguePhoto, Empty, GhostRows, GoPill, PageFrame, Row } from "./kit";

export function PhoneNewToYou() {
  const data = useNewToYou();
  const on = useSettings((s) => s.settings?.homeNewToYou !== false);
  return (
    <PageFrame name="newToYou" title="New to you" lede="Artists like the ones you play most that aren't in your library. Open one to see their albums, get them or find them in a store.">
      {!on ? (
        <Empty title="New to you is off" text="Turn it on in Settings." action={<GoPill route={{ name: "settings", row: "homeNewToYou" }}>Open Settings</GoPill>} />
      ) : !data ? (
        <GhostRows />
      ) : data.artists.length === 0 ? (
        <Empty title="Nothing new yet" text="Play some more and check back." />
      ) : (
        <ul className="ph-rows" aria-label="New to you">
          {data.artists.map((a) => (
            <Row
              key={a.mbid}
              art={<CataloguePhoto mbid={a.mbid} name={a.name} size={56} />}
              title={a.name}
              sub={a.because[0] ? `Because you play ${a.because[0]}` : "Not in your library"}
              onOpen={() => nav.go({ name: "catalogueArtist", mbid: a.mbid })}
              end={
                <button type="button" className="ph-icon-btn" aria-label={`Not interested in ${a.name}`} onClick={() => dismissArtist({ mbid: a.mbid, name: a.name })}>
                  <X size={20} aria-hidden="true" />
                </button>
              }
            />
          ))}
        </ul>
      )}
    </PageFrame>
  );
}
