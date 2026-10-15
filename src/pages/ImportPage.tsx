/**
 * Library import: each source's report, the review queue for unsure
 * matches, imported playlists (read-only, songs not in the library dimmed)
 * and the Missing list. Nothing here writes to Plex except "Save as Plex
 * playlist" and "Like matched songs in Plex", each after a confirm.
 */
import { Heart, ListMusic } from "lucide-react";
import { useEffect, useState, type KeyboardEvent } from "react";
import { Button } from "../components/Button";
import { Dialog } from "../components/Dialog";
import { Skeleton, SkeletonLines } from "../components/Skeleton";
import { backend } from "../lib/backend";
import { formatDuration } from "../lib/format";
import { STORE_LINKS, importSourceName } from "../lib/importStrings";
import type { ImportDecision, ImportReport, ImportReviewItem, ImportedList, ImportedListPage, ImportedRow, MissingSong } from "../lib/importTypes";
import type { Track } from "../lib/types";
import { setLiked } from "../stores/likes";
import { player } from "../stores/player";
import { createPlaylist } from "../stores/playlists";
import { nav } from "../stores/router";
import { useSettings } from "../stores/settings";
import { toast } from "../stores/toasts";
import { countParts, errText, importChanged, skippedLine, songsLine, useImportOverview } from "./importShared";
import "./library-import.css";

const n = (v: number) => v.toLocaleString();

function when(ms: number): string {
  return new Date(ms).toLocaleDateString(undefined, { month: "short", day: "numeric", year: "numeric" });
}

function artistOf(t: Track): string {
  return t.trackArtist ?? t.artistTitle;
}

/** A small confirm with counts (the two Plex writes). */
function Confirm({ title, body, confirm, onConfirm, onClose }: { title: string; body: string; confirm: string; onConfirm: () => void; onClose: () => void }) {
  return (
    <Dialog open onClose={onClose} label={title} className="confirm">
      <h2 className="confirm__title">{title}</h2>
      <p className="confirm__body">{body}</p>
      <div className="confirm__actions">
        <Button variant="ghost" onClick={onClose}>
          Cancel
        </Button>
        <Button
          variant="primary"
          onClick={() => {
            onClose();
            onConfirm();
          }}
        >
          {confirm}
        </Button>
      </div>
    </Dialog>
  );
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

function Row({ row, index, onPlay }: { row: ImportedRow; index: number; onPlay?: () => void }) {
  const t = row.track;
  const playable = !!t && !!onPlay;
  const key = (e: KeyboardEvent) => {
    if (playable && e.key === "Enter") {
      e.preventDefault();
      onPlay!();
    }
  };
  const why = row.status === "unsure" ? "Not sure it's in your library" : "Not in your library";
  return (
    <li
      className={`import-row ${t ? "import-row--playable" : "import-row--missing"}`}
      tabIndex={playable ? 0 : undefined}
      onDoubleClick={playable ? onPlay : undefined}
      onKeyDown={key}
      aria-label={t ? `${t.title} by ${artistOf(t)}` : `${row.title} by ${row.artist}, ${why.toLowerCase()}`}
      data-status={t ? "matched" : row.status}
    >
      <span className="import-row__n">{index + 1}</span>
      <span className="import-row__main">
        <span className="import-row__title truncate">{t?.title ?? row.title}</span>
        <span className="import-row__artist truncate">{t ? artistOf(t) : row.artist}</span>
        {!t && <span className="import-row__why">{why}</span>}
      </span>
      <span className="import-row__album truncate">{t?.albumTitle ?? row.album ?? ""}</span>
      <span className="import-row__dur">{t ? formatDuration(t.durationMs) : row.durationMs ? formatDuration(row.durationMs) : ""}</span>
    </li>
  );
}

function ListView({ id }: { id: string }) {
  const [page, setPage] = useState<ImportedListPage | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [ask, setAsk] = useState(false);
  useEffect(() => {
    backend.importListPage(id).then(setPage, (e) => setErr(errText(e) || "Couldn't load this list"));
  }, [id]);
  if (err) return <p className="import-empty" role="alert">{err}</p>;
  if (!page) {
    return (
      <div aria-busy="true">
        <Skeleton width={280} height={40} />
        <div style={{ height: 24 }} />
        <SkeletonLines lines={6} widths={["90%", "85%", "88%", "80%", "86%", "70%"]} gap={20} />
      </div>
    );
  }
  const { list, rows } = page;
  const tracks = rows.map((r) => r.track).filter((t): t is Track => !!t);
  const ids = [...new Set(tracks.map((t) => t.id))];
  const left = list.total - tracks.length;
  const playFrom = (t: Track) => player.playTracks(ids, t.id);
  const leftNote = left ? ` ${n(left)} ${left === 1 ? "song isn't" : "songs aren't"} in your library and stay out.` : "";
  return (
    <>
      <h1 className="page-title">{list.name}</h1>
      <p className="import-page__sub">
        {importSourceName(list.source)} • {n(list.matched)} of {n(list.total)} songs in your library • Read only
      </p>
      <div className="import-actions">
        <Button variant="primary" disabled={!ids.length} onClick={() => player.playTracks(ids)}>
          Play
        </Button>
        <Button disabled={!ids.length} onClick={() => setAsk(true)}>
          {list.liked ? "Like matched songs in Plex" : "Save as Plex playlist"}
        </Button>
      </div>
      <ul className="import-rows" aria-label={list.name}>
        <li className="import-row import-row--header" aria-hidden="true">
          <span className="import-row__n">#</span>
          <span>Title</span>
          <span>Album</span>
          <span className="import-row__dur">Length</span>
        </li>
        {rows.map((r, i) => (
          <Row key={`${r.songId}-${i}`} row={r} index={i} onPlay={r.track ? () => playFrom(r.track!) : undefined} />
        ))}
      </ul>
      {ask &&
        (list.liked ? (
          <Confirm
            title="Like these songs in Plex?"
            body={`${n(ids.length)} ${ids.length === 1 ? "song" : "songs"} in your library will be liked in Plex.${leftNote}`}
            confirm={`Like ${n(ids.length)} ${ids.length === 1 ? "song" : "songs"}`}
            onConfirm={() => void setLiked(tracks, true)}
            onClose={() => setAsk(false)}
          />
        ) : (
          <Confirm
            title="Save as a Plex playlist?"
            body={`A new Plex playlist named "${list.name}" gets the ${n(ids.length)} ${ids.length === 1 ? "song" : "songs"} in your library.${leftNote}`}
            confirm="Save playlist"
            onConfirm={() => void createPlaylist({ trackIds: ids, title: list.name, open: false })}
            onClose={() => setAsk(false)}
          />
        ))}
    </>
  );
}

// ------------------------------------------------------------------ missing

function MissingView() {
  const [songs, setSongs] = useState<MissingSong[] | null>(null);
  const [err, setErr] = useState<string | null>(null);
  useEffect(() => {
    backend.importMissing().then(setSongs, (e) => setErr(errText(e) || "Couldn't load missing songs"));
  }, []);
  const open = (url: string) => backend.importOpenLink(url).catch((e) => toast.error(`Couldn't open the link${errText(e) ? `: ${errText(e)}` : ""}`));
  return (
    <>
      <h1 className="page-title">Missing songs</h1>
      <p className="import-page__sub">Songs from your imported playlists and likes that aren't in your library, the most wanted first.</p>
      {err && (
        <p className="import-empty" role="alert">
          {err}
        </p>
      )}
      {!songs && !err && <SkeletonLines lines={6} widths={["90%", "85%", "88%", "80%", "86%", "70%"]} gap={20} />}
      {songs && songs.length === 0 && (
        <p className="import-empty" role="status">
          Every imported song is in your library.
        </p>
      )}
      {songs && songs.length > 0 && (
        <ul className="import-rows" aria-label="Missing songs">
          <li className="import-row missing-row import-row--header" aria-hidden="true">
            <span className="import-row__n">#</span>
            <span>Title</span>
            <span>Album</span>
            <span>Find it</span>
          </li>
          {songs.map((s, i) => (
            <li key={`${s.artist}-${s.title}`} className="import-row missing-row" aria-label={`${s.title} by ${s.artist}`}>
              <span className="import-row__n">{i + 1}</span>
              <span className="import-row__main">
                <span className="import-row__title truncate">{s.title}</span>
                <span className="import-row__artist truncate">{s.artist}</span>
                <span className="missing-row__wants" title={s.wantedBy.join(", ")}>
                  Wanted by {s.wants === 1 ? s.wantedBy[0] ?? "1 list" : `${n(s.wants)} lists`}
                </span>
              </span>
              <span className="import-row__album truncate">{s.album ?? ""}</span>
              <span className="missing-row__links">
                {STORE_LINKS.map((st) => (
                  <Button key={st.key} size="sm" variant="ghost" onClick={() => void open(s.links[st.key])} aria-label={`Search ${st.name} for ${s.title}`}>
                    {st.name}
                  </Button>
                ))}
              </span>
            </li>
          ))}
        </ul>
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
