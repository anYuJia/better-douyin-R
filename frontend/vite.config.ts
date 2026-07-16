import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import path from "path";

const devPort = Number.parseInt(process.env.TAURI_DEV_PORT || "1420", 10);

export default defineConfig({
  plugins: [react(), tailwindcss()],
  base: "./",
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  build: {
    outDir: "../dist",
    emptyOutDir: true,
    target: ["es2021", "chrome100", "safari13"],
    cssCodeSplit: false,
    modulePreload: false,
  },
  server: {
    port: Number.isFinite(devPort) ? devPort : 1420,
    strictPort: process.env.TAURI_STRICT_PORT === "true",
    host: "127.0.0.1",
  },
  envPrefix: ["VITE_", "TAURI_"],
  clearScreen: false,
});
