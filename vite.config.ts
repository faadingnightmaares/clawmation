/// <reference types="vitest/config" />
import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { fileURLToPath } from "node:url";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [react(), tailwindcss()],

  resolve: {
    alias: {
      "@": fileURLToPath(new URL("./src", import.meta.url)),
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
      //
      // …and the app's own data directories. In debug builds `paths.rs` resolves
      // the data root to this folder, so every settings save, stats bump, macro
      // write and snapshot lands inside the Vite root. Watching them makes the
      // running app reload itself mid-recording; they are runtime state, never
      // sources, so they stay out of the watcher entirely.
      ignored: [
        "**/src-tauri/**",
        "**/config/**",
        "**/macros/**",
        "**/snapshots/**",
        "**/templates/**",
      ],
    },
  },

  // Three HTML entry points: the app (`index.html`) and the two transparent
  // overlays, the recording indicator (`indicator.html`) and the live detection
  // boxes (`detections.html`), each with its own module graph.
  build: {
    rollupOptions: {
      input: {
        main: fileURLToPath(new URL("./index.html", import.meta.url)),
        indicator: fileURLToPath(new URL("./indicator.html", import.meta.url)),
        detections: fileURLToPath(new URL("./detections.html", import.meta.url)),
      },
    },
  },

  // Component tests run in jsdom; the Tauri `invoke` seam is mocked per-test.
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: "./src/setupTests.ts",
  },
}));
