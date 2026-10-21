import { CircleCheck, OctagonX, TriangleAlert } from "lucide-react";
import { HelpButton } from "../help/HelpButton";
import { useCallback, useEffect, useState } from "react";
import { Button } from "../components/Button";
import { Dialog } from "../components/Dialog";
import { Skeleton } from "../components/Skeleton";
import { backend } from "../lib/backend";
import type { AppError, LastGet, LidarrFinding, LidarrReport, LidarrStatus } from "../lib/types";
import { updateSettings, useSettings } from "../stores/settings";
import { toast } from "../stores/toasts";
import "./artist-info-settings.css";
import { FixButton } from "./LidarrModes";
import "./lidarr-settings.css";
import { failText } from "../lib/errorText";

type Ids = { title: string; desc: string };

const errText = (e: unknown) => (e && typeof e === "object" && "message" in e ? String((e as AppError).message) : "");

/**
 * Lidarr's connection state and setup check for the Settings rows. The
 * check result lives here, so it stays while the page is open and goes
 * away when an address or the key changes.
 */
export function useLidarr() {
  const home = useSettings((s) => s.settings?.lidarrUrl);
  const away = useSettings((s) => s.settings?.lidarrAwayUrl);
  const [status, setStatus] = useState<LidarrStatus | null>(null);
  const [report, setReport] = useState<LidarrReport | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [checking, setChecking] = useState(false);

  useEffect(() => {
    let alive = true;
    backend.lidarrStatus().then(
      (s) => alive && setStatus(s),
      () => {},
    );
    setReport(null);
    setError(null);
    return () => {
      alive = false;
    };
  }, [home, away]);

  const check = useCallback(async () => {
    setChecking(true);
    setError(null);
    try {
      setReport(await backend.lidarrCheck());
    } catch (e) {
      setReport(null);
      setError(errText(e) || "The check didn't finish. Try again.");
    } finally {
      setChecking(false);
    }
  }, []);

  const apply = (s: LidarrStatus) => {
    setStatus(s);
    setReport(null);
    setError(null);
  };

  const setKey = async (key: string | null) => {
    try {
      apply(await backend.lidarrKeySet(key));
      return true;
    } catch (e) {
      toast.error(failText(`Couldn't save the key`, e));
      return false;
    }
  };

  const setHeaders = async (text: string | null) => {
    try {
      apply(await backend.lidarrHeadersSet(text));
      return true;
    } catch (e) {
      toast.error(failText(`Couldn't save the headers`, e));
      return false;
    }
  };

  const confirmRoot = async (path: string, confirmed: boolean) => {
    try {
      await backend.lidarrConfirmRoot(path, confirmed);
      await check();
    } catch (e) {
      toast.error(errText(e) || `Couldn't save your answer for ${path}`);
    }
  };

  // Set up here, or a computer at home or a Bridge answers for this device.
  const ready = !!status && (((status.urlSet || status.awaySet) && status.hasKey && !status.moved) || !!status.homeSide);
  return { status, report, error, checking, ready, check, setKey, setHeaders, confirmRoot };
}

export type LidarrState = ReturnType<typeof useLidarr>;

/** Lidarr's home or away address, saved on blur or Enter. */
export function LidarrUrlField({ field, placeholder, ids }: { field: "lidarrUrl" | "lidarrAwayUrl"; placeholder: string; ids: Ids }) {
  const saved = useSettings((s) => s.settings?.[field]);
  const [value, setValue] = useState(saved ?? "");
  useEffect(() => {
    if (saved !== undefined) setValue(saved);
  }, [saved]);
  if (saved === undefined) return <Skeleton width={220} height={32} />;
  const save = () => {
    const next = value.trim().replace(/\/+$/, "");
    setValue(next);
    if (next !== saved) void updateSettings({ [field]: next });
  };
  return (
    <input
      type="url"
      className="setting-text"
      value={value}
      maxLength={300}
      placeholder={placeholder}
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

/** The API key: a field until one is saved, then "Remove key". Never shown again. */
export function LidarrKeyControl({ lidarr, ids }: { lidarr: LidarrState; ids: Ids }) {
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  if (!lidarr.status) return <Skeleton width={160} height={32} radius="pill" />;
  if (lidarr.status.hasKey && !lidarr.status.moved) {
    return (
      <Button size="sm" onClick={() => void lidarr.setKey(null)}>
        Remove key
      </Button>
    );
  }
  return (
    <>
    {lidarr.status.moved && <p className="setting-note" role="status">{lidarr.status.moved}</p>}
    <form
      className="info-key"
      onSubmit={(e) => {
        e.preventDefault();
        if (!text.trim()) return;
        setBusy(true);
        void lidarr.setKey(text.trim()).then((ok) => {
          setBusy(false);
          if (ok) setText("");
        });
      }}
    >
      <input
        type="password"
        className="info-key__field"
        value={text}
        onChange={(e) => setText(e.target.value)}
        placeholder="API key"
        autoComplete="off"
        spellCheck={false}
        aria-label="Lidarr key"
        aria-describedby={ids.desc}
      />
      <Button size="sm" type="submit" disabled={busy || !text.trim()}>
        Save key
      </Button>
    </form>
    </>
  );
}

/** Extra headers for the away address: a field until saved, then "Remove headers". */
export function LidarrHeadersControl({ lidarr, ids }: { lidarr: LidarrState; ids: Ids }) {
  const [text, setText] = useState("");
  if (!lidarr.status) return <Skeleton width={160} height={32} radius="pill" />;
  if (lidarr.status.headerNames.length) {
    return (
      <Button size="sm" onClick={() => void lidarr.setHeaders(null)}>
        Remove headers
      </Button>
    );
  }
  return (
    <form
      className="lidarr-headers"
      onSubmit={(e) => {
        e.preventDefault();
        if (!text.trim()) return;
        void lidarr.setHeaders(text).then((ok) => ok && setText(""));
      }}
    >
      <textarea
        className="lidarr-headers__field"
        value={text}
        rows={2}
        onChange={(e) => setText(e.target.value)}
        placeholder={"Name: value\nName: value"}
        autoComplete="off"
        spellCheck={false}
        aria-label="Extra sign-in details for the away address"
        aria-describedby={ids.desc}
      />
      <Button size="sm" type="submit" disabled={!text.trim()}>
        Save headers
      </Button>
    </form>
  );
}

/** "Check setup" (and "Check again"). */
export function LidarrCheckButton({ lidarr }: { lidarr: LidarrState }) {
  return (
    <Button size="sm" variant={lidarr.report || lidarr.error ? "secondary" : "primary"} disabled={lidarr.checking} onClick={() => void lidarr.check()}>
      {lidarr.checking ? "Checking…" : lidarr.report || lidarr.error ? "Check again" : "Check setup"}
    </Button>
  );
}

const ICON = {
  red: <OctagonX size={18} aria-hidden="true" />,
  amber: <TriangleAlert size={18} aria-hidden="true" />,
  ok: <CircleCheck size={18} aria-hidden="true" />,
};
const LEVEL_LABEL = { red: "Must fix", amber: "Worth changing", ok: "OK" };

function FindingRow({
  f,
  onConfirm,
  onUndo,
  onFixed,
}: {
  f: LidarrFinding;
  onConfirm: (path: string) => void;
  onUndo: (path: string) => void;
  onFixed: () => void;
}) {
  return (
    <li className={`lidarr-finding lidarr-finding--${f.level}`}>
      <span className="lidarr-finding__icon">{ICON[f.level]}</span>
      <div className="lidarr-finding__text">
        <p className="lidarr-finding__title">
          <span className="sr-only">{LEVEL_LABEL[f.level]}: </span>
          {f.title}
        </p>
        {f.detail && <p className="lidarr-finding__detail">{f.detail}</p>}
        {f.fix && <p className="lidarr-finding__fix">{f.fix}</p>}
        {f.action && <FixButton action={f.action} onDone={onFixed} />}
        {f.suggestedStaging && (
          <div className="lidarr-finding__actions">
            <Button
              size="sm"
              onClick={() =>
                void updateSettings({ lidarrStagingFolder: f.suggestedStaging })
                  .then(() => onFixed())
                  .catch((e: AppError) => toast.error(e.message))
              }
            >
              Use {f.suggestedStaging}
            </Button>
          </div>
        )}
        {f.emptyRoot && (
          <div className="lidarr-finding__actions">
            {f.emptyRoot.confirmed ? (
              <Button size="sm" variant="ghost" onClick={() => onUndo(f.emptyRoot!.path)}>
                Undo
              </Button>
            ) : (
              <Button size="sm" onClick={() => onConfirm(f.emptyRoot!.path)}>
                It's a different folder
              </Button>
            )}
          </div>
        )}
      </div>
    </li>
  );
}

/** The check's result under the row: an error, or the findings, red first. */
export function LidarrFindings({ lidarr }: { lidarr: LidarrState }) {
  const [confirming, setConfirming] = useState<string | null>(null);
  const { report, error, checking } = lidarr;
  if (checking && !report) {
    return (
      <div className="lidarr-check" aria-busy="true">
        <Skeleton height={56} radius="card" />
        <Skeleton height={56} radius="card" />
      </div>
    );
  }
  if (error) {
    return (
      <div className="lidarr-check">
        <p className="lidarr-check__error" role="alert">
          {error}
        </p>
      </div>
    );
  }
  if (!report) return null;
  const red = report.findings.filter((f) => f.level === "red");
  const amber = report.findings.filter((f) => f.level === "amber");
  const ok = report.findings.filter((f) => f.level === "ok");
  const where =
    report.route === "away"
      ? "its away address"
      : report.route === "bridge"
        ? "your Bridge"
        : report.route === "computer"
          ? "your computer at home"
          : "its home address";
  const row = (f: LidarrFinding) => (
    <FindingRow
      key={`${f.id}:${f.emptyRoot?.path ?? f.title}`}
      f={f}
      onConfirm={setConfirming}
      onUndo={(p) => void lidarr.confirmRoot(p, false)}
      onFixed={() => void lidarr.check()}
    />
  );
  return (
    <div className="lidarr-check" aria-live="polite">
      <div className="help-heading">
        <p className={`lidarr-check__summary ${report.passed ? "is-passed" : "is-blocked"}`}>
          {report.passed
            ? "Lidarr can't reach your curated music. Platter checks this again before every request."
            : report.managedOk
              ? "Managed artists keep working. Platter won't do anything for protected artists until the red items are fixed."
              : "Platter won't send Lidarr anything until the red items are fixed."}
        </p>
        <HelpButton page="lidarr" anchor="the-safety-check" label="What the check's findings mean" />
      </div>
      <p className="lidarr-check__meta">
        Lidarr {report.version} answered on {where}. Platter only read from it.
      </p>
      {red.length > 0 && <ul className="lidarr-findings">{red.map(row)}</ul>}
      {amber.length > 0 && (
        <>
          <p className="lidarr-check__note">
            Worth changing. They don't stop anything on their own.
          </p>
          <ul className="lidarr-findings">{amber.map(row)}</ul>
        </>
      )}
      {ok.length > 0 && <ul className="lidarr-findings lidarr-findings--ok">{ok.map(row)}</ul>}
      <Dialog open={!!confirming} onClose={() => setConfirming(null)} label="A different folder?" className="confirm">
        <h2 className="confirm__title">Is {confirming} a different folder?</h2>
        <p className="confirm__body">
          It's empty, so Platter can't compare it with your library. Only say yes if it isn't your music folder, isn't inside it, and doesn't hold
          it, even under another name (Lidarr and Plex often see the same folder by different paths). Platter checks it again before every request,
          and once Lidarr puts music in it, its contents decide.
        </p>
        <div className="confirm__actions">
          <Button variant="ghost" onClick={() => setConfirming(null)}>
            Cancel
          </Button>
          <Button
            variant="primary"
            onClick={() => {
              const p = confirming;
              setConfirming(null);
              if (p) void lidarr.confirmRoot(p, true);
            }}
          >
            It's a different folder
          </Button>
        </div>
      </Dialog>
    </div>
  );
}

const secs = (ms: number | null | undefined) => (ms == null ? null : `${Math.max(1, Math.round(ms / 1000))} s`);

/** "Last get: found 4 s, downloaded 31 s, playing at 38 s", from the latest tap that came through. */
export function lastGetText(g: LastGet | null): string | null {
  if (!g) return null;
  const t = g.timings;
  const parts = [
    secs(t.foundMs) && `found ${secs(t.foundMs)}`,
    secs(t.downloadedMs) && `downloaded ${secs(t.downloadedMs)}`,
    secs(t.playingMs) && `playing at ${secs(t.playingMs)}`,
  ].filter(Boolean);
  return parts.length ? `Last get: ${parts.join(", ")}.` : null;
}

export function LastGetLine() {
  const [g, setG] = useState<LastGet | null>(null);
  useEffect(() => {
    let live = true;
    backend
      .lidarrLastGet()
      .then((v) => live && setG(v))
      .catch(() => {});
    return () => {
      live = false;
    };
  }, []);
  const text = lastGetText(g);
  return text ? <span title={g?.title}>{text}</span> : null;
}
