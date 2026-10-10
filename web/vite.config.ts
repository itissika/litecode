/// <reference types="vitest/config" />
import { defineConfig, loadEnv, type ProxyOptions } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

import { DEV_UPSTREAM_CLOSED, pointProxyAtDevUpstream } from "./devUpstream.ts";

/**
 * `dev_win.ps1` sets `LITECODE_DEV_UPSTREAM_FILE`. The desktop sidecar binds an
 * ephemeral port, so the proxy target is read from that file on each request.
 * Vite passes this options object straight to http-proxy, so updating `target`
 * inside `bypass` (which runs before the proxy call) retargets that request.
 * `serve_win.ps1` / `serve.sh` leave the file unset and use `LITECODE_BIND`.
 */
function sidecarProxy(fixedTarget: string, ws: boolean, upstreamFile: string): ProxyOptions {
  if (!upstreamFile) {
    return ws
      ? { target: fixedTarget, ws: true, changeOrigin: true }
      : { target: fixedTarget, changeOrigin: true };
  }
  const options: ProxyOptions = {
    target: DEV_UPSTREAM_CLOSED,
    changeOrigin: true,
    bypass() {
      pointProxyAtDevUpstream(options, upstreamFile);
    },
  };
  if (ws) options.ws = true;
  return options;
}

export default defineConfig(({ mode }) => {
  // serve_win.ps1 / serve.sh export LITECODE_BIND so a non-default bind (e.g.
  // when another app owns 7483 on Windows) keeps the dev proxy in sync.
  // Falls back to the historical default when unset.
  const env = loadEnv(mode, ".", "LITECODE_");
  const bind = env.LITECODE_BIND || "127.0.0.1:7483";
  const upstreamFile = (env.LITECODE_DEV_UPSTREAM_FILE || process.env.LITECODE_DEV_UPSTREAM_FILE || "").trim();

  return {
    plugins: [react(), tailwindcss()],
    optimizeDeps: {
      // These are only reached from lazy file previews, so the cold-start
      // crawl never sees them. Discovering them on first open re-optimizes
      // deps and the in-flight import gets 504 Outdated Optimize Dep.
      include: ["@milkdown/crepe", "@milkdown/kit/utils", "pdfjs-dist"],
    },
    test: {
      environment: "jsdom",
      include: ["src/**/*.test.ts", "src/**/*.test.tsx", "devUpstream.test.ts"],
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
        "/ws": sidecarProxy(`ws://${bind}`, true, upstreamFile),
        "/health": sidecarProxy(`http://${bind}`, false, upstreamFile),
        "/api": sidecarProxy(`http://${bind}`, false, upstreamFile),
      },
    },
  };
});
