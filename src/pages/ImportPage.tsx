/**
 * Library import: each source's report, the review queue for unsure
 * matches, imported playlists (songs not in the library dimmed) and the
 * Missing list. Nothing here writes to Plex except "Save as Plex playlist"
 * and "Like matched songs", each after a confirm.
 */
import { Heart, ListMusic } from "lucide-react";
import { useEffect, useState } from "react";
import { Button } from "../components/Button";
import { Skeleton, SkeletonLines } from "../components/Skeleton";
import { backend } from "../lib/backend";
import { formatDuration } from "../lib/format";
import { importSourceName } from "../lib/importStrings";
import type { ImportDecision, ImportReport, ImportReviewItem, ImportedList } from "../lib/importTypes";
import type { Track } from "../lib/types";
import { nav } from "../stores/router";
import { useSettings } from "../stores/settings";
import { toast } from "../stores/toasts";
import { countParts, errText, importChanged, skippedLine, songsLine, useImportOverview } from "./importShared";
import { ListView } from "./ImportListView";
import { MissingView } from "./ImportMissing";
import "./library-import.css";

const n = (v: number) => v.toLocaleString();

function when(ms: number): string {
  return new Date(ms).toLocaleDateString(undefined, { month: "short", day: "numeric", year: "numeric" });
}

function artistOf(t: Track): string {
  return t.trackArtist ?? t.artistTitle;
}

// ------------------------------------------------------------------ reports

function ReportCard({ r }: { r: ImportReport }) {
  const pct = r.songs.total ? Math.round((r.songs.matched / r.songs.total) * 100) : 0;
  const skipped = skippedLine(r.skipped);
  return (
    <article className="import-report" aria-label={`${importSourceName(r.source)} import`} data-testid={`report-${r.source}`}>
      <span className="import-report__source">{importSourceName(r.source)}</span>
      <span className="import-report__found">{songsLine(r)}</span>
      <div className="import-report__bar" role="img" aria-label={`${pct}% found`}>
        <span style={{ width: `${pct}%` }} />
      </div>
      <ul className="import-report__counts">
        {r.songs.unsure > 0 && <li>{n(r.songs.unsure)} to check</li>}
        {r.songs.missing > 0 && <li>{n(r.songs.missing)} not in your library</li>}
        {countParts(r).map((p) => (
          <li key={p}>{p}</li>
        ))}
        {r.plays.total > 0 && <li>{n(r.playsAdded)} new plays added</li>}
      </ul>
      {skipped && <p className="import-report__skipped">{skipped}</p>}
      <p className="import-report__when">
        Imported {when(r.importedAt)}
        {r.fileName ? ` from ${r.fileName}` : ""}
      </p>
    </article>
  );
}

// ------------------------------------------------------------------ review queue

function ReviewRow({ item, onDone }: { item: ImportReviewItem; onDone: () => void }) {
  const [busy, setBusy] = useState(false);
  const decide = (d: ImportDecision) => {
    setBusy(true);
    backend.importReview(item.songId, d).then(
      () => {
        importChanged();
        onDone();
      },
      (e) => {
        setBusy(false);
        toast.error(`Couldn't save that${errText(e) ? `: ${errText(e)}` : ""}`);
      },
    );
  };
  const label = `${item.title} by ${item.artist}`;
  return (
    <li className="import-review__item" aria-label={label}>
      <div className="import-review__song">
        <span className="import-review__title">{item.title}</span>
        <span className="import-review__meta">
          {[item.artist, item.album, item.durationMs ? formatDuration(item.durationMs) : null].filter(Boolean).join(" • ")}
        </span>
        <span className="import-review__meta">from {importSourceName(item.source)}</span>
      </div>
      <ul className="import-review__cands" aria-label={`Library songs it could be`}>
        {item.candidates.map((t, i) => (
          <li key={t.id} className="import-review__cand">
            <span className="import-review__cand-text truncate">
              <strong>{t.title}</strong> {artistOf(t)} • {t.albumTitle} • {formatDuration(t.durationMs)}
              {i === 0 && <span className="import-review__best">Best match</span>}
            </span>
            <Button
              size="sm"
              variant={i === 0 ? "secondary" : "ghost"}
              disabled={busy}
              aria-label={i === 0 ? `Accept ${t.title} on ${t.albumTitle}` : `Use ${t.title} on ${t.albumTitle}`}
              onClick={() => decide(i === 0 ? { kind: "accept" } : { kind: "pick", trackId: t.id })}
            >
              {i === 0 ? "Accept" : "Use this one"}
            </Button>
          </li>
        ))}
      </ul>
      <Button className="import-review__none" size="sm" variant="ghost" disabled={busy} onClick={() => decide({ kind: "none" })} aria-label={`None of these for ${label}`}>
        None of these
      </Button>
    </li>
  );
}

function ReviewQueue() {
  const [items, setItems] = useState<ImportReviewItem[] | null>(null);
  const load = () => backend.importReviewQueue().then(setItems, () => setItems([]));
  useEffect(() => {
    void load();
  }, []);
  if (items === null) return <SkeletonLines lines={3} widths={["60%", "80%", "40%"]} />;
  if (!items.length) return null;
  return (
    <section aria-labelledby="import-review-h">
      <h2 id="import-review-h">Check these matches</h2>
      <p className="import-page__sub">These might be songs you have, but Platter isn't sure. Your answer is kept when you import again.</p>
      <ul className="import-review">
        {items.map((it) => (
          <ReviewRow key={it.songId} item={it} onDone={() => void load()} />
        ))}
      </ul>
    </section>
  );
}

// ------------------------------------------------------------------ lists

function ListLink({ l }: { l: ImportedList }) {
  return (
    <li>
      <button type="button" className="import-list-link" onClick={() => nav.go({ name: "import", view: `list:${l.id}` })}>
        <span className="import-list-link__icon" aria-hidden="true">
          {l.liked ? <Heart size={18} /> : <ListMusic size={18} />}
        </span>
        <span className="import-list-link__text">
          <span className="import-list-link__name truncate">{l.name}</span>
          <span className="import-list-link__sub">{importSourceName(l.source)}</span>
        </span>
        <span className="import-list-link__rate">
          {n(l.matched)} of {n(l.total)} in your library
        </span>
      </button>
    </li>
  );
}

function Home() {
  const [o, err] = useImportOverview();
  if (err) return <p className="import-empty" role="alert">{err}</p>;
  if (!o) {
    return (
      <div aria-busy="true">
        <Skeleton width="100%" height={160} radius="card" />
      </div>
    );
  }
  if (!o.reports.length) {
    return (
      <div className="import-empty" role="status">
        <p>Nothing imported yet.</p>
        <p>Pick a file for a service in Settings, under Library import.</p>
        <Button className="import-empty__go" onClick={() => nav.go({ name: "settings" })}>
          Open Settings
        </Button>
      </div>
    );
  }
  return (
    <>
      <div className="import-reports">
        {o.reports.map((r) => (
          <ReportCard key={r.source} r={r} />
        ))}
      </div>
      <ReviewQueue key={o.reviewCount} />
      {o.lists.length > 0 && (
        <section aria-labelledby="import-lists-h">
          <h2 id="import-lists-h">Imported playlists</h2>
          <ul className="import-lists">
            {o.lists.map((l) => (
              <ListLink key={l.id} l={l} />
            ))}
          </ul>
        </section>
      )}
      {o.missingCount > 0 && (
        <section aria-labelledby="import-missing-h">
          <h2 id="import-missing-h">Missing songs</h2>
          <p className="import-page__sub">
            {n(o.missingCount)} {o.missingCount === 1 ? "song" : "songs"} from your imports {o.missingCount === 1 ? "isn't" : "aren't"} in your library.
          </p>
          <Button onClick={() => nav.go({ name: "import", view: "missing" })}>See missing songs</Button>
        </section>
      )}
    </>
  );
}

// ------------------------------------------------------------------ page

export function ImportPage({ view }: { view?: string }) {
  const s = useSettings((x) => x.settings);
  const on = (s?.importSources ?? []).length > 0;
  return (
    <div className="page-pad import-page" data-testid="import-page">
      {s && !on ? (
        <>
          <h1 className="page-title">Library import</h1>
          <div className="import-empty" role="status">
            <p>Library import is off.</p>
            <p>Turn on a service in Settings, under Library import.</p>
            <Button className="import-empty__go" onClick={() => nav.go({ name: "settings" })}>
              Open Settings
            </Button>
          </div>
        </>
      ) : view === "missing" ? (
        <MissingView />
      ) : view?.startsWith("list:") ? (
        <ListView id={view.slice(5)} />
      ) : (
        <>
          <h1 className="page-title">Library import</h1>
          <p className="import-page__sub">How much of what you imported is in your library.</p>
          <Home />
        </>
      )}
    </div>
  );
}
