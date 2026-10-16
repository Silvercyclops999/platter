import "@testing-library/jest-dom/vitest";
import { cleanup, configure } from "@testing-library/react";
import { afterEach, vi } from "vitest";
import { toastExpiry, useToasts } from "../stores/toasts";
import { advancedView } from "../pages/settingsMeta";

afterEach(() => {
  // Vitest runs without globals, so Testing Library can't auto-register cleanup.
  cleanup();
  // A test that failed with fake timers on would hand them to every test
  // after it, which then hang waiting for time that never passes.
  vi.useRealTimers();
  // No test starts with another one's toasts.
  useToasts.setState({ toasts: [] });
});

// findBy* and waitFor give up after 10 s. Tests wait for a condition, never
// for time, so this is a hang guard and not a budget: a passing wait returns
// as soon as its condition holds, and a lost update never comes however long
// the wait. Ten seconds covers the full suite running beside a build on a
// busy machine or a slow CI runner. Don't pass a shorter `timeout` to one
// wait; that only brings the flakiness back.
configure({ asyncUtilTimeout: 10_000 });

// Toasts stay until a test clears them: a 4 s lifetime is a real timer that a
// slow run outlasts between showing a toast and checking it (src/test/toasts.test.ts
// covers the timing with fake timers).
toastExpiry.on = false;

// Settings rows under Advanced start open, so a test of one row needn't open
// Advanced first (settings-easy.test.tsx turns this off to test collapsing).
advancedView.startOpen = true;
