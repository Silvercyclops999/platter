/**
 * Edit in Plex: which servers this account can edit, which edit dialog is
 * open, and running an edit with an Undo toast.
 */
import { create } from "zustand";
import { backend } from "../lib/backend";
import { serverTagOf } from "../lib/ids";
import type { AppError, EditAccess, EditChange, EditKind } from "../lib/types";
import { hasCap, useCap } from "./caps";
import { useSettings } from "./settings";
import { toast, useToasts } from "./toasts";

/** How long the Undo button stays on the toast. */
export const UNDO_MS = 8000;

/** The album or artist a dialog is for. */
export interface EditTarget {
  id: string;
  kind: EditKind;
  title: string;
  thumb: string | null;
  /** Albums: the album artist. */
  artistTitle?: string;
}

export type EditDialog = "cover" | "background" | "match" | "details" | "merge" | "split" | "history";

interface PlexEditState {
  access: EditAccess | null;
  dialog: { name: EditDialog; target: EditTarget } | null;
  setAccess: (a: EditAccess | null) => void;
  open: (name: EditDialog, target: EditTarget) => void;
  close: () => void;
}

export const usePlexEdit = create<PlexEditState>()((set) => ({
  access: null,
  dialog: null,
  setAccess: (access) => set({ access }),
  open: (name, target) => set({ dialog: { name, target } }),
  close: () => set({ dialog: null }),
}));

let loading: Promise<void> | null = null;
let subscribed = false;

/** Ask the core again which servers can be edited. */
export function reloadEditAccess(): Promise<void> {
  loading = backend.plexEditAccess().then(
    (a) => usePlexEdit.getState().setAccess(a),
    () => usePlexEdit.getState().setAccess(null),
  );
  return loading;
}

/** Load which servers can be edited once, and again when the sign-in changes. */
export function ensureLoaded() {
  if (!loading) void reloadEditAccess();
  if (!subscribed) {
    subscribed = true;
    // Signing in as someone else or switching servers changes what's editable.
    void backend.listen("auth://changed", () => void reloadEditAccess());
  }
}

/** Whether the item's server can be edited by this account, with editing on. */
export function canEditId(access: EditAccess | null, show: boolean, id: string): boolean {
  if (!access || !show) return false;
  const tag = serverTagOf(id);
  return tag ? access.servers.includes(tag) : access.connected;
}

/** [`canEditId`] now, for menus built on demand. */
export function canEditNow(id: string): boolean {
  ensureLoaded();
  const show = useSettings.getState().settings?.plexEditShow !== false;
  return canEditId(usePlexEdit.getState().access, show, id) && hasCap("metadataEdit", id);
}

/** [`canEditId`] as a hook. */
export function useCanEdit(id: string | null | undefined): boolean {
  ensureLoaded();
  const access = usePlexEdit((s) => s.access);
  const show = useSettings((s) => s.settings?.plexEditShow !== false);
  const editable = useCap("metadataEdit", id);
  return !!id && editable && canEditId(access, show, id);
}

export function errText(e: unknown): string {
  return e && typeof e === "object" && "message" in e ? String((e as AppError).message) : "";
}

/** A failed edit: say why; a refusal hides editing for that server. */
export function editFailed(what: string, e: unknown) {
  const m = errText(e);
  toast.error(m ? `${what}: ${m}` : what);
  if (e && typeof e === "object" && (e as AppError).kind === "auth") void reloadEditAccess();
}

/** Undo a change, with a toast either way. */
export async function undoChange(changeId: string): Promise<EditChange | null> {
  try {
    const c = await backend.plexEditUndo(changeId);
    toast.info("Change undone");
    return c;
  } catch (e) {
    editFailed("Couldn't undo that", e);
    return null;
  }
}

/** Run an edit; on success a toast says what changed, with Undo when it can be undone. */
export async function runEdit(what: string, edit: () => Promise<EditChange>, art?: { path: string | null; kind: EditKind }): Promise<EditChange | null> {
  try {
    const c = await edit();
    useToasts.getState().push(
      {
        kind: "info",
        message: c.label,
        ...(art?.path ? { art } : {}),
        ...(c.undoable ? { action: { label: "Undo", run: () => void undoChange(c.id) } } : {}),
      },
      UNDO_MS,
    );
    return c;
  } catch (e) {
    editFailed(what, e);
    return null;
  }
}
