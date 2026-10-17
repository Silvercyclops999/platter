/**
 * Mix on a playlist page: the switch (a pill under the action row on your
 * own playlists, a toggle in the action row on others), what asks first
 * (sort order, shuffle), and on a mixed playlist the transition between
 * each pair of rows with a picker, and each song's tempo once it's been
 * analysed.
 */
import { Blend } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { Button, IconButton } from "../components/Button";
import { openContextMenu } from "../components/ContextMenu";
import { Dialog } from "../components/Dialog";
import { Tooltip } from "../components/Tooltip";
import { backend } from "../lib/backend";
import type { AutomixView } from "../lib/types";
import { MIX_STYLES, loadMixed, setGapStyle, setMix, styleLabel, useMixed } from "../stores/mixed";
import type { MenuItem } from "../stores/ui";

export const SORT_ASK = "Mixing will change your sort order to custom order";
export const SORT_LOCKED = "Turn off mix to change the sort order";
export const SHUFFLE_ASK = "Mixed playlists sound better in order. Some transitions may not play while shuffled.";

/** Whether a playlist is mixed (loads the list the first time). */
export function useMix(id: string): boolean {
  const mixed = useMixed((s) => !!s.mixed[id]);
  useEffect(() => {
    if (!useMixed.getState().loaded) void loadMixed();
    const un = backend.listen("syncstore://changed", (c) => {
      if (c.remote && c.namespaces.includes("mixedPlaylists")) void loadMixed();
    });
    return () => {
      un.then((f) => f());
    };
  }, []);
  return mixed;
}

/**
 * Turning Mix on with the songs sorted some other way asks first; Continue
 * puts them back in the playlist's own order. Returns the request and the
 * dialog to render.
 */
export function useMixSwitch(id: string, sorted: boolean, toCustom: () => void) {
  const [asking, setAsking] = useState(false);
  const mixed = useMixed((s) => !!s.mixed[id]);
  const toggle = () => {
    if (!mixed && sorted) setAsking(true);
    else void setMix(id, !mixed);
  };
  const dialog = (
    <Dialog open={asking} onClose={() => setAsking(false)} label="Turn on mixing?" className="confirm">
      <h2 className="confirm__title">Turn on mixing?</h2>
      <p className="confirm__body">{SORT_ASK}.</p>
      <div className="confirm__actions">
        <Button variant="ghost" onClick={() => setAsking(false)}>
          Cancel
        </Button>
        <Button
          variant="primary"
          onClick={() => {
            setAsking(false);
            toCustom();
            void setMix(id, true);
          }}
        >
          Continue
        </Button>
      </div>
    </Dialog>
  );
  return { mixed, toggle, dialog };
}

/** Your own playlist: the Mix pill under the action row. */
export function MixPill({ mixed, onToggle }: { mixed: boolean; onToggle: () => void }) {
  const tip = mixed ? "Turn off mixing" : "Turn on mixing";
  return (
    <Tooltip label={tip} describe>
      <button type="button" className={`pl-mix-pill ${mixed ? "is-on" : ""}`} aria-pressed={mixed} aria-label="Mix" onClick={onToggle}>
        <Blend size={16} aria-hidden="true" />
        <span>Mix</span>
      </button>
    </Tooltip>
  );
}

/** Someone else's playlist: Mix as a toggle in the action row. */
export function MixToggle({ mixed, onToggle, name }: { mixed: boolean; onToggle: () => void; name?: string | null }) {
  const label = mixed ? `Turn off mix for ${name ?? "this playlist"}` : `Turn on mix for ${name ?? "this playlist"}`;
  return <IconButton label={label} tooltip={mixed ? "Turn off mix" : "Turn on mix"} size="lg" pressed={mixed} icon={<Blend size={22} />} onClick={onToggle} />;
}

/** The more menu's entry on someone else's playlist. */
export function mixMenuItem(mixed: boolean, onToggle: () => void): MenuItem {
  return { label: mixed ? "Turn off mix" : "Turn on mix", icon: Blend, onSelect: onToggle };
}

/** Shuffle on a mixed playlist asks first. Returns the gate and its dialog. */
export function useShuffleAsk() {
  const go = useRef<(() => void) | null>(null);
  const [open, setOpen] = useState(false);
  const ask = (proceed: () => void) => {
    go.current = proceed;
    setOpen(true);
  };
  const close = () => {
    setOpen(false);
    go.current = null;
  };
  const dialog = (
    <Dialog open={open} onClose={close} label="Shuffle a mixed playlist?" className="confirm">
      <h2 className="confirm__title">Shuffle a mixed playlist?</h2>
      <p className="confirm__body">{SHUFFLE_ASK}</p>
      <div className="confirm__actions">
        <Button variant="ghost" onClick={close}>
          Turn off shuffle
        </Button>
        <Button
          variant="primary"
          onClick={() => {
            const f = go.current;
            close();
            f?.();
          }}
        >
          Shuffle anyway
        </Button>
      </div>
    </Dialog>
  );
  return { ask, dialog };
}

/** Tempo and transitions for the rows shown, read again as songs get analysed. */
export function useAutomixView(id: string, mixed: boolean, trackIds: string[]): AutomixView | null {
  const [view, setView] = useState<AutomixView | null>(null);
  const key = trackIds.join("\n");
  const styles = useMixed((s) => s.mixed[id]?.styles);
  useEffect(() => {
    if (!mixed || !trackIds.length) {
      setView(null);
      return;
    }
    let alive = true;
    const load = () =>
      backend.automixView(id, trackIds).then(
        (v) => alive && setView(v),
        () => {},
      );
    void load();
    const un = backend.listen("analysis://changed", (ids) => {
      if (ids.some((t) => trackIds.includes(t))) void load();
    });
    return () => {
      alive = false;
      un.then((f) => f());
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [id, mixed, key, styles]);
  return view;
}

/** "128 BPM" for a row, once known. */
export function bpmNote(view: AutomixView | null, trackId: string): string[] | undefined {
  const bpm = view?.songs.find((s) => s.id === trackId)?.bpm;
  return bpm ? [`${bpm} BPM`] : undefined;
}

/** The transition between two rows: what it'll do, and a picker. */
export function GapMarker({ playlistId, gap, from, to }: { playlistId: string; gap: AutomixView["gaps"][number]; from: string; to: string }) {
  const can = !gap.reason;
  const what = !can
    ? "Can't be mixed"
    : gap.style === "none"
      ? "No transition"
      : gap.style === "auto"
        ? gap.beatmatched
          ? "Mixes on the beat"
          : "Mixes"
        : styleLabel(gap.style);
  const label = can ? `Transition from ${from} to ${to}: ${what}. Change` : `${what} from ${from} to ${to}: ${gap.reason}`;
  const pick = (el: HTMLElement) =>
    openContextMenu(
      "Transition",
      MIX_STYLES.map((m, i) => ({
        heading: i === 0 ? "Transition" : undefined,
        label: m.label,
        description: m.hint,
        icon: null,
        checked: gap.style === m.style,
        onSelect: () => void setGapStyle(playlistId, gap.fromId, gap.toId, m.style),
      })),
      el,
    );
  return (
    <div className={`pl-gap ${can ? "" : "pl-gap--no"}`} role="row">
      <div role="gridcell" className="pl-gap__cell">
        {can ? (
          <button type="button" className="pl-gap__btn" aria-haspopup="menu" aria-label={label} onClick={(e) => pick(e.currentTarget)}>
            <Blend size={12} aria-hidden="true" />
            <span>{what}</span>
          </button>
        ) : (
          <Tooltip label={gap.reason ?? what}>
            <span className="pl-gap__btn pl-gap__btn--static" tabIndex={0} aria-label={label}>
              {what}
            </span>
          </Tooltip>
        )}
      </div>
    </div>
  );
}
