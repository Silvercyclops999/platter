/**
 * First run: right after the first sign-in, the music is simply there.
 * Nothing opens on top of it: the libraries are already picked, the look is
 * the default one, and everything else is off until it's turned on. The Get
 * started guide stays in Help for anyone who wants the tour, and the look
 * step is marked done so it isn't offered later either.
 */
import type { Settings } from "../lib/types";
import { updateSettings, useSettings } from "../stores/settings";

export function finishFirstRun(): void {
  const decide = (s: Settings | null) => {
    if (!s) return false;
    if (!s.lookStepDone) void updateSettings({ lookStepDone: true });
    return true;
  };
  if (decide(useSettings.getState().settings)) return;
  const un = useSettings.subscribe((st) => {
    if (decide(st.settings)) un();
  });
}
