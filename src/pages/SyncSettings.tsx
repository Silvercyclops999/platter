/**
 * Settings parts for the sync file (folder, state, Sync now) and for copying
 * keys between devices (the code to show, and the code to type).
 */
import { Cloud, FolderOpen } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { Button } from "../components/Button";
import { Dialog } from "../components/Dialog";
import { Switch } from "../components/Switch";
import { backend } from "../lib/backend";
import { timeAgo } from "../lib/relativeTime";
import type { AppError, CloudFolder, KeyKind, KeyReceiveState, Settings, SyncFileStatus } from "../lib/types";
import { loadSettings, updateSettings } from "../stores/settings";
import { toast } from "../stores/toasts";
import "./sync-settings.css";

const errText = (e: unknown) => (e && typeof e === "object" && "message" in e ? String((e as AppError).message) : "");

export const KEY_LABEL: Record<KeyKind, string> = {
  listenbrainz: "ListenBrainz token",
  lastfm: "Last.fm API key",
  fanart: "Fanart.tv API key",
  audiodb: "TheAudioDB API key",
  imgur: "Imgur client ID",
  catbox: "catbox.moe uploads turned on",
};
const KEY_ORDER: KeyKind[] = ["listenbrainz", "lastfm", "fanart", "audiodb", "imgur", "catbox"];

/** The sync file's state, kept current while Settings is open. */
export function useSyncFile() {
  const [status, setStatus] = useState<SyncFileStatus | null>(null);
  useEffect(() => {
    let alive = true;
    backend.syncFileStatus().then((s) => alive && setStatus(s), () => {});
    const off = backend.listen("sync-file://status", (s) => alive && setStatus(s));
    return () => {
      alive = false;
      void off.then((u) => u());
    };
  }, []);
  return [status, setStatus] as const;
}

/** One line about the sync file under the switch. */
export function syncNote(s: Settings | null, st: SyncFileStatus | null): string | undefined {
  if (!s?.syncEnabled) return undefined;
  if (!s.syncFolder) return "Choose a folder to sync with.";
  if (!st || st.state === "off") return undefined;
  if (st.state !== "ok") return st.message ?? "Couldn't sync.";
  const when = st.lastSyncedAt ? timeAgo(st.lastSyncedAt / 1000).toLowerCase() : "not yet";
  const others = st.devices.filter((d) => !d.thisDevice).map((d) => d.name);
  const with_ = others.length ? ` with ${others.join(", ")}` : "";
  return `Synced ${when}${with_}.`;
}

function FolderChoice({ open, onClose }: { open: boolean; onClose: () => void }) {
  const [found, setFound] = useState<CloudFolder[] | null>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    if (!open) return;
    setFound(null);
    backend.syncFileFolders().then(setFound, () => setFound([]));
  }, [open]);
  const done = (p: Promise<unknown>) => {
    setBusy(true);
    p.then(
      (r) => {
        setBusy(false);
        if (r !== null) {
          onClose();
          void loadSettings();
        }
      },
      (e) => {
        setBusy(false);
        toast.error(`Couldn't use that folder${errText(e) ? `: ${errText(e)}` : ""}`);
      },
    );
  };
  return (
    <Dialog open={open} onClose={onClose} label="Choose a folder to sync with" className="confirm sync-folders">
      <h2 className="confirm__title">Sync with a folder</h2>
      <p className="confirm__body">
        Pick a folder your cloud app already keeps in step, like OneDrive, iCloud Drive, Google Drive or Dropbox. Platter keeps one small file there, in a
        Platter folder. It never holds your Plex sign-in or any keys.
      </p>
      <div className="sync-folders__list" role="list" aria-label="Cloud folders on this computer">
        {found === null && <p className="sync-folders__none">Looking for cloud folders…</p>}
        {found?.length === 0 && <p className="sync-folders__none">No cloud folders found on this computer.</p>}
        {found?.map((f) => (
          <div role="listitem" key={f.path}>
            <button type="button" className="sync-folders__item" disabled={busy} onClick={() => done(backend.syncFileUseFolder(f.path))}>
              <Cloud size={20} aria-hidden="true" />
              <span className="sync-folders__text">
                <span className="sync-folders__name">{f.provider}</span>
                <span className="sync-folders__path">{f.path}</span>
              </span>
            </button>
          </div>
        ))}
      </div>
      <div className="confirm__actions">
        <Button variant="ghost" onClick={onClose}>
          Cancel
        </Button>
        <Button icon={<FolderOpen size={16} />} disabled={busy} onClick={() => done(backend.syncFileChooseFolder())}>
          Choose another folder
        </Button>
      </div>
    </Dialog>
  );
}

/** The Sync with a folder switch (asks for a folder the first time). */
export function SyncSwitch({ s, ids }: { s: Settings | null; ids: { title: string; desc: string } }) {
  const [choosing, setChoosing] = useState(false);
  if (!s) return null;
  return (
    <>
      <Switch
        checked={s.syncEnabled}
        labelledBy={ids.title}
        describedBy={ids.desc}
        onChange={(on) => {
          if (on && !s.syncFolder) setChoosing(true);
          else void updateSettings({ syncEnabled: on });
        }}
      />
      <FolderChoice open={choosing} onClose={() => setChoosing(false)} />
    </>
  );
}

export function ChangeSyncFolder() {
  const [choosing, setChoosing] = useState(false);
  return (
    <>
      <Button size="sm" onClick={() => setChoosing(true)}>
        Change folder
      </Button>
      <FolderChoice open={choosing} onClose={() => setChoosing(false)} />
    </>
  );
}

/** Sync now, or Start a new file when the old one can't be read. */
export function SyncNowControl({ status, onStatus }: { status: SyncFileStatus | null; onStatus: (s: SyncFileStatus) => void }) {
  const [busy, setBusy] = useState(false);
  const broken = status?.state === "unreadable";
  const run = () => {
    setBusy(true);
    (broken ? backend.syncFileReplace() : backend.syncFileNow()).then(
      (s) => {
        setBusy(false);
        onStatus(s);
      },
      (e) => {
        setBusy(false);
        toast.error(`Couldn't sync${errText(e) ? `: ${errText(e)}` : ""}`);
      },
    );
  };
  return (
    <Button size="sm" disabled={busy || !status || status.state === "off"} onClick={run}>
      {broken ? "Start a new file" : busy ? "Syncing…" : "Sync now"}
    </Button>
  );
}

// ---------------------------------------------------------------- keys

/** The QR code: dark modules as squares. */
function QrCode({ qr }: { qr: NonNullable<KeyReceiveState["qr"]> }) {
  const q = 2;
  const n = qr.size + q * 2;
  return (
    <svg className="keys-qr" viewBox={`0 0 ${n} ${n}`} role="img" aria-label="QR code for the code" shapeRendering="crispEdges">
      <rect width={n} height={n} fill="#fff" />
      {qr.rows.flatMap((row, y) =>
        [...row].map((c, x) => (c === "1" ? <rect key={`${x}-${y}`} x={x + q} y={y + q} width={1} height={1} fill="#000" /> : null)),
      )}
    </svg>
  );
}

const spaced = (code: string) => `${code.slice(0, 3)} ${code.slice(3)}`;

function useCountdown(until: number | null) {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    if (!until) return;
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, [until]);
  if (!until) return null;
  const left = Math.max(0, Math.ceil((until - now) / 1000));
  return `${Math.floor(left / 60)}:${String(left % 60).padStart(2, "0")}`;
}

const keyList = (kinds: KeyKind[]) => kinds.map((k) => KEY_LABEL[k]).join(", ");

/** "Copy keys from another device": shows a code for 5 minutes. */
export function ReceiveKeysDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const [st, setSt] = useState<KeyReceiveState | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const left = useCountdown(st?.phase === "waiting" ? st.expiresAt : null);
  const start = () => {
    setErr(null);
    backend.keysReceiveStart().then(setSt, (e) => setErr(errText(e) || "Couldn't show a code."));
  };
  useEffect(() => {
    if (!open) return;
    start();
    const off = backend.listen("keys://receive", setSt);
    return () => void off.then((u) => u());
  }, [open]);
  useEffect(() => {
    if (left === "0:00") backend.keysReceiveState().then(setSt, () => {});
  }, [left]);
  const close = () => {
    if (st?.phase === "waiting") void backend.keysReceiveCancel();
    setSt(null);
    onClose();
  };
  return (
    <Dialog open={open} onClose={close} label="Copy keys from another device" className="confirm keys-dialog">
      <h2 className="confirm__title">Copy keys from another device</h2>
      {err ? (
        <p className="confirm__body" role="alert">
          {err}
        </p>
      ) : st?.phase === "received" ? (
        <p className="confirm__body" role="status">
          {st.received.length
            ? `Got ${st.received.length === 1 ? "1 key" : `${st.received.length} keys`} from ${st.from ?? "your other device"}: ${keyList(st.received)}. They're in this computer's keychain.`
            : `${st.from ?? "Your other device"} sent nothing this computer could keep.`}
        </p>
      ) : st?.phase === "expired" || st?.phase === "locked" ? (
        <p className="confirm__body" role="status">
          {st.phase === "expired" ? "That code ran out." : "Too many wrong codes were tried, so that code stopped working."} Get a new code to try again.
        </p>
      ) : (
        <>
          <p className="confirm__body">
            On a computer that has your keys and is signed in to the same Plex account, open Settings and choose Send keys to a device. Type this code
            there. Both computers need to be on the same network.
          </p>
          <div className="keys-code">
            {st?.qr && <QrCode qr={st.qr} />}
            <div>
              <p className="keys-code__digits" aria-label={st?.code ? `Code ${[...st.code].join(" ")}` : "Getting a code"}>
                {st?.code ? spaced(st.code) : "··· ···"}
              </p>
              <p className="keys-code__left">{left ? `Works for ${left}` : " "}</p>
            </div>
          </div>
        </>
      )}
      <div className="confirm__actions">
        {(st?.phase === "expired" || st?.phase === "locked") && (
          <Button variant="secondary" onClick={start}>
            New code
          </Button>
        )}
        <Button variant={st?.phase === "received" ? "primary" : "ghost"} onClick={close}>
          {st?.phase === "received" ? "Done" : "Cancel"}
        </Button>
      </div>
    </Dialog>
  );
}

/** "Send keys to a device": type the other device's code, tick the keys. */
export function SendKeysDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const [have, setHave] = useState<KeyKind[] | null>(null);
  const [picked, setPicked] = useState<KeyKind[]>([]);
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [done, setDone] = useState<string | null>(null);
  const field = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (!open) return;
    setCode("");
    setErr(null);
    setDone(null);
    setHave(null);
    backend.keysAvailable().then(
      (k) => {
        setHave(k);
        setPicked(k);
      },
      () => setHave([]),
    );
  }, [open]);
  const digits = code.replace(/\D/g, "");
  const send = () => {
    setBusy(true);
    setErr(null);
    backend.keysSend(digits, picked).then(
      (o) => {
        setBusy(false);
        setDone(`Sent ${o.sent.length === 1 ? "1 key" : `${o.sent.length} keys`} to ${o.device}.`);
      },
      (e) => {
        setBusy(false);
        setErr(errText(e) || "Couldn't send the keys.");
      },
    );
  };
  return (
    <Dialog open={open} onClose={onClose} label="Send keys to a device" className="confirm keys-dialog" initialFocus={field}>
      <h2 className="confirm__title">Send keys to a device</h2>
      {done ? (
        <p className="confirm__body" role="status">
          {done}
        </p>
      ) : (
        <form
          onSubmit={(e) => {
            e.preventDefault();
            if (digits.length === 6 && picked.length && !busy) send();
          }}
        >
          <p className="confirm__body">On the other computer, open Settings and choose Copy keys from another device. Type the code it shows.</p>
          <label className="keys-send__label" htmlFor="keys-send-code">
            Code
          </label>
          <input
            id="keys-send-code"
            ref={field}
            className="keys-send__code"
            inputMode="numeric"
            autoComplete="off"
            maxLength={7}
            placeholder="000 000"
            value={code}
            onChange={(e) => setCode(e.target.value)}
          />
          <fieldset className="keys-send__keys">
            <legend className="keys-send__label">Keys to send</legend>
            {have === null && <p className="keys-send__none">Checking this computer's keys…</p>}
            {have?.length === 0 && <p className="keys-send__none">This computer has no keys to send.</p>}
            {KEY_ORDER.filter((k) => have?.includes(k)).map((k) => (
              <label key={k} className="keys-send__key">
                <input
                  type="checkbox"
                  checked={picked.includes(k)}
                  onChange={(e) => setPicked((p) => (e.target.checked ? [...p, k] : p.filter((x) => x !== k)))}
                />
                {KEY_LABEL[k]}
              </label>
            ))}
          </fieldset>
          {err && (
            <p className="keys-send__error" role="alert">
              {err}
            </p>
          )}
          <div className="confirm__actions">
            <Button variant="ghost" onClick={onClose}>
              Cancel
            </Button>
            <Button type="submit" variant="primary" disabled={busy || digits.length !== 6 || picked.length === 0}>
              {busy ? "Sending…" : "Send"}
            </Button>
          </div>
        </form>
      )}
      {done && (
        <div className="confirm__actions">
          <Button variant="primary" onClick={onClose}>
            Done
          </Button>
        </div>
      )}
    </Dialog>
  );
}

export function KeyCopyButton({ kind }: { kind: "receive" | "send" }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <Button size="sm" onClick={() => setOpen(true)}>
        {kind === "receive" ? "Show a code" : "Enter a code"}
      </Button>
      {kind === "receive" ? <ReceiveKeysDialog open={open} onClose={() => setOpen(false)} /> : <SendKeysDialog open={open} onClose={() => setOpen(false)} />}
    </>
  );
}
