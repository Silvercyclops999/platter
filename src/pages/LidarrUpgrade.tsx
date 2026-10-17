/**
 * Lidarr upgrades in the UI: "Look for a better copy" in the album menu, the
 * offer card on the album page (which songs upgrade, which are kept and
 * why), the swap dialog, the Undo toast, and the Settings parts (the staging
 * folder, the music share mapping, the upgrades list and the bulk action).
 */
import { ArrowRight, CircleAlert, Clock, Download, FolderOpen, Undo2 } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { Button } from "../components/Button";
import { Dialog } from "../components/Dialog";
import { Select } from "../components/Select";
import { Skeleton } from "../components/Skeleton";
import { backend } from "../lib/backend";
import type { AlbumUpgrade, AppError, OfferSong, ShareMap, UpgradeCandidates, UpgradeConfirmed, UpgradeOffer, UpgradeRecord, UpgradeUnder } from "../lib/types";
import { nav } from "../stores/router";
import { updateSettings, useSettings } from "../stores/settings";
import { toast, useToasts } from "../stores/toasts";
import { STATE_LABEL } from "./lidarrShared";
import "./lidarr-request.css";
import "./lidarr-upgrade.css";

const errText = (e: unknown) => (e && typeof e === "object" && "message" in e ? String((e as AppError).message) : "");

type Ids = { title: string; desc: string };

/** The album's upgrade, and its offer once there is a record. */
export function useAlbumUpgrade(albumId: string | null) {
  const staging = useSettings((s) => s.settings?.lidarrStagingFolder ?? "");
  const home = useSettings((s) => s.settings?.lidarrUrl);
  const away = useSettings((s) => s.settings?.lidarrAwayUrl);
  const [info, setInfo] = useState<AlbumUpgrade | null>(null);
  const [offer, setOffer] = useState<UpgradeOffer | null>(null);
  const [tick, setTick] = useState(0);
  const reload = useCallback(() => setTick((t) => t + 1), []);
  useEffect(() => {
    if (!albumId || !staging.trim() || !(home || away)) {
      setInfo(null);
      setOffer(null);
      return;
    }
    let alive = true;
    backend.lidarrUpgradeAlbum(albumId).then(
      async (i) => {
        if (!alive) return;
        setInfo(i);
        if (!i.record) {
          setOffer(null);
          return;
        }
        try {
          const o = await backend.lidarrUpgradeOffer(i.record.id);
          if (alive) setOffer(o);
        } catch {
          if (alive) setOffer(null);
        }
      },
      () => alive && setInfo(null),
    );
    return () => {
      alive = false;
    };
  }, [albumId, staging, home, away, tick]);
  return { info, offer, reload };
}

/** "Look for a better copy" for the album menu, when it can run. */
export function upgradeMenuItems(info: AlbumUpgrade | null, open: () => void) {
  const r = info?.record;
  const busy = r && r.state !== "failed" && !(r.swaps.length && r.swaps.every((s) => s.undone));
  if (!info?.available || busy) return [];
  return [{ label: "Look for a better copy", onSelect: open }];
}

const fmtLen = (ms: number | null | undefined) => {
  if (!ms) return "";
  const s = Math.round(ms / 1000);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
};

/**
 * Look for a better copy: first the user confirms which MusicBrainz release
 * the album is (its official tracklist beside the album's songs), then
 * Platter asks Lidarr for it, into the staging folder.
 */
export function LookForBetterDialog({
  open,
  albumId,
  title,
  quality,
  confirmed,
  onClose,
  onDone,
}: {
  open: boolean;
  albumId: string;
  title: string;
  quality: string | null;
  confirmed: UpgradeConfirmed | null;
  onClose: () => void;
  onDone: () => void;
}) {
  const staging = useSettings((s) => s.settings?.lidarrStagingFolder ?? "");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [mine, setMine] = useState<UpgradeConfirmed | null>(confirmed);
  const [changing, setChanging] = useState(false);
  const [cands, setCands] = useState<UpgradeCandidates | null>(null);
  const [pick, setPick] = useState<string | null>(null);
  useEffect(() => {
    if (!open) return;
    setError(null);
    setMine(confirmed);
    setChanging(!confirmed);
  }, [open, confirmed]);
  const choosing = open && (changing || !mine);
  useEffect(() => {
    if (!choosing) return;
    let alive = true;
    setCands(null);
    backend.lidarrUpgradeCandidates(albumId).then(
      (c) => {
        if (!alive) return;
        setCands(c);
        setPick(c.confirmed?.releaseGroupId ?? c.candidates[0]?.group.id ?? null);
      },
      (e) => alive && setError(errText(e) || "MusicBrainz didn't answer. Try again in a minute."),
    );
    return () => {
      alive = false;
    };
  }, [choosing, albumId]);

  const confirm = async () => {
    if (!pick) return;
    setBusy(true);
    setError(null);
    try {
      setMine(await backend.lidarrUpgradeConfirm(albumId, pick));
      setChanging(false);
    } catch (e) {
      setError(errText(e) || "That couldn't be confirmed.");
    } finally {
      setBusy(false);
    }
  };
  const send = async () => {
    setBusy(true);
    setError(null);
    try {
      const r = await backend.lidarrUpgradeRequest(albumId);
      toast.info(r.state === "queued" ? `Saved. Platter asks for a better copy of ${title} when it can reach Lidarr` : `Lidarr is looking for a better copy of ${title}`);
      onDone();
    } catch (e) {
      setError(errText(e) || "That didn't go through. Try again.");
    } finally {
      setBusy(false);
    }
  };
  const chosen = cands?.candidates.find((c) => c.group.id === pick);
  return (
    <Dialog open={open} onClose={onClose} label={`Look for a better copy of ${title}`} className="confirm lidarr-request upgrade-dialog upgrade-dialog--wide">
      <h2 className="confirm__title">{choosing ? `Which release is ${title}?` : `Look for a better copy of ${title}?`}</h2>
      {choosing ? (
        <>
          <p className="confirm__body">
            Pick the official release this album is. Platter only ever swaps in songs from it, and checks each one first.
          </p>
          {!cands && !error && <Skeleton height={120} radius="card" />}
          {cands && !cands.candidates.length && (
            <p className="lidarr-request__error" role="alert">
              <CircleAlert size={16} aria-hidden="true" />
              <span>MusicBrainz has no official studio release by {cands.artist} with this title, so there's nothing to look for.</span>
            </p>
          )}
          {cands && cands.candidates.length > 0 && (
            <>
              <div className="upgrade-releases" role="radiogroup" aria-label="Releases">
                {cands.candidates.map((c) => (
                  <label key={c.group.id} className={`upgrade-release ${pick === c.group.id ? "is-picked" : ""}`}>
                    <input type="radio" name="upgrade-release" checked={pick === c.group.id} onChange={() => setPick(c.group.id)} />
                    <span className="upgrade-release__title">{c.group.title}</span>
                    <span className="upgrade-release__meta">
                      {[c.group.firstRelease.slice(0, 4), `${c.tracks.length} ${c.tracks.length === 1 ? "track" : "tracks"}`].filter(Boolean).join(" · ")}
                    </span>
                  </label>
                ))}
              </div>
              <div className="upgrade-compare">
                <div>
                  <h3 className="upgrade-card__list-title">Your album</h3>
                  <ol className="upgrade-compare__list">
                    {cands.songs.map((s, i) => (
                      <li key={i}>
                        <span className="truncate">{s.title}</span>
                        <span className="upgrade-compare__len">{fmtLen(s.durationMs)}</span>
                      </li>
                    ))}
                  </ol>
                </div>
                <div>
                  <h3 className="upgrade-card__list-title">Official tracklist</h3>
                  <ol className="upgrade-compare__list">
                    {chosen?.tracks.map((t, i) => (
                      <li key={i}>
                        <span className="truncate">{t.title}</span>
                        <span className="upgrade-compare__len">{fmtLen(t.lengthMs)}</span>
                      </li>
                    ))}
                  </ol>
                </div>
              </div>
            </>
          )}
        </>
      ) : (
        <>
          <p className="confirm__body">
            {quality ? `It's ${quality} now. ` : ""}Platter asks Lidarr for {mine?.title ?? title} in better quality, downloaded into the staging folder:
          </p>
          <p className="lidarr-request__path">{staging}</p>
          <p className="confirm__body lidarr-request__note">
            Nothing in your library changes until you swap. Then each song is checked first, and anything unreleased or unproven stays as it is.{" "}
            <button type="button" className="link" onClick={() => setChanging(true)}>
              Change the release
            </button>
          </p>
        </>
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
        {choosing ? (
          <Button variant="primary" disabled={busy || !pick} onClick={() => void confirm()}>
            {busy ? "Saving…" : "This is the album"}
          </Button>
        ) : (
          <Button variant="primary" disabled={busy} onClick={() => void send()}>
            {busy ? "Asking…" : "Look for it"}
          </Button>
        )}
      </div>
    </Dialog>
  );
}

function SongLine({ s, kept }: { s: OfferSong; kept?: boolean }) {
  const num = s.track ? `${s.disc > 1 ? `${s.disc}-` : ""}${s.track}` : "";
  return (
    <li className="upgrade-song">
      <span className="upgrade-song__num" aria-hidden="true">
        {num}
      </span>
      <span className="upgrade-song__title truncate">{s.title}</span>
      {kept ? (
        <span className="upgrade-song__reason">{s.reason}</span>
      ) : (
        <span className="upgrade-song__quality">
          {s.from}
          <ArrowRight size={12} aria-label="to" />
          {s.to}
        </span>
      )}
    </li>
  );
}

/** Say a swap happened, with Undo. */
function swappedToast(summary: string, id: string, onChanged: () => void) {
  useToasts.getState().push(
    {
      kind: "info",
      message: summary,
      action: {
        label: "Undo",
        run: () => {
          backend.lidarrUpgradeUndo(id).then(
            () => {
              toast.info("The old files are back");
              onChanged();
            },
            (e) => toast.error(errText(e) || "Couldn't undo that"),
          );
        },
      },
    },
    12000,
  );
}

/** The swap: pick the songs (each one, when the album needs asking), then swap. */
export function SwapDialog({ open, offer, onClose, onDone }: { open: boolean; offer: UpgradeOffer; onClose: () => void; onDone: () => void }) {
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    if (!open) return;
    setError(null);
    setPicked(new Set(offer.askEach ? [] : offer.upgrade.map((s) => s.trackId)));
  }, [open, offer]);
  const flip = (id: string) =>
    setPicked((p) => {
      const n = new Set(p);
      if (n.has(id)) n.delete(id);
      else n.add(id);
      return n;
    });
  const swap = async () => {
    setBusy(true);
    setError(null);
    try {
      const r = await backend.lidarrUpgradeSwap(offer.upgradeId, [...picked]);
      swappedToast(r.summary, offer.upgradeId, onDone);
      onDone();
    } catch (e) {
      setError(errText(e) || "The swap didn't happen. Every file is as it was.");
    } finally {
      setBusy(false);
    }
  };
  const n = picked.size;
  return (
    <Dialog open={open} onClose={onClose} label={`Swap in the better copy of ${offer.title}`} className="confirm lidarr-request upgrade-dialog">
      <h2 className="confirm__title">Swap in the better copy?</h2>
      {offer.askEach && offer.askReason && (
        <p className="upgrade-note">
          <CircleAlert size={16} aria-hidden="true" />
          <span>{offer.askReason} Tick each song to swap.</span>
        </p>
      )}
      <ul className="upgrade-picks" aria-label="Songs to swap">
        {offer.upgrade.map((s) => (
          <li key={s.trackId}>
            <label className="upgrade-pick">
              <input type="checkbox" checked={picked.has(s.trackId)} onChange={() => flip(s.trackId)} />
              <span className="upgrade-song__title truncate">{s.title}</span>
              <span className="upgrade-song__quality">
                {s.from}
                <ArrowRight size={12} aria-label="to" />
                {s.to}
              </span>
            </label>
          </li>
        ))}
      </ul>
      <p className="confirm__body lidarr-request__note">
        Each new file takes the old file's name and every one of its tags. The old files go to {offer.backupDir}, and Undo puts them back exactly.
        Then Plex rescans just this album's folder.
      </p>
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
        <Button variant="primary" disabled={busy || n === 0} onClick={() => void swap()}>
          {busy ? "Swapping…" : `Swap ${n} ${n === 1 ? "song" : "songs"}`}
        </Button>
      </div>
    </Dialog>
  );
}

/** The album page's card: where the upgrade stands, and the offer once the better copy is in. */
export function UpgradeCard({ upgrade, onChanged }: { upgrade: ReturnType<typeof useAlbumUpgrade>; onChanged: () => void }) {
  const { info, offer } = upgrade;
  const [swapping, setSwapping] = useState(false);
  const [busy, setBusy] = useState(false);
  const r = info?.record;
  if (!r || (r.swaps.length && r.swaps.every((s) => s.undone) && !offer?.failureMessage)) return null;
  if (!offer) return <Skeleton height={72} radius="card" />;

  const undo = async () => {
    setBusy(true);
    try {
      await backend.lidarrUpgradeUndo(r.id);
      toast.info("The old files are back");
      onChanged();
    } catch (e) {
      toast.error(errText(e) || "Couldn't undo that");
      onChanged();
    } finally {
      setBusy(false);
    }
  };
  const swapNow = async () => {
    setBusy(true);
    try {
      const res = await backend.lidarrUpgradeSwap(r.id, null);
      swappedToast(res.summary, r.id, onChanged);
    } catch (e) {
      toast.error(errText(e) || "The swap didn't happen. Every file is as it was");
    } finally {
      setBusy(false);
      onChanged();
    }
  };
  const clear = async () => {
    try {
      await backend.lidarrUpgradeClear(r.id);
      onChanged();
    } catch (e) {
      toast.error(errText(e) || "Couldn't clear that");
    }
  };

  const waiting = offer.status === "waiting";
  return (
    <section className="upgrade-card" aria-label="Better copy">
      <div className="upgrade-card__head">
        <div className="upgrade-card__text">
          <p className="upgrade-card__title">
            {offer.status === "swapped"
              ? offer.summary
              : offer.status === "ready"
                ? offer.summary
                : waiting
                  ? r.state === "failed"
                    ? "Lidarr couldn't get a better copy"
                    : "Looking for a better copy"
                  : "Better copy"}
          </p>
          {offer.message && <p className="upgrade-card__meta">{offer.message}</p>}
          {offer.status === "swapped" && offer.lastSwap && (
            <p className="upgrade-card__meta">
              <FolderOpen size={14} aria-hidden="true" /> Old files are in {offer.lastSwap.backupDir}
            </p>
          )}
          {offer.status === "ready" && (
            <p className="upgrade-card__meta">
              <FolderOpen size={14} aria-hidden="true" /> Old files go to {offer.backupDir}
            </p>
          )}
        </div>
        {waiting && (
          <span className={`lidarr-state lidarr-state--${r.state}`}>
            {r.state === "queued" ? <Clock size={14} aria-hidden="true" /> : <Download size={14} aria-hidden="true" />}
            {STATE_LABEL[r.state]}
          </span>
        )}
        <div className="upgrade-card__actions">
          {offer.status === "ready" &&
            (offer.ask || offer.askEach ? (
              <Button variant="primary" size="sm" onClick={() => setSwapping(true)}>
                Swap files…
              </Button>
            ) : (
              <Button variant="primary" size="sm" disabled={busy} onClick={() => void swapNow()}>
                Swap now
              </Button>
            ))}
          {offer.status === "needsMapping" && (
            <Button size="sm" onClick={() => nav.go({ name: "settings" })}>
              Open Settings
            </Button>
          )}
          {offer.status === "swapped" && (
            <Button size="sm" icon={<Undo2 size={14} />} disabled={busy} onClick={() => void undo()}>
              Undo
            </Button>
          )}
          {(r.state === "failed" || offer.status === "nothing") && (
            <Button size="sm" variant="ghost" onClick={() => void clear()}>
              Clear
            </Button>
          )}
        </div>
      </div>
      {offer.failureMessage && (
        <p className="lidarr-request__error upgrade-card__failure" role="alert">
          <CircleAlert size={16} aria-hidden="true" />
          <span>{offer.failureMessage}</span>
        </p>
      )}
      {(offer.status === "ready" || offer.status === "nothing") && (
        <div className="upgrade-card__lists">
          {offer.upgrade.length > 0 && (
            <div>
              <h3 className="upgrade-card__list-title">Upgrades</h3>
              <ul className="upgrade-songs">
                {offer.upgrade.map((s) => (
                  <SongLine key={s.trackId} s={s} />
                ))}
              </ul>
            </div>
          )}
          {offer.kept.length > 0 && (
            <div>
              <h3 className="upgrade-card__list-title">Kept as they are</h3>
              <ul className="upgrade-songs">
                {offer.kept.map((s) => (
                  <SongLine key={`${s.trackId}:${s.title}`} s={s} kept />
                ))}
              </ul>
            </div>
          )}
        </div>
      )}
      {offer.status === "ready" && offer.askEach && offer.askReason && <p className="upgrade-card__meta">{offer.askReason}</p>}
      <SwapDialog
        open={swapping}
        offer={offer}
        onClose={() => setSwapping(false)}
        onDone={() => {
          setSwapping(false);
          onChanged();
        }}
      />
    </section>
  );
}

// ---------------------------------------------------------------- Settings

/** Lidarr's staging folder, as Lidarr sees it; saved on blur or Enter. */
export function StagingFolderField({ ids }: { ids: Ids }) {
  const saved = useSettings((s) => s.settings?.lidarrStagingFolder);
  const [value, setValue] = useState(saved ?? "");
  useEffect(() => {
    if (saved !== undefined) setValue(saved);
  }, [saved]);
  if (saved === undefined) return <Skeleton width={220} height={32} />;
  const save = () => {
    const next = value.trim().replace(/(.)[\\/]+$/, "$1");
    setValue(next);
    if (next !== saved) void updateSettings({ lidarrStagingFolder: next });
  };
  return (
    <input
      className="setting-text"
      value={value}
      maxLength={400}
      placeholder="/downloads/upgrades"
      spellCheck={false}
      autoComplete="off"
      aria-labelledby={ids.title}
      aria-describedby={ids.desc}
      onChange={(e) => setValue(e.target.value)}
      onBlur={save}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          save();
        }
      }}
    />
  );
}

/** The prefix a provenance note starts with when it names an official source. */
export function OfficialPrefixField({ ids }: { ids: Ids }) {
  const saved = useSettings((s) => s.settings?.upgradeOfficialPrefix);
  const [value, setValue] = useState(saved ?? "");
  useEffect(() => {
    if (saved !== undefined) setValue(saved);
  }, [saved]);
  if (saved === undefined) return <Skeleton width={160} height={32} />;
  const save = () => {
    const next = value.trim();
    if (next !== saved) void updateSettings({ upgradeOfficialPrefix: next });
  };
  return (
    <input
      className="setting-text"
      value={value}
      maxLength={60}
      placeholder="Official"
      spellCheck={false}
      autoComplete="off"
      aria-labelledby={ids.title}
      aria-describedby={ids.desc}
      onChange={(e) => setValue(e.target.value)}
      onBlur={save}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          save();
        }
      }}
    />
  );
}

/** Where this computer reaches the music share, the staging folder and the backup folder. */
export function ShareMapEditor() {
  const [map, setMap] = useState<ShareMap | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  useEffect(() => {
    let alive = true;
    backend.musicShareGet().then(
      async (m) => {
        if (!alive) return;
        if (!m.music.length) {
          const folders = await backend.musicSharePlexFolders().catch(() => [] as string[]);
          m = { ...m, music: folders.map((server) => ({ server, local: "" })) };
        }
        if (!m.music.length) m = { ...m, music: [{ server: "", local: "" }] };
        if (alive) setMap(m);
      },
      () => alive && setMap({ music: [{ server: "", local: "" }], staging: "", backup: "" }),
    );
    return () => {
      alive = false;
    };
  }, []);
  if (!map) return <Skeleton height={96} radius="card" />;
  const edit = (patch: Partial<ShareMap>) => {
    setSaved(false);
    setMap({ ...map, ...patch });
  };
  const save = async () => {
    setError(null);
    try {
      const m = await backend.musicShareSet(map);
      setMap(m.music.length ? m : { ...m, music: [{ server: "", local: "" }] });
      setSaved(true);
    } catch (e) {
      setError(errText(e) || "Couldn't save the music folders");
    }
  };
  return (
    <form
      className="share-map"
      onSubmit={(e) => {
        e.preventDefault();
        void save();
      }}
    >
      {map.music.map((p, i) => (
        <div key={i} className="share-map__row">
          <label className="share-map__field">
            <span>Plex's music folder</span>
            <input
              className="setting-text"
              value={p.server}
              placeholder="/data/music"
              spellCheck={false}
              onChange={(e) => edit({ music: map.music.map((x, j) => (j === i ? { ...x, server: e.target.value } : x)) })}
            />
          </label>
          <ArrowRight size={16} aria-hidden="true" className="share-map__arrow" />
          <label className="share-map__field">
            <span>On this computer</span>
            <input
              className="setting-text"
              value={p.local}
              placeholder="M:\\"
              spellCheck={false}
              onChange={(e) => edit({ music: map.music.map((x, j) => (j === i ? { ...x, local: e.target.value } : x)) })}
            />
          </label>
        </div>
      ))}
      <div className="share-map__row">
        <label className="share-map__field">
          <span>Staging folder on this computer</span>
          <input className="setting-text" value={map.staging} placeholder="U:\\" spellCheck={false} onChange={(e) => edit({ staging: e.target.value })} />
        </label>
        <label className="share-map__field">
          <span>Backup folder for replaced files</span>
          <input
            className="setting-text"
            value={map.backup}
            placeholder="Empty: _to_delete in your music folder"
            spellCheck={false}
            onChange={(e) => edit({ backup: e.target.value })}
          />
        </label>
      </div>
      {error && (
        <p className="lidarr-request__error" role="alert">
          <CircleAlert size={16} aria-hidden="true" />
          <span>{error}</span>
        </p>
      )}
      <div className="share-map__actions">
        <Button size="sm" type="submit">
          Save folders
        </Button>
        {saved && <span className="share-map__saved">Saved on this computer.</span>}
      </div>
    </form>
  );
}

const UNDER_OPTIONS: { value: UpgradeUnder; label: string }[] = [
  { value: "lossy", label: "MP3 and other lossy copies" },
  { value: "belowCd", label: "Anything below CD quality" },
];

/** Settings: upgrades on this computer, and the bulk action. */
export function UpgradesList() {
  const [list, setList] = useState<UpgradeRecord[] | null>(null);
  const [under, setUnder] = useState<UpgradeUnder>("lossy");
  const [asking, setAsking] = useState(false);
  const [busy, setBusy] = useState(false);
  const load = useCallback(() => {
    let alive = true;
    backend.lidarrUpgrades(true).then(
      (l) => alive && setList(l),
      () => alive && setList([]),
    );
    return () => {
      alive = false;
    };
  }, []);
  useEffect(load, [load]);
  const bulk = async () => {
    setBusy(true);
    try {
      const r = await backend.lidarrUpgradeBulk(under);
      const n = r.requested.length;
      toast.info(
        n === 0
          ? "No albums to look for."
          : `Lidarr is looking for better copies of ${n} ${n === 1 ? "album" : "albums"}${r.more ? `. ${r.more} more next time` : ""}`,
      );
      setAsking(false);
      load();
    } catch (e) {
      toast.error(errText(e) || "That didn't go through");
    } finally {
      setBusy(false);
    }
  };
  const clear = async (r: UpgradeRecord) => {
    try {
      setList(await backend.lidarrUpgradeClear(r.id));
    } catch (e) {
      toast.error(errText(e) || "Couldn't clear that");
    }
  };
  return (
    <div className="upgrades">
      <div className="upgrades__bulk">
        <Select
          value={under}
          options={UNDER_OPTIONS}
          onChange={(v) => setUnder(v as UpgradeUnder)}
          label="Albums to look for"
        />
        <Button size="sm" onClick={() => setAsking(true)}>
          Look for better copies
        </Button>
      </div>
      {!list ? (
        <Skeleton height={44} radius="card" />
      ) : !list.length ? (
        <p className="lidarr-requests__empty">No upgrades yet. An album's menu has Look for a better copy.</p>
      ) : (
        <ul className="lidarr-requests" aria-label="Upgrades">
          {list.map((r) => {
            const last = [...r.swaps].reverse().find((s) => !s.undone);
            return (
              <li key={r.id} className="lidarr-requests__item">
                <div className="lidarr-requests__text">
                  <p className="lidarr-requests__title">
                    <button type="button" className="link" onClick={() => nav.go({ name: "album", id: r.albumId })}>
                      {r.title}
                    </button>{" "}
                    <span className="lidarr-requests__artist">· {r.artist}</span>
                  </p>
                  <p className="lidarr-requests__meta">{last ? last.summary : `Now ${r.fromQuality}`}</p>
                  {last && <p className="lidarr-requests__meta">Old files are in {last.backupDir}</p>}
                  {r.state === "imported" && !last && <p className="lidarr-requests__meta">The better copy is in. Open the album to see what would change.</p>}
                  {r.message && <p className="lidarr-requests__meta">{r.message}</p>}
                </div>
                <span className={`lidarr-state lidarr-state--${last ? "imported" : r.state}`}>{last ? "Swapped" : r.state === "imported" ? "Ready to swap" : STATE_LABEL[r.state]}</span>
                {!last && (r.state === "imported" || r.state === "failed" || r.state === "queued") && (
                  <Button size="sm" variant="ghost" onClick={() => void clear(r)} aria-label={`Clear ${r.title}`}>
                    Clear
                  </Button>
                )}
              </li>
            );
          })}
        </ul>
      )}
      <Dialog open={asking} onClose={() => setAsking(false)} label="Look for better copies" className="confirm lidarr-request">
        <h2 className="confirm__title">Look for better copies?</h2>
        <p className="confirm__body">
          Platter asks Lidarr for the official release of each released album that's {under === "lossy" ? "MP3 or another lossy copy" : "below CD quality"}, up to 25
          at a time, into the staging folder. Each one runs the setup check first. Nothing in your library changes until you swap.
        </p>
        <div className="confirm__actions">
          <Button variant="ghost" onClick={() => setAsking(false)}>
            Cancel
          </Button>
          <Button variant="primary" disabled={busy} onClick={() => void bulk()}>
            {busy ? "Asking…" : "Look for them"}
          </Button>
        </div>
      </Dialog>
    </div>
  );
}
