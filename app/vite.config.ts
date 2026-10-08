import { defineConfig } from "vitest/config";
import solid from "@solidjs/vite-plugin";
import tailwindcss from "@tailwindcss/vite";

// Tauri expects a fixed port and must see Rust-side errors.
export default defineConfig({
  plugins: [solid(), tailwindcss()],
  clearScreen: false,
  server: { port: 1420, strictPort: true, watch: { ignored: ["**/src-tauri/**"] } },
  envPrefix: ["VITE_", "TAURI_ENV_"],
  build: { target: "es2022", sourcemap: true },
  test: {
    environment: "jsdom",
    include: ["tests/unit/**/*.test.{ts,tsx}"],
    setupFiles: ["tests/unit/setup.ts"],
  },
  resolve: { conditions: ["browser", "development"] },
});
