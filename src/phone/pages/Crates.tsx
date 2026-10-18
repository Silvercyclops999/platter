/**
 * Crates on the phone: the library sorted into labels, producers, studios,
 * eras and genres. Pick a kind, then a crate to see its records.
 */
import { useState } from "react";
import { Art } from "../../components/Art";
import { backend } from "../../lib/backend";
import type { CrateKind } from "../../lib/types";
import { useLoad } from "../../lib/useLoad";
import { CRATE_TABS, CRATE_TAB_LABEL, crateLine, records } from "../../pages/crates";
import { player } from "../../stores/player";
import { nav } from "../../stores/router";
import { BackBar, PageFailed, PageLoading, ShelfCard } from "./common";

export function PhoneCrates({ kind: start }: { kind?: CrateKind }) {
  const [kind, setKind] = useState<CrateKind>(start ?? CRATE_TABS[0]);
  const { data, error, retry } = useLoad(() => backend.crates(kind), [kind]);
  return (
    <div className="ph-page ph-crates">
      <h1 className="ph-title">Crates</h1>
      <div className="ph-pills ph-pills--scroll" role="group" aria-label="Sort records by">
        {CRATE_TABS.map((k) => (
          <button key={k} type="button" className={`ph-chip ${kind === k ? "is-on" : ""}`} aria-pressed={kind === k} onClick={() => setKind(k)}>
            {CRATE_TAB_LABEL[k]}
          </button>
        ))}
      </div>
      {error && !data ? (
        <PageFailed what="crates" retry={retry} />
      ) : !data ? (
        <PageLoading />
      ) : data.crates.length === 0 ? (
        <p className="ph-rows__note">No crates of this kind yet. They fill in from your records' tags and sleeve notes.</p>
      ) : (
        <ul className="ph-crate-list">
          {data.crates.map((c) => (
            <li key={c.key}>
              <button type="button" className="ph-crate" onClick={() => nav.go({ name: "crate", kind: c.kind, key: c.key })}>
                <span className="ph-crate__stack" aria-hidden="true">
                  {c.albums.slice(0, 3).map((a, i) => (
                    <span key={a.id} className={`ph-crate__sleeve ph-crate__sleeve--${i}`}>
                      <Art path={a.thumb} size={52} sleeve={{ title: a.title, artist: a.artistTitle }} />
                    </span>
                  ))}
                </span>
                <span className="ph-row__text">
                  <span className="ph-row__title">{c.name}</span>
                  <span className="ph-row__sub tabular">{crateLine(c)}</span>
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

export function PhoneCrate({ kind, crateKey }: { kind: CrateKind; crateKey: string }) {
  const { data, error, retry } = useLoad(() => backend.cratePage(kind, crateKey), [kind, crateKey]);
  if (error && !data) return <PageFailed what="this crate" retry={retry} />;
  if (data === undefined) return <PageLoading />;
  if (data === null)
    return (
      <div className="ph-page">
        <BackBar />
        <p className="ph-rows__note">This crate is empty now.</p>
      </div>
    );
  return (
    <div className="ph-page ph-crate-page">
      <BackBar />
      <h1 className="ph-title">{data.name}</h1>
      <p className="ph-section-sub tabular">{records(data.albums.length)}</p>
      <div className="ph-actions">
        <span />
        <button type="button" className="ph-pill is-primary" onClick={() => void player.playTracks(data.trackIds)}>
          Play the crate
        </button>
      </div>
      <ul className="ph-lib-grid">
        {data.albums.map(({ album }) => (
          <li key={album.id}>
            <ShelfCard title={album.title} sub={album.artistTitle} thumb={album.thumb} kind="album" onOpen={() => nav.go({ name: "album", id: album.id })} />
          </li>
        ))}
      </ul>
    </div>
  );
}
