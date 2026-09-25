/// <reference types="vitest/config" />
import { defineConfig, loadEnv } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

export default defineConfig(({ mode }) => {
  // serve_win.ps1 / serve.sh export LITECODE_BIND so a non-default bind (e.g.
  // when another app owns 7483 on Windows) keeps the dev proxy in sync.
  // Falls back to the historical default when unset.
  const bind = loadEnv(mode, ".", "LITECODE_").LITECODE_BIND || "127.0.0.1:7483";

  return {
    plugins: [react(), tailwindcss()],
    test: {
      environment: "jsdom",
      include: ["src/**/*.test.ts", "src/**/*.test.tsx"],
    },
    server: {
      // Pin IPv4: `localhost` resolves to `::1` first on Windows, so Vite would
      // bind IPv6-only and the `http://127.0.0.1:<port>` handshake URL that
      // serve_win.ps1 prints would refuse connections.
      host: "127.0.0.1",
      port: 5173,
      watch: {
        ignored: ['**/node_modules/**', '**/dist/**', '**/.git/**'],
      },
      proxy: {
        "/ws": {
          target: `ws://${bind}`,
          ws: true,
          changeOrigin: true,
        },
        "/health": {
          target: `http://${bind}`,
          changeOrigin: true,
        },
        "/api": {
          target: `http://${bind}`,
          changeOrigin: true,
        },
      },
    },
  };
});
