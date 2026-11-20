import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// In dev, API/media calls go to `vcsd serve` (browser mode) on :7878.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    proxy: { "/api": "http://127.0.0.1:7878", "/media": "http://127.0.0.1:7878" },
  },
  build: { target: "es2021", outDir: "dist", emptyOutDir: true, chunkSizeWarningLimit: 1200 },
});
