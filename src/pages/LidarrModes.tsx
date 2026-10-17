import { useCallback, useEffect, useState } from "react";
import { Button } from "../components/Button";
import { Dialog } from "../components/Dialog";
import { Skeleton } from "../components/Skeleton";
import { backend } from "../lib/backend";
import type {
  BridgeInfo,
  AppError,
  LidarrArtistMode,
  LidarrArtistPrefs,
  LidarrChange,
  LidarrFixAction,
  LidarrModeSetting,
  LidarrProposal,
  LidarrRematchPlan,
  LidarrStaged,
  LidarrTarget,
  LidarrWork,
} from "../lib/types";
import { updateSettings } from "../stores/settings";
import { toast } from "../stores/toasts";
import "./lidarr-modes.css";

const errText = (e: unknown) => (e && typeof e === "object" && "message" in e ? String((e as AppError).message) : "");

/** Something changed in Lidarr: lists on the page load again. */
const CHANGED = "platter:lidarr-changed";
const changed = () => window.dispatchEvent(new Event(CHANGED));

/** Load now and whenever Lidarr changes from this page. */
function useReload(load: () => void) {
  useEffect(() => {
    load();
    window.addEventListener(CHANGED, load);
    return () => window.removeEventListener(CHANGED, load);
  }, [load]);
}

const when = (secs: number) => new Date(secs * 1000).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });

/** Shows a change to Lidarr with its exact before and after; makes it on yes. */
export function ChangeDialog({ proposal, onClose, onDone }: { proposal: LidarrProposal | null; onClose: () => void; onDone?: () => void }) {
  const [busy, setBusy] = useState(false);
  if (!proposal) return null;
  const make = async () => {
    setBusy(true);
    try {
      await backend.lidarrChangeApply(proposal.id);
      changed();
      toast.info("Done. It's in the changes log");
      onClose();
      onDone?.();
    } catch (e) {
      toast.error(errText(e) || "Lidarr didn't take the change");
    } finally {
      setBusy(false);
    }
  };
  return (
    <Dialog open onClose={onClose} label={proposal.what} className="confirm">
      <h2 className="confirm__title">{proposal.what}</h2>
      <dl className="lidarr-change">
        <dt>Now</dt>
        <dd>{proposal.before}</dd>
        <dt>After</dt>
        <dd>{proposal.after}</dd>
      </dl>
      <p className="confirm__body">
        {proposal.canPutBack
          ? "Platter changes only this in Lidarr, logs it, and can put it back from the changes log."
          : "Platter changes only this in Lidarr and logs it. To undo it later, change it back in Lidarr."}
      </p>
      <div className="confirm__actions">
        <Button variant="ghost" onClick={onClose}>
          Cancel
        </Button>
        <Button variant="primary" disabled={busy} onClick={() => void make()}>
          Make this change
        </Button>
      </div>
    </Dialog>
  );
}

/** "Fix this" on a setup check finding: shows the change, then makes it. */
export function FixButton({ action, onDone }: { action: LidarrFixAction; onDone: () => void }) {
  const [proposal, setProposal] = useState<LidarrProposal | null>(null);
  const [asking, setAsking] = useState(false);
  const [path, setPath] = useState("");
  const [plan, setPlan] = useState<LidarrRematchPlan | null>(null);
  const show = async (input: string | null) => {
    try {
      setProposal(await backend.lidarrChangePreview(action, input));
      setAsking(false);
    } catch (e) {
      toast.error(errText(e) || "Platter couldn't read Lidarr");
    }
  };
  if (action.kind === "rematch") {
    return (
      <div className="lidarr-finding__actions lidarr-rematch">
        {action.folders.slice(0, 5).map((f) => (
          <Button
            key={f}
            size="sm"
            onClick={() =>
              void backend.lidarrRematchPreview(f).then(setPlan, (e) => toast.error(errText(e) || "Platter couldn't read Lidarr"))
            }
          >
            Re-match {f.split(/[\\/]/).pop()}
          </Button>
        ))}
        <RematchDialog plan={plan} onClose={() => setPlan(null)} onDone={onDone} />
      </div>
    );
  }
  return (
    <div className="lidarr-finding__actions">
      <Button size="sm" onClick={() => (action.kind === "setRecycleBin" ? setAsking(true) : void show(null))}>
        Fix this
      </Button>
      <Dialog open={asking} onClose={() => setAsking(false)} label="Recycling bin folder" className="confirm">
        <h2 className="confirm__title">Where should Lidarr keep removed files?</h2>
        <p className="confirm__body">A folder as Lidarr sees it, outside your Plex music and every Lidarr root folder.</p>
        <input
          className="setting-text"
          value={path}
          placeholder="/data/lidarr-recycle"
          aria-label="Recycling bin folder"
          onChange={(e) => setPath(e.target.value)}
        />
        <div className="confirm__actions">
          <Button variant="ghost" onClick={() => setAsking(false)}>
            Cancel
          </Button>
          <Button variant="primary" disabled={!path.trim()} onClick={() => void show(path)}>
            Show the change
          </Button>
        </div>
      </Dialog>
      <ChangeDialog proposal={proposal} onClose={() => setProposal(null)} onDone={onDone} />
    </div>
  );
}

function RematchDialog({ plan, onClose, onDone }: { plan: LidarrRematchPlan | null; onClose: () => void; onDone: () => void }) {
  const [busy, setBusy] = useState(false);
  if (!plan) return null;
  const go = async () => {
    setBusy(true);
    try {
      await backend.lidarrRematchApply(plan.folder, plan.releaseId);
      changed();
      toast.info(`Lidarr matched ${plan.files.length} ${plan.files.length === 1 ? "file" : "files"} to ${plan.album}`);
      onClose();
      onDone();
    } catch (e) {
      toast.error(errText(e) || "The re-match didn't go through");
    } finally {
      setBusy(false);
    }
  };
  return (
    <Dialog open onClose={onClose} label={`Re-match ${plan.album}`} className="confirm">
      <h2 className="confirm__title">Re-match {plan.album || "this folder"}</h2>
      <p className="confirm__body">
        {plan.files.length
          ? `${plan.files.length} ${plan.files.length === 1 ? "file goes" : "files go"} to ${plan.album} (${plan.release}). Nothing is renamed, and Platter checks the album's files afterwards.`
          : "Nothing in this folder can be re-matched safely."}
      </p>
      {plan.editionNote && <p className="confirm__body">{plan.editionNote}</p>}
      {plan.kept.length > 0 && (
        <ul className="lidarr-kept">
          {plan.kept.map((k) => (
            <li key={k.path}>
              {k.path.split(/[\\/]/).pop()}: {k.why}
            </li>
          ))}
        </ul>
      )}
      <div className="confirm__actions">
        <Button variant="ghost" onClick={onClose}>
          Cancel
        </Button>
        <Button variant="primary" disabled={busy || !plan.files.length} onClick={() => void go()}>
          Re-match
        </Button>
      </div>
    </Dialog>
  );
}

/** Practice mode, who did the Lidarr work last, and a pause if there is one. */
export function LidarrWorkStatus() {
  const [work, setWork] = useState<LidarrWork | null>(null);
  const [bridges, setBridges] = useState<BridgeInfo[]>([]);
  const load = useCallback(() => {
    backend.lidarrWork().then(setWork, () => setWork(null));
    backend.bridgeOverview().then(
      (o) => setBridges(o.bridges),
      () => setBridges([]),
    );
  }, []);
  useReload(load);
  if (!work) return null;
  const c = work.catchUp;
  return (
    <div className="lidarr-work" aria-live="polite">
      {work.practice && work.practiceUntil && (
        <div className="lidarr-work__practice">
          <p>
            Practice mode until {when(work.practiceUntil)}: Platter writes down what it would add to your library or swap, without doing it. Check the changes
            log below.
          </p>
          <Button size="sm" variant="primary" onClick={() => void updateSettings({ lidarrPracticeUntil: 0 }).then(changed)}>
            Looks right, start for real
          </Button>
        </div>
      )}
      {!work.practice && (
        <Button size="sm" variant="ghost" onClick={() => void updateSettings({ lidarrPracticeUntil: Math.floor(Date.now() / 1000) + 7 * 86_400 }).then(changed)}>
          Practice for a week
        </Button>
      )}
      {c.paused && (
        <div className="lidarr-work__paused" role="alert">
          <p>{c.paused}</p>
          <Button size="sm" onClick={() => void backend.lidarrResumeProtected().then(changed)}>
            Resume
          </Button>
        </div>
      )}
      {c.gap && <p className="lidarr-work__note">{c.gap}</p>}
      <p className="lidarr-work__note">
        {c.lastDevice && c.lastAt ? `Lidarr work last done by ${c.lastDevice}, ${when(c.lastAt)}.` : "Platter hasn't caught up with Lidarr yet."}
      </p>
      {bridges.length > 0 && (
        <p className="lidarr-work__note" data-testid="lidarr-bridge-line">
          {bridges[0].online
            ? `${bridges[0].name} does this work while no computer has Platter open.`
            : `${bridges[0].name} is paired but hasn't been heard from lately, so this work waits for a computer with Platter open.`}
        </p>
      )}
    </div>
  );
}

const MODE_LABEL: Record<LidarrModeSetting, string> = { auto: "Auto", managed: "Managed", protected: "Protected" };

/** Every artist with its mode and why; switch either way (Managed asks twice). */
export function LidarrArtistsList() {
  const [modes, setModes] = useState<LidarrArtistMode[] | null>(null);
  const [prefs, setPrefs] = useState<Record<string, LidarrArtistPrefs>>({});
  const [managing, setManaging] = useState<{ artist: LidarrArtistMode; step: 1 | 2 } | null>(null);
  const [proposal, setProposal] = useState<LidarrProposal | null>(null);
  const load = useCallback(() => {
    backend.lidarrModes().then(setModes, (e) => {
      setModes([]);
      toast.error(errText(e) || "Platter couldn't read Lidarr");
    });
    backend.lidarrArtistChoices().then(setPrefs, () => {});
  }, []);
  useReload(load);
  if (!modes) return <Skeleton height={56} radius="card" />;
  if (!modes.length)
    return (
      <div className="lidarr-work__note" role="status">
        <p>No artists yet</p>
        <p>Artists show up here once Lidarr knows about them. Add one in Lidarr, or request an album from Platter.</p>
      </div>
    );
  const set = async (a: LidarrArtistMode, mode: LidarrModeSetting, confirmations = 0) => {
    if (mode === "managed" && confirmations < 2) {
      setManaging({ artist: a, step: 1 });
      return;
    }
    try {
      await backend.lidarrModeSet(a.key, a.name, mode, confirmations);
      changed();
    } catch (e) {
      toast.error(errText(e) || `Couldn't change how Platter treats ${a.name}`);
    }
  };
  const choose = async (a: LidarrArtistMode, p: Partial<LidarrArtistPrefs>) => {
    const base: Pick<LidarrArtistPrefs, "autoAdd" | "hunt" | "target"> = prefs[a.key] ?? { autoAdd: null, hunt: false, target: "lossless" };
    const cur = { ...base, ...p };
    try {
      const next = await backend.lidarrArtistChoicesSet(a.key, a.name, cur.autoAdd, cur.hunt, cur.target);
      setPrefs({ ...prefs, [a.key]: next });
    } catch (e) {
      toast.error(errText(e) || `Couldn't save the choices for ${a.name}`);
    }
  };
  return (
    <>
      <ul className="lidarr-artists">
        {modes.map((a) => {
          const p = prefs[a.key];
          return (
            <li key={a.key} className={`lidarr-artist lidarr-artist--${a.mode}`}>
              <div className="lidarr-artist__head">
                <span className="lidarr-artist__name">{a.name}</span>
                <select
                  className="setting-select"
                  aria-label={`Mode for ${a.name}`}
                  value={a.setting}
                  onChange={(e) => void set(a, e.target.value as LidarrModeSetting)}
                >
                  {(Object.keys(MODE_LABEL) as LidarrModeSetting[]).map((m) => (
                    <option key={m} value={m}>
                      {MODE_LABEL[m]}
                    </option>
                  ))}
                </select>
              </div>
              <p className="lidarr-artist__line">{a.line}</p>
              {a.mode === "protected" && (
                <div className="lidarr-artist__choices">
                  <label>
                    <input
                      type="checkbox"
                      checked={p?.autoAdd === true}
                      onChange={(e) => void choose(a, { autoAdd: e.target.checked ? true : null })}
                    />{" "}
                    Add new official albums automatically
                  </label>
                  <label>
                    <input type="checkbox" checked={!!p?.hunt} onChange={(e) => void choose(a, { hunt: e.target.checked })} /> Look for better copies
                  </label>
                  {p?.hunt && (
                    <select
                      className="setting-select"
                      aria-label={`Better copies for ${a.name}`}
                      value={p.target}
                      onChange={(e) => void choose(a, { target: e.target.value as LidarrTarget })}
                    >
                      <option value="lossless">Lossless 16-bit</option>
                      <option value="hiRes">Hi-res 24-bit</option>
                    </select>
                  )}
                  {p?.hunt && p.target === "hiRes" && (
                    <Button size="sm" variant="ghost" onClick={() => void backend.lidarrHiresPreview().then(setProposal, (e) => toast.info(errText(e)))}>
                      Add the Platter Hi-Res profile
                    </Button>
                  )}
                </div>
              )}
            </li>
          );
        })}
      </ul>
      <Dialog open={!!managing} onClose={() => setManaging(null)} label="Switch to Managed?" className="confirm">
        {managing && managing.step === 1 && (
          <>
            <h2 className="confirm__title">Let Lidarr manage {managing.artist.name}?</h2>
            <p className="confirm__body">
              Managed hands {managing.artist.name}'s folder to Lidarr: it imports new albums there, and can rename, retag and replace files with better
              copies. Curated files (unreleased, leaked, demos, your own tags) could change. {managing.artist.line}.
            </p>
            <div className="confirm__actions">
              <Button variant="ghost" onClick={() => setManaging(null)}>
                Keep it as it is
              </Button>
              <Button onClick={() => setManaging({ ...managing, step: 2 })}>Continue</Button>
            </div>
          </>
        )}
        {managing && managing.step === 2 && (
          <>
            <h2 className="confirm__title">Let Lidarr replace files for {managing.artist.name}?</h2>
            <p className="confirm__body">
              Platter stops protecting {managing.artist.name}. You can switch back to Protected later, but files Lidarr changed in the meantime stay changed.
            </p>
            <div className="confirm__actions">
              <Button variant="ghost" onClick={() => setManaging(null)}>
                Cancel
              </Button>
              <Button
                variant="primary"
                onClick={() => {
                  const a = managing.artist;
                  setManaging(null);
                  void set(a, "managed", 2);
                }}
              >
                Switch to Managed
              </Button>
            </div>
          </>
        )}
      </Dialog>
      <ChangeDialog proposal={proposal} onClose={() => setProposal(null)} />
    </>
  );
}

/** Protected albums in staging: waiting for you, ready to add, added. */
export function LidarrWaitingList() {
  const [list, setList] = useState<LidarrStaged[] | null>(null);
  const [always, setAlways] = useState<Record<string, boolean>>({});
  const [open, setOpen] = useState<string | null>(null);
  const [trashAsk, setTrashAsk] = useState<LidarrStaged | null>(null);
  const load = useCallback(() => {
    backend.lidarrWork().then(
      (w) => setList(w.staged.filter((s) => s.state !== "undone")),
      () => setList([]),
    );
  }, []);
  useReload(load);
  if (!list) return <Skeleton height={56} radius="card" />;
  if (!list.length) return <p className="lidarr-work__note">Nothing in the staging folder.</p>;
  const add = async (s: LidarrStaged) => {
    try {
      await backend.lidarrStagedAdd(s.id, !!always[s.id]);
      toast.info(`${s.title} is in your library`);
      changed();
    } catch (e) {
      toast.error(errText(e) || "Couldn't add it");
    }
  };
  const undo = async (s: LidarrStaged, trashOk: boolean) => {
    try {
      await backend.lidarrStagedUndo(s.id, trashOk);
      toast.info(`${s.title} is out of your library, in the backup folder`);
      setTrashAsk(null);
      changed();
    } catch (e) {
      const m = errText(e);
      if (!trashOk && m.includes("empties its trash")) setTrashAsk(s);
      else toast.error(m || "Couldn't take it out");
    }
  };
  return (
    <>
      <ul className="lidarr-staged">
        {list.map((s) => (
          <li key={s.id} className={`lidarr-staged__item lidarr-staged__item--${s.state}`}>
            <p className="lidarr-staged__title">
              {s.title} <span className="lidarr-staged__artist">by {s.artist}</span>
            </p>
            {s.reason && <p className="lidarr-staged__reason">{s.reason}</p>}
            {s.state === "added" && <p className="lidarr-staged__reason">In your library: {s.folder}</p>}
            {s.plan && s.state !== "added" && (
              <>
                <Button size="sm" variant="ghost" aria-expanded={open === s.id} onClick={() => setOpen(open === s.id ? null : s.id)}>
                  {open === s.id ? "Hide names and tags" : "Show names and tags"}
                </Button>
                {open === s.id && (
                  <div className="lidarr-staged__plan">
                    <p>Folder: {s.folder}</p>
                    <ul>
                      {s.plan.files.map((f) => (
                        <li key={f.src}>
                          <span className="lidarr-staged__file">{f.name}</span>
                          {f.edits.length > 0 && (
                            <span className="lidarr-staged__tags">{f.edits.map(([k, v]) => (v === null ? `${k} removed` : `${k}=${v}`)).join(", ")}</span>
                          )}
                        </li>
                      ))}
                    </ul>
                  </div>
                )}
              </>
            )}
            {(s.state === "ready" || s.state === "practice") && (
              <div className="lidarr-finding__actions">
                <label>
                  <input type="checkbox" checked={!!always[s.id]} onChange={(e) => setAlways({ ...always, [s.id]: e.target.checked })} /> Do this
                  automatically for {s.artist}
                </label>
                <Button size="sm" variant="primary" onClick={() => void add(s)}>
                  Add to library
                </Button>
              </div>
            )}
            {s.state === "added" && (
              <div className="lidarr-finding__actions">
                <Button size="sm" variant="ghost" onClick={() => void undo(s, false)}>
                  Undo
                </Button>
              </div>
            )}
          </li>
        ))}
      </ul>
      <Dialog open={!!trashAsk} onClose={() => setTrashAsk(null)} label="Keep Plex's trash for the move?" className="confirm">
        <h2 className="confirm__title">Keep Plex's trash for the move?</h2>
        <p className="confirm__body">
          Plex empties its trash after every scan, which would drop this album's play counts and ratings from Plex during the move. Platter can turn that
          off just for the move and back on right after.
        </p>
        <div className="confirm__actions">
          <Button variant="ghost" onClick={() => setTrashAsk(null)}>
            Cancel
          </Button>
          <Button variant="primary" onClick={() => trashAsk && void undo(trashAsk, true)}>
            Turn it off for the move
          </Button>
        </div>
      </Dialog>
    </>
  );
}

/** Every change Platter made to Lidarr, with "Put it back" where it can. */
export function LidarrChangesList() {
  const [list, setList] = useState<LidarrChange[] | null>(null);
  const [back, setBack] = useState<LidarrChange | null>(null);
  const load = useCallback(() => {
    backend.lidarrChanges().then(setList, () => setList([]));
  }, []);
  useReload(load);
  if (!list) return <Skeleton height={56} radius="card" />;
  if (!list.length) return <p className="lidarr-work__note">Platter hasn't changed anything in Lidarr.</p>;
  return (
    <>
      <ul className="lidarr-changes">
        {list.slice(0, 50).map((c) => (
          <li key={c.id} className={`lidarr-changes__item${c.practice ? " is-practice" : ""}`}>
            <p className="lidarr-changes__what">
              {c.practice && <span className="lidarr-changes__tag">Practice</span>}
              {c.what}
            </p>
            <p className="lidarr-changes__meta">
              {when(c.at)} on {c.device}
              {c.byLidarr ? ", made by Lidarr on its own" : c.asked ? "" : ", part of a request you made"}
              {c.putBackAt ? `, put back ${when(c.putBackAt)}` : ""}
            </p>
            {(c.before || c.after) && (
              <p className="lidarr-changes__diff">
                {c.before && <span>Before: {c.before}</span>}
                {c.after && <span>After: {c.after}</span>}
              </p>
            )}
            {c.undo !== null && !c.putBackAt && !c.practice && (
              <Button size="sm" variant="ghost" onClick={() => setBack(c)}>
                Put it back
              </Button>
            )}
          </li>
        ))}
      </ul>
      <Dialog open={!!back} onClose={() => setBack(null)} label="Put it back?" className="confirm">
        <h2 className="confirm__title">Put it back?</h2>
        {back && (
          <dl className="lidarr-change">
            <dt>Now</dt>
            <dd>{back.after}</dd>
            <dt>After</dt>
            <dd>{back.before}</dd>
          </dl>
        )}
        <div className="confirm__actions">
          <Button variant="ghost" onClick={() => setBack(null)}>
            Cancel
          </Button>
          <Button
            variant="primary"
            onClick={() => {
              const c = back;
              setBack(null);
              if (c)
                void backend.lidarrChangePutBack(c.id).then(
                  () => {
                    toast.info("Put back");
                    changed();
                  },
                  (e) => toast.error(errText(e) || "Couldn't put it back"),
                );
            }}
          >
            Put it back
          </Button>
        </div>
      </Dialog>
    </>
  );
}
