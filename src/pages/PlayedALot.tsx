/**
 * "Played a lot, not in your library": songs and albums from imported
 * listening history with no match in the library, most played first, with
 * store searches, Request (through Lidarr's usual preview and confirm) and
 * Hide. "Request these" previews up to 10 albums and sends them after one
 * confirm. Rows leave on their own when the library gains the song.
 */
import { CircleAlert, Clock, Download } from "lucide-react";
import { Suspense, lazy, useCallback, useEffect, useId, useRef, useState } from "react";
import { Art } from "../components/Art";
import { Button } from "../components/Button";
import { Dialog } from "../components/Dialog";
import { Select, type SelectOption } from "../components/Select";
import { SkeletonLines } from "../components/Skeleton";
import { Switch } from "../components/Switch";
import { backend } from "../lib/backend";
import { STORE_LINKS } from "../lib/importStrings";
import type { PlayedHidden, PlayedKind, PlayedPage, PlayedRow } from "../lib/importTypes";
import type { LidarrPreview, LidarrRequestInput, LidarrRequestState, PlayedPeriod } from "../lib/types";
import { updateSettings, useSettings } from "../stores/settings";
import { toast } from "../stores/toasts";
import { upsertRequest } from "../stores/requests";
import { GET_UNDO_MS } from "./GetButton";
import { BringInCard, BringInSettings } from "./BringIn";
import { errText } from "./importShared";
import { STATE_LABEL, useLidarrReady } from "./lidarrShared";
import { openSettingsAt } from "./SoundMapPage";
import "./lidarr-request.css";
import { failText } from "../lib/errorText";
import { MoreText } from "./SettingMore";

const LidarrRequestDialog = lazy(() => import("./LidarrRequest").then((m) => ({ default: m.LidarrRequestDialog })));

/** Most albums one "Request these" sends (the core refuses more). */
export const BATCH_MAX = 10;

export const PERIOD_OPTIONS: readonly SelectOption<PlayedPeriod>[] = [
  { value: "all", label: "All time" },
  { value: "year", label: "Last year" },
  { value: "days90", label: "Last 90 days" },
];

export const MIN_PLAYS_OPTIONS: readonly SelectOption<string>[] = ["1", "2", "3", "5", "10", "20", "50"].map((v) => ({ value: v, label: `${v}+ ${v === "1" ? "play" : "plays"}` }));

const n = (v: number) => v.toLocaleString();

/** "3 hr 12 min", "45 min". */
export function playTime(ms: number): string {
  const m = Math.round(ms / 60_000);
  if (m < 60) return `${Math.max(m, 1)} min`;
  const h = Math.floor(m / 60);
  return m % 60 ? `${n(h)} hr ${m % 60} min` : `${n(h)} hr`;
}

function day(ms: number): string {
  return new Date(ms).toLocaleDateString(undefined, { month: "short", day: "numeric", year: "numeric" });
}

/** What to ask Lidarr for: an album row's album, or the album a song comes from. */
async function pick(r: PlayedRow): Promise<{ input: LidarrRequestInput } | { reason: string }> {
  const picked = await backend.lidarrPick({ artist: r.artist, title: r.title, song: r.kind === "song" });
  if (!picked.group) return { reason: picked.reason ?? "There's no official release of this for Lidarr to get." };
  return { input: { releaseGroupId: picked.group.id, title: picked.group.title, artist: r.artist } };
}

export function PlayedALot() {
  const s = useSettings((x) => x.settings);
  const minPlays = s?.playedMinPlays ?? 5;
  const period: PlayedPeriod = s?.playedPeriod ?? "all";
  const [kind, setKind] = useState<PlayedKind>("song");
  const [page, setPage] = useState<PlayedPage | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [more, setMore] = useState(false);
  const [states, setStates] = useState<Record<string, LidarrRequestState>>({});
  const [asking, setAsking] = useState<{ key: string; input: LidarrRequestInput } | null>(null);
  const [chosen, setChosen] = useState<PlayedRow[]>([]);
  const [batch, setBatch] = useState(false);
  const [hidden, setHidden] = useState<PlayedHidden[]>([]);
  const ready = useLidarrReady();
  const ids = useId();

  // Each load gets a number; only the newest one's answer (and a "Show
  // more" started after it) lands, so reloads can't overwrite each other.
  const seq = useRef(0);
  const load = useCallback(() => {
    const n = ++seq.current;
    backend.importPlayed({ kind, offset: 0, minPlays, period }).then(
      (p) => {
        if (n !== seq.current) return;
        setPage(p);
        setErr(null);
      },
      (e) => n === seq.current && setErr(errText(e) || "Couldn't load what you played"),
    );
    backend.importPlayedHidden().then((h) => n === seq.current && setHidden(h), () => {});
    return () => {
      // Left the page or the filters changed: drop what's still on its way.
      if (n === seq.current) seq.current++;
    };
  }, [kind, minPlays, period]);

  useEffect(load, [load]);
  // The library gained songs: their rows leave.
  useEffect(() => {
    const off = backend.listen("library://changed", () => load());
    return () => {
      void off.then((f) => f());
    };
  }, [load]);
  useEffect(() => setChosen([]), [kind, minPlays, period]);

  const showMore = async () => {
    if (!page) return;
    const n = seq.current;
    setMore(true);
    try {
      const next = await backend.importPlayed({ kind, offset: page.rows.length, minPlays, period });
      // A reload since then: its page is current, and this one would be stale on top of it.
      if (n !== seq.current) return;
      setPage((cur) => (cur === page ? { ...next, offset: 0, rows: [...page.rows, ...next.rows] } : cur));
    } catch (e) {
      if (n === seq.current) toast.error(errText(e) || "Couldn't load more songs");
    } finally {
      setMore(false);
    }
  };

  const open = (url: string) => backend.importOpenLink(url).catch((e) => toast.error(failText(`Couldn't open the link`, e)));

  // One-click Get: the album is picked, then sent with every check. The
  // dialog opens only for the first request ever, when a check says no (it
  // shows why), or when "Ask before each Get" is on.
  const request = async (r: PlayedRow) => {
    try {
      const p = await pick(r);
      if ("reason" in p) {
        toast.info(p.reason);
        return;
      }
      if (useSettings.getState().settings?.askBeforeGet) {
        setAsking({ key: r.key, input: p.input });
        return;
      }
      const out = await backend.lidarrGet(p.input);
      if (out.record) {
        const rec = out.record;
        upsertRequest(rec);
        setStates((m) => ({ ...m, [r.key]: rec.state }));
      } else {
        setAsking({ key: r.key, input: p.input });
      }
    } catch (e) {
      toast.error(errText(e) || "Couldn't look this up. Try again");
    }
  };

  const hide = (kindOf: PlayedHidden["kind"], r: PlayedRow, on = true) =>
    backend.importPlayedHide(kindOf, r.artist, kindOf === "artist" ? null : r.title, on).then(
      (h) => {
        setHidden(h);
        setChosen((c) => c.filter((x) => x.key !== r.key));
        load();
      },
      (e) => toast.error(errText(e) || "Couldn't hide that"),
    );
  const unhide = (h: PlayedHidden) =>
    backend.importPlayedHide(h.kind, h.artist, h.name || null, false).then(
      (list) => {
        setHidden(list);
        load();
      },
      (e) => toast.error(errText(e) || "Couldn't bring that back"),
    );

  const stateOf = (r: PlayedRow) => states[r.key] ?? r.request;
  const toggle = (r: PlayedRow, on: boolean) =>
    setChosen((c) => (on ? (c.length >= BATCH_MAX || c.some((x) => x.key === r.key) ? c : [...c, r]) : c.filter((x) => x.key !== r.key)));

  if (err)
    return (
      <p className="import-empty" role="alert">
        {err}
      </p>
    );

  return (
    <section className="played" aria-labelledby={`${ids}-h`}>
      <h2 id={`${ids}-h`} className="sr-only">
        Played a lot, not in your library
      </h2>
      <p className="import-page__sub">Songs and albums from your imported listening history that aren't in your library, the most played first.</p>
      {s?.bringIn && <BringInCard />}
      <div className="played__filters">
        <div className="chips" role="radiogroup" aria-label="Show">
          {(["song", "album"] as const).map((k) => (
            <button key={k} type="button" role="radio" aria-checked={kind === k} className={`chip ${kind === k ? "is-selected" : ""}`} onClick={() => setKind(k)}>
              {k === "song" ? "Songs" : "Albums"}
            </button>
          ))}
        </div>
        <Select label="Fewest plays" value={String(minPlays)} options={withValue(MIN_PLAYS_OPTIONS, String(minPlays))} onChange={(v) => void updateSettings({ playedMinPlays: Number(v) })} />
        <Select label="Period" value={period} options={PERIOD_OPTIONS} onChange={(v) => void updateSettings({ playedPeriod: v })} />
        {ready && chosen.length > 0 && (
          <Button size="sm" variant="primary" onClick={() => setBatch(true)}>
            Request these {chosen.length}
          </Button>
        )}
      </div>
      {!page && <SkeletonLines lines={6} widths={["90%", "85%", "88%", "80%", "86%", "70%"]} gap={20} />}
      {page && !page.hasHistory && (
        <div className="import-empty" role="status">
          <p>No listening history yet</p>
          <p>Import a history download and what you played a lot that isn't in your library shows up here.</p>
          <Button className="import-empty__go" onClick={() => openSettingsAt("#settings-library-import", { page: "libraryImport" })}>
            Import history
          </Button>
        </div>
      )}
      {page && page.hasHistory && page.rows.length === 0 && (
        <p className="import-empty" role="status">
          {`Everything you played ${minPlays === 1 ? "" : `${minPlays} times or more `}${period === "all" ? "" : period === "year" ? "in the last year " : "in the last 90 days "}is in your library.`}
        </p>
      )}
      {page && page.rows.length > 0 && (
        <>
          <p className="played__count" role="status">
            {n(page.total)} {kind === "song" ? (page.total === 1 ? "song" : "songs") : page.total === 1 ? "album" : "albums"}
            {ready && ` · Pick up to ${BATCH_MAX} to request together`}
          </p>
          <ul className="import-rows played__rows" aria-label={kind === "song" ? "Songs played a lot" : "Albums played a lot"}>
            {page.rows.map((r, i) => (
              <PlayedLine
                key={r.key}
                r={r}
                i={i}
                state={stateOf(r) ?? null}
                canRequest={ready}
                chosen={chosen.some((x) => x.key === r.key)}
                full={chosen.length >= BATCH_MAX}
                onChoose={(on) => toggle(r, on)}
                open={open}
                onRequest={() => request(r)}
                onHide={() => void hide(r.kind, r)}
                onHideArtist={() => void hide("artist", r)}
              />
            ))}
          </ul>
          {page.rows.length < page.total && (
            <Button className="played__more" variant="ghost" disabled={more} onClick={() => void showMore()}>
              {more ? "Loading…" : `Show more (${n(page.total - page.rows.length)} left)`}
            </Button>
          )}
        </>
      )}
      {hidden.length > 0 && <HiddenList hidden={hidden} onUnhide={(h) => void unhide(h)} />}
      {asking && (
        <Suspense fallback={null}>
          <LidarrRequestDialog
            open
            input={asking.input}
            onClose={() => setAsking(null)}
            onDone={(rec) => {
              setStates((m) => ({ ...m, [asking.key]: rec.state }));
              setAsking(null);
            }}
          />
        </Suspense>
      )}
      {batch && (
        <BatchDialog
          rows={chosen}
          onClose={() => setBatch(false)}
          onDone={(done) => {
            setStates((m) => ({ ...m, ...done }));
            setChosen([]);
            setBatch(false);
          }}
        />
      )}
    </section>
  );
}

/** Keep a stored value that isn't one of the choices selectable. */
function withValue(options: readonly SelectOption<string>[], v: string): readonly SelectOption<string>[] {
  return options.some((o) => o.value === v) ? options : [...options, { value: v, label: `${v}+ plays` }].sort((a, b) => Number(a.value) - Number(b.value));
}

function PlayedLine({
  r,
  i,
  state,
  canRequest,
  chosen,
  full,
  onChoose,
  open,
  onRequest,
  onHide,
  onHideArtist,
}: {
  r: PlayedRow;
  i: number;
  state: LidarrRequestState | null;
  canRequest: boolean;
  chosen: boolean;
  full: boolean;
  onChoose: (on: boolean) => void;
  open: (url: string) => unknown;
  onRequest: () => Promise<void>;
  onHide: () => void;
  onHideArtist: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const [undoable, setUndoable] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(
    () => () => {
      if (timer.current) clearTimeout(timer.current);
    },
    [],
  );
  const requested = state && state !== "failed";
  const get = () => {
    setUndoable(true);
    timer.current = setTimeout(() => {
      timer.current = null;
      setUndoable(false);
      setBusy(true);
      void onRequest().finally(() => setBusy(false));
    }, GET_UNDO_MS);
  };
  const undo = () => {
    if (timer.current) clearTimeout(timer.current);
    timer.current = null;
    setUndoable(false);
  };
  const detail =
    r.kind === "album"
      ? `${n(r.songs ?? 0)} ${r.songs === 1 ? "song" : "songs"} played${r.inLibrary ? ` · ${n(r.inLibrary)} already in your library` : ""}`
      : (r.album ?? "");
  return (
    <li className={`import-row missing-row played-row ${chosen ? "is-selected" : ""}`} aria-label={`${r.title} by ${r.artist}`}>
      <span className="import-row__n">
        {canRequest && !requested ? (
          <input
            type="checkbox"
            className="played-row__pick"
            checked={chosen}
            disabled={!chosen && full}
            aria-label={`Pick ${r.title} to request`}
            onChange={(e) => onChoose(e.target.checked)}
          />
        ) : (
          i + 1
        )}
      </span>
      <span className="import-row__song">
        <Art path={r.thumb} size={40} className="import-row__art" />
        <span className="import-row__main">
          <span className="import-row__title truncate">{r.title}</span>
          <span className="import-row__artist truncate">{r.artist}</span>
          <span className="missing-row__wants">
            {n(r.plays)} {r.plays === 1 ? "play" : "plays"} · {playTime(r.ms)} · last played {day(r.lastPlayed)}
          </span>
        </span>
      </span>
      {r.kind === "album" ? (
        <span className="import-row__album played-row__album" title={`${detail}. First played ${day(r.firstPlayed)}`}>
          <span className="truncate">
            {n(r.songs ?? 0)} {r.songs === 1 ? "song" : "songs"} played
          </span>
          {!!r.inLibrary && <span className="truncate">{n(r.inLibrary)} already in your library</span>}
        </span>
      ) : (
        <span className="import-row__album truncate">{detail}</span>
      )}
      <span className="missing-row__links">
        {STORE_LINKS.map((st) => (
          <Button key={st.key} size="sm" variant="ghost" onClick={() => void open(r.links[st.key])} aria-label={`Search ${st.name} for ${r.title}`}>
            {st.name}
          </Button>
        ))}
        {canRequest &&
          (requested ? (
            <span className={`lidarr-state lidarr-state--${state}`}>
              {state === "queued" ? <Clock size={14} aria-hidden="true" /> : <Download size={14} aria-hidden="true" />}
              {STATE_LABEL[state]}
            </span>
          ) : undoable ? (
            <>
              <span className="lidarr-state lidarr-state--checking" role="status">
                Checking
              </span>
              <Button size="sm" variant="ghost" onClick={undo} aria-label={`Undo Get ${r.title}`}>
                Undo
              </Button>
            </>
          ) : (
            <Button size="sm" variant="primary" disabled={busy} aria-label={`Get ${r.title}`} onClick={get}>
              {busy ? "Checking…" : "Get"}
            </Button>
          ))}
        <Button size="sm" variant="ghost" onClick={onHide} aria-label={`Hide ${r.title}`}>
          Hide
        </Button>
        <Button size="sm" variant="ghost" onClick={onHideArtist} aria-label={`Hide everything by ${r.artist}`}>
          Hide artist
        </Button>
      </span>
    </li>
  );
}

function HiddenList({ hidden, onUnhide }: { hidden: PlayedHidden[]; onUnhide: (h: PlayedHidden) => void }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="played__hidden">
      <Button size="sm" variant="ghost" aria-expanded={open} onClick={() => setOpen((o) => !o)}>
        {open ? "Hide the hidden list" : `${n(hidden.length)} hidden`}
      </Button>
      {open && (
        <ul className="played__hidden-list" aria-label="Hidden from Played a lot">
          {hidden.map((h) => (
            <li key={`${h.kind}:${h.artist}:${h.name}`}>
              <span className="truncate">{h.kind === "artist" ? `Everything by ${h.artist}` : `${h.name} by ${h.artist}`}</span>
              <Button size="sm" variant="ghost" onClick={() => onUnhide(h)}>
                Show again
              </Button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

type Planned = { row: PlayedRow; input?: LidarrRequestInput; preview?: LidarrPreview; reason?: string };

/** "Request these": a preview for each, then one confirm. */
function BatchDialog({ rows, onClose, onDone }: { rows: PlayedRow[]; onClose: () => void; onDone: (states: Record<string, LidarrRequestState>) => void }) {
  const [plans, setPlans] = useState<Planned[] | null>(null);
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    void (async () => {
      const out: Planned[] = [];
      const seen = new Set<string>();
      for (const row of rows.slice(0, BATCH_MAX)) {
        try {
          const p = await pick(row);
          if ("reason" in p) {
            out.push({ row, reason: p.reason });
            continue;
          }
          if (seen.has(p.input.releaseGroupId)) {
            out.push({ row, reason: `Same album as another pick (${p.input.title}).` });
            continue;
          }
          seen.add(p.input.releaseGroupId);
          const preview = await backend.lidarrRequestPreview(p.input);
          out.push({ row, input: p.input, preview, reason: preview.existing ? `Lidarr already has it (${STATE_LABEL[preview.existing].toLowerCase()}).` : undefined });
        } catch (e) {
          out.push({ row, reason: errText(e) || "Couldn't check this one." });
        }
      }
      if (alive) setPlans(out);
    })();
    return () => {
      alive = false;
    };
  }, [rows]);

  const sendable = (plans ?? []).filter((p) => p.input && p.preview && !p.reason);
  const needsTyped = sendable.some((p) => p.preview?.needsTyped);

  const send = async () => {
    setSending(true);
    setError(null);
    try {
      const items = await backend.lidarrRequestBatch(sendable.map((p) => ({ ...p.input!, title: p.preview!.title })));
      const states: Record<string, LidarrRequestState> = {};
      let sent = 0;
      for (const it of items) {
        const plan = sendable.find((p) => p.input!.releaseGroupId === it.releaseGroupId);
        if (plan && it.record) {
          states[plan.row.key] = it.record.state;
          sent++;
        }
      }
      const failed = items.filter((it) => !it.record);
      if (failed.length) toast.error(`${failed.length} not sent: ${failed.map((f) => `${f.title} (${f.error ?? "failed"})`).join("; ")}`);
      if (sent) toast.info(`Requested ${sent} ${sent === 1 ? "album" : "albums"} from Lidarr`);
      onDone(states);
    } catch (e) {
      setError(errText(e) || "The requests didn't go through. Try again.");
    } finally {
      setSending(false);
    }
  };

  return (
    <Dialog open onClose={onClose} label="Request these albums" className="confirm lidarr-request played-batch">
      <h2 className="confirm__title">Request {rows.length === 1 ? "this album" : `these ${rows.length}`}?</h2>
      {!plans && (
        <p className="confirm__body" aria-busy="true">
          Finding each album and checking Lidarr's setup…
        </p>
      )}
      {plans && (
        <ul className="played-batch__list" aria-label="What will be requested">
          {plans.map((p) => (
            <li key={p.row.key} className={p.reason ? "is-skipped" : ""}>
              <strong>{p.preview?.title ?? p.row.title}</strong> by {p.row.artist}
              <span className="played-batch__where">
                {p.reason ? `Skipped: ${p.reason}` : `${p.preview?.reachable ? "Into" : "Waits for Lidarr, then into"} ${p.preview?.destination ?? "Lidarr's download folder"}`}
              </span>
            </li>
          ))}
        </ul>
      )}
      {plans && (
        <p className="confirm__body lidarr-request__note">Only these albums are marked for download. Nothing in your library is renamed, retagged or moved.</p>
      )}
      {needsTyped && (
        <p className="lidarr-request__error" role="alert">
          <CircleAlert size={16} aria-hidden="true" />
          <span>Your first request needs its album's name typed. Request one album on its own first, then come back.</span>
        </p>
      )}
      {error && (
        <p className="lidarr-request__error" role="alert">
          <CircleAlert size={16} aria-hidden="true" />
          <span>{error}</span>
        </p>
      )}
      <div className="confirm__actions">
        <Button variant="ghost" onClick={onClose}>
          Cancel
        </Button>
        <Button variant="primary" disabled={!plans || !sendable.length || needsTyped || sending} onClick={() => void send()}>
          {sending ? "Sending…" : `Request ${sendable.length}`}
        </Button>
      </div>
    </Dialog>
  );
}

/** The "Played a lot" settings, inside Settings > Library import. */
export function PlayedSettings() {
  const s = useSettings((x) => x.settings);
  const id = useId();
  if (!s) return null;
  return (
    <>
    <div className="setting-row played-settings" data-setting="showPlayedALot">
      <div className="setting-row__text">
        <div className="setting-row__head">
          <div id={`${id}-t`} className="setting-row__title">
            Show "Played a lot" on the Missing page
          </div>
          <MoreText
            title='Show "Played a lot" on the Missing page'
            text="Songs and albums from imported listening history that aren't in your library, the most played first. Synced."
          />
        </div>
        <p id={`${id}-d`} className="setting-row__desc">
          Imported songs and albums you don't have, the most played first. Synced.
        </p>
      </div>
      <div className="setting-row__control">
        <Switch checked={s.showPlayedALot} onChange={(v) => void updateSettings({ showPlayedALot: v })} labelledBy={`${id}-t`} describedBy={`${id}-d`} />
      </div>
      {s.showPlayedALot && (
        <div className="setting-row__block played-settings__choices">
          <span>
            <Select
              label="Fewest plays"
              value={String(s.playedMinPlays)}
              options={withValue(MIN_PLAYS_OPTIONS, String(s.playedMinPlays))}
              onChange={(v) => void updateSettings({ playedMinPlays: Number(v) })}
            />
          </span>
          <span>
            <Select label="Period" value={s.playedPeriod} options={PERIOD_OPTIONS} onChange={(v) => void updateSettings({ playedPeriod: v })} />
          </span>
        </div>
      )}
    </div>
    {s.showPlayedALot && <BringInSettings />}
    </>
  );
}
