/**
 * Crates on the phone: the library sorted into labels, producers, studios,
 * eras and genres. They live in Library under the Crates pill; a crate opens
 * its own page of records.
 */
import { useEffect, useState } from "react";
import { Art } from "../../components/Art";
import { backend } from "../../lib/backend";
import type { CrateKind } from "../../lib/types";
import { useLoad } from "../../lib/useLoad";
import { CRATE_PLATE, CRATE_TABS, CRATE_TAB_LABEL, crateLine, records } from "../../pages/crates";
import { player } from "../../stores/player";
import { nav } from "../../stores/router";
import { BackBar, PageFailed, PageLoading, ShelfCard } from "./common";

const NOTE_KEY = "platter.phone.cratesNoteSeen";

/** Whether the one-line explanation of crates has been shown on this phone. */
export function cratesNoteSeen(): boolean {
  try {
    return localStorage.getItem(NOTE_KEY) === "1";
  } catch {
    return false;
  }
}

function markNoteSeen() {
  try {
    localStorage.setItem(NOTE_KEY, "1");
  } catch {
    // Shown again next time; no harm.
  }
}

export const CRATES_NOTE = "Crates are hand-picked piles of records, like a crate at a record shop.";

/** The crate kinds as a quiet row of tabs, then the crates of that kind. */
export function CratesBody({ start }: { start?: CrateKind }) {
  const [kind, setKind] = useState<CrateKind>(start ?? CRATE_TABS[0]);
  // The note shows on the first visit only, and stays for that visit.
  const [note] = useState(() => !cratesNoteSeen());
  useEffect(() => {
    if (note) markNoteSeen();
  }, [note]);
  const { data, error, retry } = useLoad(() => backend.crates(kind), [kind]);
  return (
    <div className="ph-crates">
      {note && (
        <p className="ph-note" role="note" data-testid="crates-note">
          {CRATES_NOTE}
        </p>
      )}
      <div className="ph-seg" role="tablist" aria-label="Sort records by">
        {CRATE_TABS.map((k) => (
          <button key={k} type="button" role="tab" className={`ph-seg__tab ${kind === k ? "is-on" : ""}`} aria-selected={kind === k} onClick={() => setKind(k)}>
            {CRATE_TAB_LABEL[k]}
          </button>
        ))}
      </div>
      {error && !data ? (
        <PageFailed what="crates" retry={retry} />
      ) : !data ? (
        <ul className="ph-crate-list" aria-busy="true">
          {[0, 1, 2, 3].map((i) => (
            <li key={i} className="ph-lib-row--ghost" />
          ))}
        </ul>
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

/** Crates reached by a link: the same list on a page of its own. */
export function PhoneCrates({ kind }: { kind?: CrateKind }) {
  return (
    <div className="ph-page">
      <BackBar />
      <h1 className="ph-title">Crates</h1>
      <CratesBody start={kind} />
    </div>
  );
}

export function PhoneCrate({ kind, crateKey }: { kind: CrateKind; crateKey: string }) {
  const { data, error, retry } = useLoad(() => backend.cratePage(kind, crateKey), [kind, crateKey]);
  if (error && !data)
    return (
      <div className="ph-page">
        <BackBar />
        <PageFailed what="this crate" retry={retry} />
      </div>
    );
  if (data === undefined) return <PageLoading />;
  if (data === null)
    return (
      <div className="ph-page">
        <BackBar />
        <div className="ph-empty">
          <p className="ph-empty__title">This crate is empty now</p>
          <p className="ph-empty__text">Its records were moved or retagged.</p>
        </div>
      </div>
    );
  return (
    <div className="ph-page ph-crate-page">
      <BackBar title={data.name} />
      <p className="ph-eyebrow">{CRATE_PLATE[kind]}</p>
      <h1 className="ph-title ph-title--page">{data.name}</h1>
      <div className="ph-actions">
        <p className="ph-section-sub tabular">{records(data.albums.length)}</p>
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
