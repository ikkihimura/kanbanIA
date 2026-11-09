import process from "node:process";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

const host = process.env["TAURI_DEV_HOST"];

// https://v2.tauri.app/start/frontend/vite/
export default defineConfig({
  plugins: [react()],
  // No ocultar errores de Rust en la terminal.
  clearScreen: false,
  server: {
    // Tauri espera un puerto fijo (devUrl en tauri.conf.json).
    port: 1420,
    strictPort: true,
    host: host ?? false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    watch: { ignored: ["**/src-tauri/**"] },
  },
});
