/// <reference types="vitest/config" />
import { defineConfig } from "vite";
import { configDefaults } from "vitest/config";
import react from "@vitejs/plugin-react";
import process from "node:process";
import path from "node:path";

const host = process.env.TAURI_DEV_HOST;

// Tests that read computed styles (getComputedStyle, or visibility that comes
// from a stylesheet) and so need the app's real CSS loaded.
const STYLE_TESTS = ["src/test/sleeve-notes.test.tsx"];

// https://vite.dev/config/
export default defineConfig(() => ({
  plugins: [react()],

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    // 3. tell Vite to ignore watching `src-tauri`
    // Only this checkout's own .claude and dev folders: a worktree lives under
    // .claude itself, so a "**/.claude/**" glob would stop it watching anything.
    watch: {
      ignored: ["**/src-tauri/**", path.resolve(import.meta.dirname, ".claude") + "/**", path.resolve(import.meta.dirname, "dev") + "/**"],
    },
  },

  build: {
    rolldownOptions: {
      output: {
        // Everything the first screen needs goes in one chunk; rarely used
        // pages (Settings, Library import, the editors) load on first use.
        codeSplitting: { groups: [{ name: "app", tags: ["$initial"] }] },
      },
    },
  },

  test: {
    environment: "jsdom",
    setupFiles: ["./src/test/setup.ts"],
    testTimeout: 15000,
    // Every core by default; a few workers keep a desktop cool. CI uses them all.
    maxWorkers: process.env.CI ? undefined : 4,
    // Role queries compute styles for every element they name, against every
    // stylesheet on the page, so the app's CSS stays out of most tests. Only
    // `?raw` imports are processed (token tests read real file contents).
    // Tests that check computed styles go in STYLE_TESTS and get all the CSS.
    projects: [
      {
        extends: true,
        test: {
          name: "ui",
          include: ["src/**/*.test.{ts,tsx}"],
          exclude: [...configDefaults.exclude, ...STYLE_TESTS],
          css: { include: [/\?raw$/] },
        },
      },
      {
        extends: true,
        test: { name: "styles", include: STYLE_TESTS, css: true },
      },
    ],
  },
}));
