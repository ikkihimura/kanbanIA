import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

// Pruebas de UI en jsdom. Los comandos IPC se simulan por módulo en cada prueba:
// estas pruebas verifican la interfaz, no la integración con Tauri (cubierta por las IPC de Rust).
export default defineConfig({
  plugins: [react()],
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.{ts,tsx}"],
    setupFiles: ["./src/test/setup.ts"],
    restoreMocks: true,
  },
});
