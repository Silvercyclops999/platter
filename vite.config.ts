/// <reference types="vitest/config" />
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import process from "node:process";
import path from "node:path";

const host = process.env.TAURI_DEV_HOST;

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
    // No source maps in the app (scripts/check-bundle.mjs fails the build on one).
    sourcemap: false,
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
    // CI machines run the suite several times slower than a desktop, and
    // role queries on big pages grow with the app's CSS: more room there.
    testTimeout: process.env.CI ? 30000 : 15000,
    // Every core by default; a few workers keep a desktop cool. CI uses them all.
    maxWorkers: process.env.CI ? undefined : 4,
    include: ["src/**/*.test.{ts,tsx}"],
    // Process CSS so `?raw` imports (token tests) see real file contents.
    css: true,
  },
}));
