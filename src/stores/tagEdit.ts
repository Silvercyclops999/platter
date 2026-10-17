/**
 * The tag editor's dialogs (which is open, for what), and saving or
 * undoing with a toast that says what happened.
 */
import { create } from "zustand";
import { backend } from "../lib/backend";
import type { TagCheckScope, TagField, TagFixKind, TagSaveRequest, TagSaveResult } from "../lib/types";
import { errText } from "./plexEdit";
import { toast, useToasts } from "./toasts";

/** How long Undo stays on the toast. */
export const UNDO_MS = 10000;

export interface EditorTarget {
  albumId: string | null;
  trackIds: string[];
  /** Where the check's Fix points. */
  focus?: TagField | null;
}

export interface FixTarget {
  kind: TagFixKind;
  albumId: string;
  title: string;
}

export interface ExplicitTarget {
  albumIds: string[];
  era?: { artistId: string; era: string } | null;
  explicit: boolean;
  title: string;
}

interface TagEditState {
  editor: EditorTarget | null;
  check: { scope: TagCheckScope; title: string } | null;
  fix: FixTarget | null;
  explicit: ExplicitTarget | null;
  openEditor: (t: EditorTarget) => void;
  openCheck: (scope: TagCheckScope, title: string) => void;
  openFix: (t: FixTarget) => void;
  openExplicit: (t: ExplicitTarget) => void;
  close: (which: "editor" | "check" | "fix" | "explicit") => void;
}

export const useTagEdit = create<TagEditState>()((set) => ({
  editor: null,
  check: null,
  fix: null,
  explicit: null,
  openEditor: (editor) => set({ editor }),
  openCheck: (scope, title) => set({ check: { scope, title } }),
  openFix: (fix) => set({ fix }),
  openExplicit: (explicit) => set({ explicit }),
  close: (which) => set({ [which]: null }),
}));

/** Undo an edit, with a toast either way. */
export async function undoTagEdit(id: string): Promise<TagSaveResult | null> {
  try {
    const r = await backend.tagEditUndo(id);
    if (r.problems.length) toast.error(r.message);
    else toast.info(r.message);
    return r;
  } catch (e) {
    toast.error(errText(e) || "Couldn't undo that edit");
    return null;
  }
}

/**
 * Save, then a toast: "Saved N files" with Undo, or why not. Returns the
 * result (null when the call itself failed: the error is in the toast).
 */
export async function saveTags(request: TagSaveRequest): Promise<TagSaveResult | null> {
  try {
    const r = await backend.tagEditSave(request);
    if (r.saved > 0) {
      const extra = r.rescanError ? ` ${r.rescanError}` : "";
      useToasts.getState().push(
        {
          kind: r.problems.length ? "error" : "info",
          message: `${r.message}${extra}`,
          ...(r.undoId ? { action: { label: "Undo", run: () => void undoTagEdit(r.undoId!) } } : {}),
        },
        UNDO_MS,
      );
    } else if (!r.problems.length) {
      toast.info(r.message);
    }
    return r;
  } catch (e) {
    toast.error(errText(e) || "Couldn't save the tags");
    return null;
  }
}
