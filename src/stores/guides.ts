/**
 * Setup guides: which one is open, and how far each has got. Progress is
 * kept on this device (localStorage), so a guide closed halfway opens where
 * it was left, and a finished or skipped one isn't offered again.
 */
import { create } from "zustand";

export type GuideId = "firstRun" | "lidarr" | "import" | "sharing" | "streamDeck";

/** Each guide's name (kept here so buttons that open one don't load the guides). */
export const GUIDE_TITLES: Record<GuideId, string> = {
  firstRun: "Get started",
  lidarr: "Connect Lidarr",
  import: "Import your library",
  sharing: "Invite a friend",
  streamDeck: "Set up Stream Deck",
};

export interface GuideProgress {
  /** The step to open on. */
  step: number;
  /** Reached the end. */
  done: boolean;
  /** Closed with Skip guide: not offered by itself again (Settings still opens it). */
  skipped: boolean;
}

const KEY = "platter.guides.v1"; // gitleaks:allow (localStorage key name, not a secret)

function load(): Partial<Record<GuideId, GuideProgress>> {
  try {
    const raw = localStorage.getItem(KEY);
    const p = raw ? (JSON.parse(raw) as unknown) : null;
    return p && typeof p === "object" ? (p as Partial<Record<GuideId, GuideProgress>>) : {};
  } catch {
    return {};
  }
}

function save(p: Partial<Record<GuideId, GuideProgress>>) {
  try {
    localStorage.setItem(KEY, JSON.stringify(p));
  } catch {
    /* storage unavailable: this session only */
  }
}

interface GuidesState {
  active: GuideId | null;
  progress: Partial<Record<GuideId, GuideProgress>>;
}

export const useGuides = create<GuidesState>()(() => ({ active: null, progress: load() }));

const FRESH: GuideProgress = { step: 0, done: false, skipped: false };

function setProgress(id: GuideId, p: Partial<GuideProgress>) {
  const progress = { ...useGuides.getState().progress, [id]: { ...FRESH, ...useGuides.getState().progress[id], ...p } };
  useGuides.setState({ progress });
  save(progress);
}

export const guides = {
  progress(id: GuideId): GuideProgress {
    return { ...FRESH, ...useGuides.getState().progress[id] };
  },
  /** Opens a guide where it was left; a finished one starts again. */
  open(id: GuideId) {
    if (guides.progress(id).done) setProgress(id, { step: 0, done: false });
    useGuides.setState({ active: id });
  },
  goTo(id: GuideId, step: number) {
    setProgress(id, { step: Math.max(0, step) });
  },
  /** Closes for now: the guide opens on the same step next time. */
  close() {
    useGuides.setState({ active: null });
  },
  /** Closes and doesn't offer it again by itself. */
  skip(id: GuideId) {
    setProgress(id, { skipped: true });
    useGuides.setState({ active: null });
  },
  finish(id: GuideId) {
    setProgress(id, { step: 0, done: true });
    useGuides.setState({ active: null });
  },
  /** Whether a guide should offer itself (not finished or skipped). */
  offerable(id: GuideId): boolean {
    const p = guides.progress(id);
    return !p.done && !p.skipped;
  },
  /** Tests: forget everything. */
  reset() {
    save({});
    useGuides.setState({ active: null, progress: {} });
  },
};
