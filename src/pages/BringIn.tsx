/**
 * "Bring in my listening": the progress card on the Missing page and its
 * settings. The waves themselves run in the core, a few albums a day, each
 * through Lidarr's usual checks.
 */
import { useCallback, useEffect, useId, useRef, useState } from "react";
import { Button } from "../components/Button";
import { Select, type SelectOption } from "../components/Select";
import { Switch } from "../components/Switch";
import { backend } from "../lib/backend";
import type { BringInStatus } from "../lib/types";
import { updateSettings, useSettings } from "../stores/settings";
import { toast } from "../stores/toasts";
import { errText } from "./importShared";
import { MoreText } from "./SettingMore";

const n = (v: number) => v.toLocaleString();

/** How often the card reads progress while it's open. */
const REFRESH_MS = 60_000;

/** "Wave 1 of 3 · albums with 100+ plays". */
export function waveLine(s: BringInStatus): string {
  if (s.finished || s.threshold == null) return "Every wave is done";
  return `Wave ${s.wave + 1} of ${s.waves.length} · albums with ${n(s.threshold)}+ plays`;
}

/** What it's doing now, in one sentence. */
export function stateLine(s: BringInStatus): string {
  if (!s.ready)
    return "Waiting for Lidarr: set it up and run its setup check in Settings.";
  if (s.userPaused) return "Paused. Nothing is asked for until you carry on.";
  if (s.paused) return `Paused: ${s.paused}`;
  if (s.finished) return "Every album with enough plays has been dealt with.";
  if (s.today >= s.perDay)
    return `Today's ${n(s.perDay)} are done. More tomorrow.`;
  if (s.practice) return "Practice mode: albums are only listed, nothing is sent to Lidarr.";
  return s.bridge ? "Asking Lidarr for a few albums a day." : "Running while Platter is open.";
}

/** Where the waves run, when a Bridge has them. */
export function whereLine(s: BringInStatus): string | null {
  return s.bridge ? `Running on your Bridge (${s.bridge}), even while this computer is off.` : null;
}

export function BringInCard() {
  const [s, setS] = useState<BringInStatus | null>(null);
  const [open, setOpen] = useState(false);
  /** Bumped by every read and every action, so only the newest answer lands. */
  const seq = useRef(0);
  const load = useCallback(() => {
    const mine = ++seq.current;
    // A read that fails keeps what's shown.
    backend.bringInStatus().then(
      (v) => {
        if (mine === seq.current) setS(v);
      },
      () => {},
    );
  }, []);
  useEffect(() => {
    load();
    const t = setInterval(() => {
      if (!document.hidden) load();
    }, REFRESH_MS);
    const onShow = () => {
      if (!document.hidden) load();
    };
    document.addEventListener("visibilitychange", onShow);
    return () => {
      clearInterval(t);
      document.removeEventListener("visibilitychange", onShow);
    };
  }, [load]);
  if (!s || !s.on) return null;
  const act = (p: Promise<BringInStatus>) => {
    const mine = ++seq.current;
    return p.then(
      (v) => {
        if (mine === seq.current) setS(v);
      },
      (e) => toast.error(errText(e) || "Couldn't change Bring in my listening"),
    );
  };
  return (
    <section className="bring-in" aria-label="Bring in my listening">
      <div className="bring-in__head">
        <span className="bring-in__title">Bring in my listening</span>
        <Button
          size="sm"
          variant="ghost"
          onClick={() => void act(backend.bringInPause(!s.userPaused))}
        >
          {s.userPaused ? "Carry on" : "Pause"}
        </Button>
      </div>
      <p className="bring-in__wave">{waveLine(s)}</p>
      <p className="bring-in__counts">
        {n(s.done)} done · {n(s.remaining)} left in this wave · {n(s.today)} of{" "}
        {n(s.perDay)} today
      </p>
      <p
        className={`bring-in__state ${s.paused || s.userPaused ? "is-paused" : ""}`}
        role="status"
      >
        {stateLine(s)}
      </p>
      {whereLine(s) && <p className="bring-in__counts">{whereLine(s)}</p>}
      {s.needsYou.length > 0 && (
        <div className="bring-in__needs">
          <Button
            size="sm"
            variant="ghost"
            aria-expanded={open}
            onClick={() => setOpen((o) => !o)}
          >
            Needs you ({n(s.needsYou.length)})
          </Button>
          {open && (
            <ul
              className="bring-in__needs-list"
              aria-label="Albums that need you"
            >
              {s.needsYou.map((x) => (
                <li key={x.key}>
                  <span className="bring-in__needs-text">
                    <strong>{x.album}</strong> by {x.artist} · {n(x.plays)}{" "}
                    plays
                    <span className="bring-in__why">{x.reason}</span>
                  </span>
                  <span className="bring-in__needs-actions">
                    <Button
                      size="sm"
                      variant="ghost"
                      onClick={() =>
                        void act(backend.bringInNeedsYou(x.key, true))
                      }
                      aria-label={`Try ${x.album} again`}
                    >
                      Try again
                    </Button>
                    <Button
                      size="sm"
                      variant="ghost"
                      onClick={() =>
                        void act(backend.bringInNeedsYou(x.key, false))
                      }
                      aria-label={`Leave ${x.album}`}
                    >
                      Leave it
                    </Button>
                  </span>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
      {s.practice && s.practiced.length > 0 && (
        <p className="bring-in__practice">
          Practice mode would have asked for:{" "}
          {s.practiced
            .slice(0, 5)
            .map((p) => p.album)
            .join(", ")}
          {s.practiced.length > 5 ? ", and more" : ""}.
        </p>
      )}
    </section>
  );
}

const PER_DAY: readonly SelectOption<string>[] = [
  "10",
  "20",
  "30",
  "50",
  "100",
].map((v) => ({ value: v, label: `${v} a day` }));
const FREE: readonly SelectOption<string>[] = [
  "0",
  "20",
  "50",
  "100",
  "250",
].map((v) => ({
  value: v,
  label: v === "0" ? "Any free space" : `${v} GB free`,
}));
const QUEUE: readonly SelectOption<string>[] = ["5", "10", "20", "50"].map(
  (v) => ({ value: v, label: `Queue up to ${v}` }),
);

function keep(
  options: readonly SelectOption<string>[],
  v: string,
  label: (v: string) => string,
) {
  return options.some((o) => o.value === v)
    ? options
    : [...options, { value: v, label: label(v) }];
}

/** Parse "100, 50, 20" into waves, or null when it isn't. */
export function parseWaves(text: string): number[] | null {
  const parts = text.split(/[\s,]+/).filter(Boolean);
  const nums = parts.map(Number);
  if (
    !nums.length ||
    nums.length > 5 ||
    nums.some((x) => !Number.isInteger(x) || x < 1 || x > 100_000)
  )
    return null;
  const sorted = [...new Set(nums)].sort((a, b) => b - a);
  return sorted.length === nums.length ? sorted : null;
}

/** Settings > Library import: "Bring in my listening". */
export function BringInSettings() {
  const s = useSettings((x) => x.settings);
  const id = useId();
  const [text, setText] = useState("");
  useEffect(
    () => setText((s?.bringInWaves ?? []).join(", ")),
    [s?.bringInWaves],
  );
  if (!s) return null;
  const saveWaves = () => {
    const w = parseWaves(text);
    if (!w) {
      toast.error("Waves are 1 to 5 different play counts, like 100, 50, 20");
      setText(s.bringInWaves.join(", "));
      return;
    }
    if (w.join() !== s.bringInWaves.join())
      void updateSettings({ bringInWaves: w });
  };
  return (
    <div className="setting-row bring-in-settings" data-setting="bringIn">
      <div className="setting-row__text">
        <div className="setting-row__head">
          <div id={`${id}-t`} className="setting-row__title">
            Bring in my listening
          </div>
          <MoreText
            title="Bring in my listening"
            text="Asks Lidarr for albums from Played a lot in waves, most played first, a few a day, each after the usual setup check. Pauses when Lidarr is busy or space runs low. Albums it can't sort out wait for you on the Missing page. Synced."
          />
        </div>
        <p id={`${id}-d`} className="setting-row__desc">
          Asks Lidarr for your most played albums, a few a day. Synced.
        </p>
      </div>
      <div className="setting-row__control">
        <Switch
          checked={s.bringIn}
          onChange={(v) => void updateSettings({ bringIn: v })}
          labelledBy={`${id}-t`}
          describedBy={`${id}-d`}
        />
      </div>
      {s.bringIn && (
        <div className="setting-row__block played-settings__choices">
          <label
            className="bring-in-settings__waves"
           
          >
            <span>Waves (plays)</span>
            <input
              className="setting-text"
              value={text}
              inputMode="numeric"
              aria-label="Waves, fewest plays"
              onChange={(e) => setText(e.target.value)}
              onBlur={saveWaves}
              onKeyDown={(e) => {
                if (e.key === "Enter") saveWaves();
              }}
            />
          </label>
          <span>
            <Select
              label="Albums a day"
              value={String(s.bringInPerDay)}
              options={keep(
                PER_DAY,
                String(s.bringInPerDay),
                (v) => `${v} a day`,
              )}
              onChange={(v) =>
                void updateSettings({ bringInPerDay: Number(v) })
              }
            />
          </span>
          <span>
            <Select
              label="Pause below free space"
              value={String(s.bringInMinFreeGb)}
              options={keep(
                FREE,
                String(s.bringInMinFreeGb),
                (v) => `${v} GB free`,
              )}
              onChange={(v) =>
                void updateSettings({ bringInMinFreeGb: Number(v) })
              }
            />
          </span>
          <span>
            <Select
              label="Pause above Lidarr's queue"
              value={String(s.bringInMaxQueue)}
              options={keep(
                QUEUE,
                String(s.bringInMaxQueue),
                (v) => `Queue up to ${v}`,
              )}
              onChange={(v) =>
                void updateSettings({ bringInMaxQueue: Number(v) })
              }
            />
          </span>
        </div>
      )}
    </div>
  );
}
