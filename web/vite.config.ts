/// <reference types="vitest/config" />
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.ts", "src/**/*.test.tsx"],
  },
  server: {
    port: 5173,
    watch: {
      ignored: ['**/node_modules/**', '**/dist/**', '**/.git/**'],
    },
    proxy: {
      "/ws": {
        target: "ws://127.0.0.1:7483",
        ws: true,
        changeOrigin: true,
      },
      "/health": {
        target: "http://127.0.0.1:7483",
        changeOrigin: true,
      },
      "/api": {
        target: "http://127.0.0.1:7483",
        changeOrigin: true,
      },
    },
  },
});
