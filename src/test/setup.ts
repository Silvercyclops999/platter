import "@testing-library/jest-dom/vitest";
import { cleanup, configure } from "@testing-library/react";
import { afterEach } from "vitest";

// Vitest runs without globals, so Testing Library can't auto-register cleanup.
afterEach(() => cleanup());

// findBy* and waitFor wait up to 3 s instead of 1 s: the full suite runs many
// files at once, and a busy machine made one-second waits fail at random.
configure({ asyncUtilTimeout: 3000 });
