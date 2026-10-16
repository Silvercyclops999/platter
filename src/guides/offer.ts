/**
 * First run: right after the first sign-in, offer the Get started guide
 * (which has the look step in it). With setup guides turned off, the
 * plain "Pick a look" step shows instead, as before.
 */
import type { Settings } from "../lib/types";
import { guides } from "../stores/guides";
import { offerLookStep } from "../stores/looks";
import { updateSettings, useSettings } from "../stores/settings";

export function offerFirstRun(): void {
  const decide = (s: Settings | null) => {
    if (!s) return false;
    if (s.showSetupGuides && guides.offerable("firstRun") && !s.lookStepDone) {
      // The guide covers the look step, so it isn't offered again.
      void updateSettings({ lookStepDone: true });
      guides.open("firstRun");
    } else {
      offerLookStep();
    }
    return true;
  };
  if (decide(useSettings.getState().settings)) return;
  const un = useSettings.subscribe((st) => {
    if (decide(st.settings)) un();
  });
}
