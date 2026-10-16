/**
 * Your shelf: the albums you own on vinyl, CD, cassette or another format,
 * the Discogs import with what it found ("On the shelf, not in your
 * library", each with Request and store links), and "Missing from the
 * shelf" (albums you play most and don't own) while its setting is on.
 * The Stats page shows a short version of the same.
 */
import { Disc3, RefreshCw } from "lucide-react";
import { useEffect, useState } from "react";
import { Art } from "../components/Art";
import { Button } from "../components/Button";
import { ShelfMark, usePhysicalDialog } from "../components/PhysicalDialog";
import { Skeleton } from "../components/Skeleton";
import { backend } from "../lib/backend";
import { plural } from "../lib/format";
import type { DiscogsStatus, LidarrRequestInput, PhysicalFormat, ShelfAlbum, ShelfOnly, ShelfStats } from "../lib/types";
import { FORMAT_LABEL, ensurePhysical, shelfLabel, usePhysical, useShowPhysical } from "../stores/physical";
import { nav } from "../stores/router";
import { useSettings } from "../stores/settings";
import { toast, useToasts } from "../stores/toasts";
import { RequestDialog, StoreLinks, errText, requestInput } from "./catalogue";
import { useLidarrReady } from "./lidarrShared";
import "./catalogue.css";
import "./shelf.css";

/** "Discogs data" attribution, shown wherever Discogs data is. */
export const DISCOGS_CREDIT = "Data provided by Discogs.";

/** The shelf numbers, reloaded when the marks or the setting change. */
export function useShelfStats(): ShelfStats | null {
  const on = useShowPhysical();
  const missing = useSettings((s) => !!s.settings?.shelfMissingList);
  const owned = usePhysical((s) => s.owned);
  const [stats, setStats] = useState<ShelfStats | null>(null);
  useEffect(() => ensurePhysical(), []);
  useEffect(() => {
    if (!on) return;
    let alive = true;
    backend.physicalStats().then(
      (s) => alive && setStats(s),
      () => alive && setStats(null),
    );
    return () => {
      alive = false;
    };
  }, [on, missing, owned]);
  return on ? stats : null;
}

const ON: Record<PhysicalFormat, string> = { vinyl: "on vinyl", cd: "on CD", cassette: "on cassette", other: "on another format" };

/** "3 on vinyl · 1 on CD" (copies). */
export function formatLine(s: ShelfStats): string {
  return s.byFormat.map((f) => `${f.copies} ${ON[f.format]}`).join(" · ");
}

/** Read the Discogs collection now; the toast says what it found. */
export async function importDiscogs(): Promise<boolean> {
  try {
    const r = await backend.discogsImport();
    const lede = `Read ${plural(r.total, "record")} from Discogs, ${r.matched} in your library`;
    if (r.unmatched.length > 0)
      useToasts.getState().push({ kind: "info", message: lede, action: { label: "See the rest", run: () => nav.go({ name: "shelf" }) } }, 6000);
    else toast.info(lede);
    return true;
  } catch (e) {
    toast.error(errText(e) || "Couldn't read your Discogs collection");
    return false;
  } finally {
    void usePhysical.getState().loadDiscogs();
  }
}

function AlbumRow({ a, sub }: { a: ShelfAlbum; sub: string }) {
  return (
    <li className="shelf-row">
      <button type="button" className="shelf-row__open" onClick={() => nav.go({ name: "album", id: a.albumId })} aria-label={`${a.title} by ${a.artist}, ${sub}`}>
        <Art path={a.thumb} size={48} alt="" />
        <span className="shelf-row__text">
          <span className="shelf-row__title truncate">{a.title}</span>
          <span className="shelf-row__sub truncate">
            {a.formats.length > 0 && <ShelfMark formats={a.formats} />}
            {a.artist} · {sub}
          </span>
        </span>
      </button>
    </li>
  );
}

/** Albums played most that aren't owned. */
export function MissingList({ albums, limit }: { albums: ShelfAlbum[]; limit?: number }) {
  const list = limit ? albums.slice(0, limit) : albums;
  if (list.length === 0) return <p className="shelf-empty">Every album you play a lot is on your shelf.</p>;
  return (
    <ol className="shelf-rows" aria-label="Missing from the shelf">
      {list.map((a) => (
        <AlbumRow key={a.albumId} a={a} sub={plural(a.plays, "play")} />
      ))}
    </ol>
  );
}

/** The Stats page's shelf. Hidden while the setting is off or nothing's owned. */
export function ShelfStatsSection() {
  const s = useShelfStats();
  if (!s || (s.albums === 0 && !s.missing?.length)) return null;
  return (
    <section className="stats-section shelf-stats" aria-labelledby="stats-shelf">
      <div className="stats-section__head">
        <h2 id="stats-shelf" className="stats-section__title">
          On your shelf
        </h2>
        <span className="stats-section__actions">
          <Button size="sm" variant="ghost" onClick={() => nav.go({ name: "shelf" })}>
            Open your shelf
          </Button>
        </span>
      </div>
      <dl className="stats-tiles shelf-tiles">
        <div className="stats-tile">
          <dt className="stats-tile__label">Albums</dt>
          <dd className="stats-tile__value">{s.albums}</dd>
        </div>
        {s.byFormat.map((f) => (
          <div key={f.format} className="stats-tile">
            <dt className="stats-tile__label">{FORMAT_LABEL[f.format]}</dt>
            <dd className="stats-tile__value">
              {f.copies}
              <span className="stats-tile__sub">{f.copies === 1 ? "copy" : "copies"}{f.copies !== f.albums && ` of ${plural(f.albums, "album")}`}</span>
            </dd>
          </div>
        ))}
      </dl>
      {s.mostPlayed.length > 0 && (
        <>
          <h3 className="shelf-subtitle">Owned albums you play most</h3>
          <ol className="shelf-rows" aria-label="Owned albums you play most">
            {s.mostPlayed.slice(0, 5).map((a) => (
              <AlbumRow key={a.albumId} a={a} sub={plural(a.plays, "play")} />
            ))}
          </ol>
        </>
      )}
      {s.missing && (
        <>
          <h3 className="shelf-subtitle">Missing from the shelf</h3>
          <MissingList albums={s.missing} limit={5} />
        </>
      )}
    </section>
  );
}

function ShelfOnlyRow({ item, lidarrReady, onRequest }: { item: ShelfOnly; lidarrReady: boolean; onRequest: (i: ShelfOnly) => void }) {
  const sub = [item.artist, item.year, FORMAT_LABEL[item.format]].filter(Boolean).join(" · ");
  return (
    <li className="shelf-only">
      <Art path={item.thumb} size={56} alt="" />
      <span className="shelf-only__text">
        <span className="shelf-only__title truncate">{item.title}</span>
        <span className="shelf-only__sub truncate">{sub}</span>
      </span>
      <span className="shelf-only__actions">
        {lidarrReady && (
          <Button size="sm" variant="primary" onClick={() => onRequest(item)} aria-label={`Request ${item.title}`}>
            Request
          </Button>
        )}
        <StoreLinks links={item.links} title={item.title} />
      </span>
    </li>
  );
}

function DiscogsPanel({ st }: { st: DiscogsStatus | null }) {
  if (!st) return <Skeleton width="60%" height={20} />;
  if (!st.saved)
    return (
      <div className="shelf-discogs">
        <p className="shelf-discogs__text">Bring in your Discogs collection: add your personal token in Settings, under Integrations.</p>
        <Button size="sm" onClick={() => nav.go({ name: "settings" })}>
          Open Settings
        </Button>
      </div>
    );
  const r = st.report;
  return (
    <div className="shelf-discogs">
      <p className="shelf-discogs__text">
        {r
          ? `Discogs collection of ${r.username}: ${plural(r.total, "record")}, ${r.matched} in your library. Last read ${new Date(r.importedAt * 1000).toLocaleDateString()}.`
          : `Discogs collection of ${st.username ?? "your account"}, not read yet.`}
      </p>
      <Button size="sm" icon={<RefreshCw size={14} />} disabled={st.importing} onClick={() => void importDiscogs()}>
        {st.importing ? "Reading…" : r ? "Read again" : "Import collection"}
      </Button>
    </div>
  );
}

export function ShelfPage() {
  const on = useShowPhysical();
  const owned = usePhysical((s) => s.owned);
  const discogs = usePhysical((s) => s.discogs);
  const stats = useShelfStats();
  const lidarrReady = useLidarrReady();
  const [asking, setAsking] = useState<LidarrRequestInput | null>(null);
  useEffect(() => {
    ensurePhysical();
    void usePhysical.getState().loadDiscogs();
  }, []);

  const request = async (item: ShelfOnly) => {
    try {
      const found = await backend.physicalFindRelease(item.artist, item.title);
      if (!found) return toast.error(`MusicBrainz doesn't know ${item.title} yet, so it can't be requested.`);
      if (found.libraryAlbumId) return nav.go({ name: "album", id: found.libraryAlbumId });
      if (found.refusal) return toast.error(found.refusal);
      setAsking(requestInput(found));
    } catch (e) {
      toast.error(errText(e) || "Couldn't look it up on MusicBrainz");
    }
  };

  if (!on)
    return (
      <div className="page-pad shelf-page">
        <h1 className="page-title">Your shelf</h1>
        <div className="empty-state" role="status">
          <Disc3 size={40} strokeWidth={1.5} aria-hidden="true" />
          <h2 className="empty-state__title">Your physical collection is hidden</h2>
          <p className="empty-state__text">Turn on Show physical collection in Settings to see it here.</p>
          <Button onClick={() => nav.go({ name: "settings" })}>Open Settings</Button>
        </div>
      </div>
    );

  const list = owned ? [...owned.values()] : null;
  const unmatched = discogs?.report?.unmatched ?? [];
  return (
    <div className="page-pad shelf-page">
      <h1 className="page-title">Your shelf</h1>
      <p className="import-page__sub">
        {list && stats
          ? `${plural(stats.albums, "album")} you own${stats.byFormat.length ? `: ${formatLine(stats)}` : ""}. Mark more from an album's menu.`
          : "Albums you own on vinyl, CD, cassette or another format. Mark one from an album's menu."}
      </p>
      <DiscogsPanel st={discogs} />

      {unmatched.length > 0 && (
        <section className="shelf-section" aria-labelledby="shelf-only">
          <h2 id="shelf-only" className="section-title">
            On the shelf, not in your library
          </h2>
          <ul className="shelf-only-list">
            {unmatched.map((u) => (
              <ShelfOnlyRow key={u.releaseId} item={u} lidarrReady={lidarrReady} onRequest={(i) => void request(i)} />
            ))}
          </ul>
          {!lidarrReady && <p className="cat-actions__note">Set up Lidarr in Settings to request them.</p>}
          <p className="shelf-credit">{DISCOGS_CREDIT}</p>
        </section>
      )}

      <section className="shelf-section" aria-labelledby="shelf-owned">
        <h2 id="shelf-owned" className="section-title">
          Owned albums
        </h2>
        {!list ? (
          <Skeleton width="100%" height={56} />
        ) : list.length === 0 ? (
          <p className="shelf-empty">Nothing yet. Pick "I own this" from an album's menu, or import your Discogs collection.</p>
        ) : (
          <ol className="shelf-rows shelf-rows--grid" aria-label="Owned albums">
            {list.map((o) => (
              <li key={o.albumId} className="shelf-row">
                <button
                  type="button"
                  className="shelf-row__open"
                  onClick={() => nav.go({ name: "album", id: o.albumId })}
                  aria-label={`${o.title} by ${o.artist}, ${shelfLabel(o.formats).toLowerCase()}`}
                >
                  <Art path={o.thumb} size={48} alt="" />
                  <span className="shelf-row__text">
                    <span className="shelf-row__title truncate">{o.title}</span>
                    <span className="shelf-row__sub truncate">
                      <ShelfMark formats={o.formats} />
                      {o.artist} · {shelfLabel(o.formats)}
                    </span>
                  </span>
                </button>
                <Button
                  size="sm"
                  variant="ghost"
                  aria-label={`Edit what you own of ${o.title}`}
                  onClick={() => usePhysicalDialog.getState().open({ albumId: o.albumId, title: o.title, artist: o.artist, thumb: o.thumb })}
                >
                  Edit
                </Button>
              </li>
            ))}
          </ol>
        )}
      </section>

      {stats?.missing && (
        <section className="shelf-section" aria-labelledby="shelf-missing">
          <h2 id="shelf-missing" className="section-title">
            Missing from the shelf
          </h2>
          <p className="shelf-lede">Albums you play most that you don't own yet.</p>
          <MissingList albums={stats.missing} />
        </section>
      )}

      <RequestDialog input={asking} onClose={() => setAsking(null)} onDone={() => setAsking(null)} />
    </div>
  );
}
