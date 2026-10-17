/**
 * Library doctor: one list of things worth fixing in the library, grouped
 * by kind, each with a plain explanation, a fix through the app's own flows
 * (Change cover, Fix match, merge, open the album) that asks before writing,
 * Ignore, and "Why?" with what it's based on.
 */
import { RefreshCw, Stethoscope } from "lucide-react";
import { Suspense, lazy, useCallback, useEffect, useId, useMemo, useState } from "react";
import { Art } from "../components/Art";
import { Button } from "../components/Button";
import { Dialog } from "../components/Dialog";
import { SkeletonLines } from "../components/Skeleton";
import { ChipRow } from "../components/Tabs";
import { backend } from "../lib/backend";
import { timeAgo } from "../lib/relativeTime";
import type { DoctorFinding, DoctorKind, DoctorSuggestedTags, DoctorReport, LidarrRequestInput } from "../lib/types";
import { useDoctor } from "../stores/doctor";
import { errText, runEdit, useCanEdit, usePlexEdit, type EditTarget } from "../stores/plexEdit";
import { nav } from "../stores/router";
import { updateSettings } from "../stores/settings";
import { toast } from "../stores/toasts";
import { useLidarrReady } from "./lidarrShared";
import "./doctor.css";

const LidarrRequestDialog = lazy(() => import("./LidarrRequest").then((m) => ({ default: m.LidarrRequestDialog })));

/** Section headings, in page order. */
export const KIND_LABEL: Record<DoctorKind, string> = {
  duplicate: "Duplicate files",
  cover: "Covers",
  albumTags: "Album tags that differ",
  splitAlbum: "Albums split in two",
  albumArtist: "Missing album artist",
  lowQuality: "Low-quality copies",
  unmatched: "Albums that weren't matched",
  soundsLike: "Songs that sound like another recording",
};

export const KIND_ORDER: DoctorKind[] = ["duplicate", "cover", "albumTags", "splitAlbum", "albumArtist", "lowQuality", "unmatched", "soundsLike"];

const n = (v: number) => v.toLocaleString();

/** "3 things to fix", "Nothing to fix". */
export function countLine(count: number): string {
  if (count === 0) return "Nothing to fix";
  return count === 1 ? "1 thing to fix" : `${n(count)} things to fix`;
}

/** What the fix button says, and whether it needs editing rights on the server. */
export function fixLabel(f: DoctorFinding): { label: string; edit: boolean } {
  switch (f.fix) {
    case "cover":
      return { label: "Change cover", edit: true };
    case "match":
      return { label: "Fix match", edit: true };
    case "mergeAlbums":
      return { label: f.others.length > 1 ? "Merge albums" : "Merge into one", edit: true };
    case "tags":
      return { label: "See the tags", edit: false };
    case "suggestedTags":
      return { label: "See suggested tags", edit: false };
    case "openAlbum":
      return { label: f.kind === "lowQuality" ? "Open the better copy" : "Open album", edit: false };
  }
}

function useReport() {
  const [report, setReport] = useState<DoctorReport | null>(null);
  const [error, setError] = useState<string | null>(null);
  const load = useCallback(() => {
    backend.doctorReport().then(
      (r) => {
        setReport(r);
        setError(null);
        useDoctor.getState().set(r.count, r.running);
      },
      (e) => setError(errText(e) || "Couldn't load the library doctor."),
    );
  }, []);
  useEffect(() => {
    load();
    // A fresh callback each time: an earlier mount's late unlisten must not drop this one.
    const again = () => load();
    const offs = [backend.listen("doctor://changed", again), backend.listen("library://changed", again)];
    return () => {
      for (const o of offs) void o.then((u) => u());
    };
  }, [load]);
  return { report, error, reload: load };
}

export function DoctorPage() {
  const { report, error, reload } = useReport();
  const [kind, setKind] = useState<DoctorKind | null>(null);
  const [showIgnored, setShowIgnored] = useState(false);
  const [merging, setMerging] = useState<DoctorFinding | null>(null);
  const [asking, setAsking] = useState<LidarrRequestInput | null>(null);
  const lidarr = useLidarrReady();

  const counts = useMemo(() => {
    const m = new Map<DoctorKind, number>();
    for (const f of report?.findings ?? []) m.set(f.kind, (m.get(f.kind) ?? 0) + 1);
    return m;
  }, [report]);
  // A filter for a kind that has nothing left shows everything again.
  const active = kind && counts.get(kind) ? kind : null;

  const ignore = async (f: DoctorFinding, on: boolean) => {
    try {
      await backend.doctorIgnore(f.id, f.kind, f.title, on);
      reload();
      if (on) toast.info("Ignored. It stays out of the count on every device");
    } catch (e) {
      toast.error(errText(e) || (on ? `Couldn't ignore ${f.title}. Try again` : `Couldn't bring back ${f.title}. Try again`));
    }
  };

  const checkNow = async () => {
    try {
      await backend.doctorRun();
      toast.info("Checking your library again. Findings update as it goes");
    } catch (e) {
      toast.error(errText(e) || "Couldn't start a check");
    }
  };

  const betterCopy = async (f: DoctorFinding) => {
    try {
      const picked = await backend.lidarrPick({ artist: f.artist, artistId: f.artistId, title: f.album, song: false });
      if (!picked.group) {
        toast.info(picked.reason ?? "There's no official release of this album for Lidarr to get");
        return;
      }
      setAsking({ releaseGroupId: picked.group.id, title: picked.group.title, artist: f.artist, artistId: f.artistId });
    } catch (e) {
      toast.error(errText(e) || "Couldn't look this album up. Try again");
    }
  };

  const findings = report?.findings ?? [];
  const shown = active ? findings.filter((f) => f.kind === active) : findings;

  return (
    <div className="page-pad doctor" data-testid="doctor-page">
      <header className="doctor__head">
        <div className="doctor__heading">
          <h1 className="page-title">Library doctor</h1>
          {report?.on && (
            <p className="doctor__sub" role="status">
              <span>{countLine(report.count)}</span>
              <span aria-hidden="true"> · </span>
              <span>{report.running ? "Checking your library…" : report.checkedAt ? `Checked ${timeAgo(report.checkedAt).toLowerCase()}` : "Not checked yet"}</span>
            </p>
          )}
        </div>
        {report?.on && (
          <Button variant="secondary" size="sm" icon={<RefreshCw size={16} aria-hidden="true" />} disabled={report.running} onClick={() => void checkNow()}>
            Check now
          </Button>
        )}
      </header>

      {error && (
        <p className="doctor__empty" role="alert">
          {error}
        </p>
      )}
      {!report && !error && <SkeletonLines lines={5} widths={["90%", "80%", "86%", "70%", "84%"]} gap={24} />}

      {report && !report.on && (
        <div className="doctor__off" role="status">
          <Stethoscope size={24} aria-hidden="true" />
          <p>The library doctor is off. Turn it on to check your library in the background for duplicate files, small covers, tags that don't agree and more.</p>
          <Button variant="primary" onClick={() => void updateSettings({ libraryDoctor: true })}>
            Turn on
          </Button>
        </div>
      )}

      {report?.on && (
        <>
          {report.tagsTotal > 0 && report.tagsRead < report.tagsTotal && (
            <p className="doctor__note">
              Some checks use your files' own tags, which are still being read: {n(report.tagsRead)} of {n(report.tagsTotal)} files so far.
            </p>
          )}
          {findings.length > 0 && (
            <ChipRow label="Show findings" className="doctor__chips" scroll>
              <button type="button" className={`chip ${active ? "" : "is-selected"}`} aria-pressed={!active} onClick={() => setKind(null)}>
                All
              </button>
              {KIND_ORDER.filter((k) => counts.get(k)).map((k) => (
                <button key={k} type="button" className={`chip ${active === k ? "is-selected" : ""}`} aria-pressed={active === k} onClick={() => setKind(active === k ? null : k)}>
                  {KIND_LABEL[k]} <span className="doctor__chip-count">{counts.get(k)}</span>
                </button>
              ))}
            </ChipRow>
          )}
          {findings.length === 0 && (
            <p className="doctor__empty" role="status">
              Nothing to fix. The library doctor checks again after each sync and when new tags are read.
            </p>
          )}
          {KIND_ORDER.filter((k) => shown.some((f) => f.kind === k)).map((k) => (
            <KindSection key={k} kind={k} items={shown.filter((f) => f.kind === k)} lidarr={lidarr} onIgnore={ignore} onMerge={setMerging} onBetterCopy={betterCopy} />
          ))}
          {report.ignored.length > 0 && (
            <section className="doctor__ignored" aria-label="Ignored">
              <Button variant="ghost" size="sm" aria-expanded={showIgnored} onClick={() => setShowIgnored((v) => !v)}>
                {showIgnored ? "Hide ignored" : `Show ignored (${n(report.ignored.length)})`}
              </Button>
              {showIgnored && (
                <ul className="doctor__list">
                  {report.ignored.map((f) => (
                    <li key={f.id} className="doctor-item doctor-item--ignored">
                      <Art path={f.thumb} size={40} />
                      <div className="doctor-item__body">
                        <span className="doctor-item__title truncate">{f.title}</span>
                        <span className="doctor-item__meta truncate">
                          {KIND_LABEL[f.kind]} · {f.album} · {f.artist}
                        </span>
                      </div>
                      <Button variant="ghost" size="sm" onClick={() => void ignore(f, false)}>
                        Show again
                      </Button>
                    </li>
                  ))}
                </ul>
              )}
            </section>
          )}
        </>
      )}

      {merging && <MergeDialog finding={merging} onClose={() => setMerging(null)} />}
      {asking && (
        <Suspense fallback={null}>
          <LidarrRequestDialog open input={asking} onClose={() => setAsking(null)} onDone={() => setAsking(null)} />
        </Suspense>
      )}
    </div>
  );
}

function KindSection({
  kind,
  items,
  lidarr,
  onIgnore,
  onMerge,
  onBetterCopy,
}: {
  kind: DoctorKind;
  items: DoctorFinding[];
  lidarr: boolean;
  onIgnore: (f: DoctorFinding, on: boolean) => void;
  onMerge: (f: DoctorFinding) => void;
  onBetterCopy: (f: DoctorFinding) => void;
}) {
  const id = useId();
  return (
    <section className="doctor__section" aria-labelledby={id}>
      <h2 id={id} className="doctor__section-title">
        {KIND_LABEL[kind]} <span className="doctor__section-count">{n(items.length)}</span>
      </h2>
      <ul className="doctor__list">
        {items.map((f) => (
          <FindingRow key={f.id} f={f} lidarr={lidarr} onIgnore={onIgnore} onMerge={onMerge} onBetterCopy={onBetterCopy} />
        ))}
      </ul>
    </section>
  );
}

function target(f: DoctorFinding): EditTarget {
  return { id: f.albumId, kind: "album", title: f.album, thumb: f.thumb, artistTitle: f.artist };
}

function FindingRow({
  f,
  lidarr,
  onIgnore,
  onMerge,
  onBetterCopy,
}: {
  f: DoctorFinding;
  lidarr: boolean;
  onIgnore: (f: DoctorFinding, on: boolean) => void;
  onMerge: (f: DoctorFinding) => void;
  onBetterCopy: (f: DoctorFinding) => void;
}) {
  const [why, setWhy] = useState(false);
  const whyId = useId();
  const canEdit = useCanEdit(f.albumId);
  // Until the server's editing rights are known, offer the fix itself.
  const rightsPending = usePlexEdit((s) => s.access === null);
  const { label, edit } = fixLabel(f);
  // Without editing rights the album is the next best place to go.
  const fixable = !edit || canEdit || rightsPending;

  const fix = () => {
    if (!fixable) return nav.go({ name: "album", id: f.albumId });
    switch (f.fix) {
      case "cover":
        return usePlexEdit.getState().open("cover", target(f));
      case "match":
        return usePlexEdit.getState().open("match", target(f));
      case "mergeAlbums":
        return onMerge(f);
      case "tags":
      case "suggestedTags":
        return setWhy(true);
      case "openAlbum":
        return nav.go({ name: "album", id: f.kind === "lowQuality" && f.others[0] ? f.others[0].id : f.albumId });
    }
  };

  return (
    <li className="doctor-item" data-kind={f.kind}>
      <Art path={f.thumb} size={56} />
      <div className="doctor-item__body">
        <h3 className="doctor-item__title">{f.title}</h3>
        <p className="doctor-item__meta">
          <button type="button" className="link doctor-item__link" onClick={() => nav.go({ name: "album", id: f.albumId })}>
            {f.album}
          </button>
          <span aria-hidden="true"> · </span>
          <button type="button" className="link doctor-item__link" onClick={() => nav.go({ name: "artist", id: f.artistId })}>
            {f.artist}
          </button>
        </p>
        <p className="doctor-item__text">{f.text}</p>
        <div className="doctor-item__actions">
          <Button variant="primary" size="sm" onClick={fix}>
            {fixable ? label : "Open album"}
          </Button>
          {f.kind === "lowQuality" && lidarr && (
            <Button variant="secondary" size="sm" onClick={() => onBetterCopy(f)}>
              Find a better copy
            </Button>
          )}
          <Button variant="ghost" size="sm" onClick={() => onIgnore(f, true)}>
            Ignore
          </Button>
          <button type="button" className="link doctor-item__why" aria-expanded={why} aria-controls={whyId} onClick={() => setWhy((v) => !v)}>
            Why?
          </button>
        </div>
        {why && (
          <div id={whyId} className="doctor-why">
            <ul className="doctor-why__list">
              {f.evidence.map((e) => (
                <li key={e}>{e}</li>
              ))}
            </ul>
            {f.table.length > 0 && <TagTable f={f} />}
            {f.suggest && <SuggestTable s={f.suggest} />}
            {!fixable && <p className="doctor-why__note">Editing is off in Settings, or this account can't edit this server, so the fix opens the album instead.</p>}
          </div>
        )}
      </div>
    </li>
  );
}

/** Each song's own tags; a value that differs from most songs stands out. */
function TagTable({ f }: { f: DoctorFinding }) {
  const most = (vals: (string | number | null)[]) => {
    const c = new Map<string, number>();
    for (const v of vals) c.set(String(v ?? ""), (c.get(String(v ?? "")) ?? 0) + 1);
    return [...c.entries()].sort((a, b) => b[1] - a[1])[0]?.[0] ?? "";
  };
  const albumMost = most(f.table.map((r) => r.album));
  const artistMost = most(f.table.map((r) => r.albumArtist));
  const yearMost = most(f.table.map((r) => r.year));
  const cell = (v: string | number | null, usual: string) => {
    const s = v === null || v === "" ? "" : String(v);
    return <td className={s !== usual ? "doctor-tags__odd" : undefined}>{s || <span className="doctor-tags__none">None</span>}</td>;
  };
  return (
    <table className="doctor-tags">
      <caption className="sr-only">Each song's tags on {f.album}</caption>
      <thead>
        <tr>
          <th scope="col">Song</th>
          <th scope="col">Album</th>
          <th scope="col">Album artist</th>
          <th scope="col">Year</th>
        </tr>
      </thead>
      <tbody>
        {f.table.map((r) => (
          <tr key={r.trackId}>
            <th scope="row" className="truncate">
              {r.title}
            </th>
            {cell(r.album, albumMost)}
            {cell(r.albumArtist, artistMost)}
            {cell(r.year, yearMost)}
          </tr>
        ))}
      </tbody>
    </table>
  );
}

/** A song's own tags next to the ones its sound suggests. Shown only; nothing is written. */
export function SuggestTable({ s }: { s: DoctorSuggestedTags }) {
  const rows: [string, string, string | null][] = [
    ["Title", s.title, s.newTitle],
    ["Artist", s.artist, s.newArtist],
    ["Album", s.album, s.newAlbum],
    ["MusicBrainz recording", "", s.recordingMbid],
  ];
  return (
    <>
      <table className="doctor-tags doctor-suggest">
        <caption className="sr-only">The file's tags and the ones its sound suggests</caption>
        <thead>
          <tr>
            <th scope="col">Tag</th>
            <th scope="col">Now</th>
            <th scope="col">Suggested ({s.score}% match)</th>
          </tr>
        </thead>
        <tbody>
          {rows.map(([name, now, next]) => (
            <tr key={name}>
              <th scope="row">{name}</th>
              <td>{now || <span className="doctor-tags__none">None</span>}</td>
              <td className={next && next !== now ? "doctor-tags__odd" : undefined}>{next || <span className="doctor-tags__none">None</span>}</td>
            </tr>
          ))}
        </tbody>
      </table>
      <p className="doctor-why__note">Platter doesn't change the file. Copy these into your tagger if they're right.</p>
    </>
  );
}

/** Confirm, then merge the parts of a split album into the biggest one. */
function MergeDialog({ finding, onClose }: { finding: DoctorFinding; onClose: () => void }) {
  const [saving, setSaving] = useState(false);
  const parts = finding.others.length + 1;
  const merge = async () => {
    setSaving(true);
    const done = await runEdit(
      "Couldn't merge the albums",
      () =>
        backend.plexEditMergeAlbums(
          finding.albumId,
          finding.others.map((o) => o.id),
        ),
      { path: finding.thumb, kind: "album" },
    );
    setSaving(false);
    if (done) onClose();
  };
  return (
    <Dialog open onClose={onClose} label={`Merge ${finding.album}`} className="confirm">
      <h2 className="confirm__title">Merge the {parts === 2 ? "two" : n(parts)} parts of {finding.album}?</h2>
      <p className="confirm__body">
        Their songs become one album, {finding.album} by {finding.artist}. You can undo this from the message that follows, or split the album again from Edit.
      </p>
      <div className="doctor-merge">
        {[{ id: finding.albumId, title: finding.album, thumb: finding.thumb }, ...finding.others].map((a) => (
          <div key={a.id} className="doctor-merge__part">
            <Art path={a.thumb} size={64} />
            <span className="truncate">{a.title}</span>
          </div>
        ))}
      </div>
      <div className="confirm__actions">
        <Button variant="ghost" onClick={onClose}>
          Cancel
        </Button>
        <Button variant="primary" disabled={saving} onClick={() => void merge()}>
          Merge
        </Button>
      </div>
    </Dialog>
  );
}
