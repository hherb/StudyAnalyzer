import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// Tauri serves the dev build from a fixed port and owns the terminal, so vite
// must not clear the screen or pick a different port when 1420 is busy.
// `TAURI_DEV_HOST` is set when Tauri is building for a device on the network: it
// replaces the dev URL's host with a LAN address, and the server has to be
// listening there or the app opens a blank window. Left unset — the desktop
// case — the server stays on localhost, which is all the webview needs.
// Declared rather than pulled in: the project has no @types/node, and this is
// the only Node global the config needs.
declare const process: { env: Record<string, string | undefined> };
const devHost = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: devHost || undefined,
    // Watching the Rust build tree makes the dev server thrash; the Tauri CLI
    // restarts it on Rust changes instead.
    watch: {
      ignored: ["**/src-tauri/**", "**/.cargo-home/**", "**/.cargo-target/**"],
    },
  },
  build: {
    // The app only ever runs inside the platform webview.
    target: "safari15",
    sourcemap: true,
  },
});
