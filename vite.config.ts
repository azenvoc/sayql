import { defineConfig, type Plugin } from "vite";
import react from "@vitejs/plugin-react";
// tauri.conf.json's version is the one the app reports.
import tauriConf from "./src-tauri/tauri.conf.json" with { type: "json" };
// @ts-expect-error type error without @types/node package
import process from "node:process";
const host = process.env.TAURI_DEV_HOST;

/**
 * Puts the app's version into splash.html, which has no scripts to look it up, so the splash
 * never shows a stale one.
 */
const splashVersion: Plugin = {
  name: "sayql-splash-version",
  transformIndexHtml: (html) => html.replaceAll("%SAYQL_VERSION%", tauriConf.version),
};

// https://vite.dev/config/
export default defineConfig(() => ({
  plugins: [react(), splashVersion],

  // Two pages: the app, and the splash card shown while it loads (see splash.html).
  build: {
    rolldownOptions: {
      input: { index: "index.html", splash: "splash.html" },
    },
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
