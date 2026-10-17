/**
 * Release radar: new albums, EPs and singles from artists you follow and
 * play most (from MusicBrainz, looked at about once a day), as a Home shelf
 * and a page. Each says whether it's in your library, or offers Request when
 * Lidarr is set up and its check passed.
 */
import { Ban, Check, Clock, Download, RefreshCw, Radar as RadarIcon } from "lucide-react";
import { Suspense, lazy, useState } from "react";
import { Art } from "../components/Art";
import { Button } from "../components/Button";
import { CardSkeleton } from "../components/Card";
import { Shelf, ShelfItem } from "../components/Shelf";
import { FilterChips } from "../components/Tabs";
import { backend } from "../lib/backend";
import { RADAR_FILTERS, filterRadar, radarDate, radarKind, type RadarFilter } from "../lib/radar";
import { timeAgo } from "../lib/relativeTime";
import type { AppError, RadarItem } from "../lib/types";
import { useApp } from "../stores/app";
import { useRadar } from "../stores/radar";
import { nav } from "../stores/router";
import { toast } from "../stores/toasts";
import { STATE_LABEL } from "./lidarrShared";
import "./home.css";
import "./lidarr-request.css";
import "./radar.css";

const LidarrRequestDialog = lazy(() => import("./LidarrRequest").then((m) => ({ default: m.LidarrRequestDialog })));

/** Most cards on the Home shelf. */
const SHELF_MAX = 12;

function open(item: RadarItem) {
  if (item.libraryAlbumId) nav.go({ name: "album", id: item.libraryAlbumId });
  else if (item.artistId) nav.go({ name: "artist", id: item.artistId });
}

function RadarAction({ item, lidarrReady, onRequest }: { item: RadarItem; lidarrReady: boolean; onRequest: () => void }) {
  if (item.libraryAlbumId) {
    return (
      <span className="lidarr-state lidarr-state--imported">
        <Check size={14} aria-hidden="true" />
        In your library
      </span>
    );
  }
  if (item.request && item.request !== "failed") {
    return (
      <span className={`lidarr-state lidarr-state--${item.request}`} title={item.auto ? "Requested by auto-request" : undefined}>
        {item.request === "queued" ? <Clock size={14} aria-hidden="true" /> : <Download size={14} aria-hidden="true" />}
        {STATE_LABEL[item.request]}
        {item.auto && <span className="sr-only"> (auto-request)</span>}
      </span>
    );
  }
  if (item.refusal) {
    return (
      <span className="lidarr-state radar-card__blocked" title={item.refusal}>
        <Ban size={14} aria-hidden="true" />
        Can't request
        <span className="sr-only">. {item.refusal}</span>
      </span>
    );
  }
  if (!lidarrReady) return null;
  return (
    <Button size="sm" onClick={onRequest} aria-label={`Request ${item.title}`}>
      {item.request === "failed" ? "Request again" : "Request"}
    </Button>
  );
}

/** One release: its cover, title, artist, type and date, and what can be done with it. */
export function RadarCard({ item, lidarrReady, now, onChanged }: { item: RadarItem; lidarrReady: boolean; now?: number; onChanged?: () => void }) {
  const [requesting, setRequesting] = useState(false);
  const opens = !!(item.libraryAlbumId || item.artistId);
  const upcoming = radarDate(item.date, now).startsWith("Out ");
  return (
    <div className={`card card--album radar-card ${item.libraryAlbumId ? "" : "radar-card--missing"}`} aria-label={`${item.title} by ${item.artist}`}>
      <div className="card__art radar-card__art">
        <Art path={item.thumb} fluid kind="album" />
        {item.new && <span className="radar-card__badge">New</span>}
        {upcoming && !item.new && <span className="radar-card__badge radar-card__badge--soon">Soon</span>}
      </div>
      {opens ? (
        <button type="button" className="card__link" onClick={() => open(item)} title={item.title}>
          <span className="card__title truncate">{item.title}</span>
        </button>
      ) : (
        <span className="card__title truncate">{item.title}</span>
      )}
      <div className="card__subtitle truncate">{item.artist}</div>
      <div className="radar-card__meta truncate">
        {radarKind(item.kind)} · {radarDate(item.date, now)}
      </div>
      <div className="radar-card__action">
        <RadarAction item={item} lidarrReady={lidarrReady} onRequest={() => setRequesting(true)} />
      </div>
      {requesting && (
        <Suspense fallback={null}>
          <LidarrRequestDialog
            open
            input={{ releaseGroupId: item.releaseGroupId, title: item.title, artist: item.artist, artistId: item.artistId }}
            onClose={() => setRequesting(false)}
            onDone={() => {
              setRequesting(false);
              onChanged?.();
            }}
          />
        </Suspense>
      )}
    </div>
  );
}

/** Home's shelf: the newest releases, with Show all for the page. Hidden while off, empty or offline. */
export function RadarShelf() {
  const { page, reload } = useRadar();
  const cachedOnly = useApp((s) => !!s.health?.cachedOnly);
  if (!page?.shown || page.items.length === 0 || cachedOnly) return null;
  return (
    <Shelf title="Release radar" skeletonKind="album" onShowAll={() => nav.go({ name: "releaseRadar" })}>
      {page.items.slice(0, SHELF_MAX).map((item) => (
        <ShelfItem key={item.releaseGroupId}>
          <RadarCard item={item} lidarrReady={page.lidarrReady} onChanged={reload} />
        </ShelfItem>
      ))}
    </Shelf>
  );
}

const errText = (e: unknown) => (e && typeof e === "object" && "message" in e ? String((e as AppError).message) : "");

/** The page: every release found in the last 90 days, newest first, filtered by type. */
export function ReleaseRadarPage() {
  const { page, reload } = useRadar();
  const [filter, setFilter] = useState<RadarFilter>("all");
  const [checking, setChecking] = useState(false);
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
  const busy = checking || !!page?.checking;
  const items = page ? filterRadar(page.items, filter) : [];
  const who = page?.artists === 1 ? "1 artist" : `${page?.artists ?? 0} artists`;
  return (
    <div className="page-pad shelf-page radar-page">
      <header className="radar-page__head">
        <h1 className="page-title">Release radar</h1>
        {page?.shown && (
          <div className="radar-page__sub">
            <p className="radar-page__line">
              New albums, EPs and singles from the {who} you follow and play most.
              {page.checkedAt ? ` Checked ${timeAgo(page.checkedAt).toLowerCase()}.` : ""}
            </p>
            <Button size="sm" variant="secondary" icon={<RefreshCw size={14} className={busy ? "spin" : undefined} />} disabled={busy} onClick={() => void check()}>
              {busy ? "Checking…" : "Check now"}
            </Button>
          </div>
        )}
      </header>
      {!page ? (
        <div className="shelf-page__grid" aria-busy="true">
          {Array.from({ length: 12 }, (_, i) => (
            <div key={i} aria-hidden="true">
              <CardSkeleton />
            </div>
          ))}
        </div>
      ) : !page.shown ? (
        <div className="empty-state" role="status">
          <RadarIcon size={40} strokeWidth={1.5} aria-hidden="true" />
          <h2 className="empty-state__title">Release radar is off</h2>
          <p className="empty-state__text">Turn it on in Settings to see new releases from artists you follow and play most.</p>
          <Button variant="secondary" size="sm" onClick={() => nav.go({ name: "settings" })}>
            Open Settings
          </Button>
        </div>
      ) : page.items.length === 0 ? (
        <div className="empty-state" role="status">
          <RadarIcon size={40} strokeWidth={1.5} aria-hidden="true" />
          <h2 className="empty-state__title">{page.checkedAt ? "Nothing new lately" : "No releases yet"}</h2>
          <p className="empty-state__text">
            {page.artists === 0
              ? "Follow artists, or play some music, and their new releases show up here."
              : page.checkedAt
                ? "None of your artists have put out anything in the last 90 days. Platter looks again every day."
                : "Platter looks for new releases about once a day. The first look is on its way."}
          </p>
        </div>
      ) : (
        <>
          <FilterChips label="Show" className="radar-page__chips" items={RADAR_FILTERS} value={filter} onChange={setFilter} />
          {items.length === 0 ? (
            <p className="home__empty" role="status">
              Nothing of this kind lately.
            </p>
          ) : (
            <div className="shelf-page__grid" role="list" aria-label="New releases">
              {items.map((item) => (
                <div role="listitem" key={item.releaseGroupId}>
                  <RadarCard item={item} lidarrReady={page.lidarrReady} onChanged={reload} />
                </div>
              ))}
            </div>
          )}
        </>
      )}
    </div>
  );
}
