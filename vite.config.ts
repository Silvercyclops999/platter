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

  test: {
    environment: "jsdom",
    setupFiles: ["./src/test/setup.ts"],
    testTimeout: 15000,
    include: ["src/**/*.test.{ts,tsx}"],
    // Process CSS so `?raw` imports (token tests) see real file contents.
    css: true,
  },
}));
