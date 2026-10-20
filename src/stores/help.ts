/**
 * The help panel: which page is open in the right slot, and where. Help
 * opens next to what it explains, so the screen stays where it was.
 */
import { create } from "zustand";
import { useUi } from "./ui";
import { usePhoneForm } from "../phone/form";

interface HelpState {
  slug: string | null;
  anchor: string | null;
  /** Bumped on every open, so the same section scrolls into view again. */
  tick: number;
}

export const useHelp = create<HelpState>()(() => ({ slug: null, anchor: null, tick: 0 }));

export const help = {
  /** Opens a page, or the list of pages for "". */
  open(slug: string, anchor?: string) {
    // The phone has no side panel: help is a page there.
    if (usePhoneForm.getState().phone) {
      void import("./router").then(({ nav }) => nav.go({ name: "help", ...(slug ? { slug } : {}), ...(anchor ? { anchor } : {}) }));
      return;
    }
    useHelp.setState((s) => ({ slug: slug || null, anchor: anchor ?? null, tick: s.tick + 1 }));
    useUi.getState().setHelpOpen(true);
  },
  close() {
    useUi.getState().setHelpOpen(false);
  },
};
