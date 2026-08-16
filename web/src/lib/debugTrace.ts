/**
 * Opt-in client trace for transcript / turn robustness.
 * Default off. Enable from DevTools:
 *   litecodeDebug.on()            // turn + buffer
 *   litecodeDebug.on("turn")
 *   litecodeDebug.off()
 * Or localStorage `litecode.debug` = `*` | `turn,buffer`, or `?debug=*`.
 */

export const DEBUG_STORAGE_KEY = "litecode.debug";

export const DEBUG_CHANNELS = ["turn", "buffer"] as const;
export type DebugChannel = (typeof DEBUG_CHANNELS)[number];

export function parseDebugSpec(raw: string | null | undefined): Set<DebugChannel> {
  const enabled = new Set<DebugChannel>();
  if (!raw) return enabled;
  const v = raw.trim().toLowerCase();
  if (!v || v === "0" || v === "off" || v === "false") return enabled;
  if (v === "1" || v === "*" || v === "on" || v === "true" || v === "all") {
    for (const ch of DEBUG_CHANNELS) enabled.add(ch);
    return enabled;
  }
  for (const part of v.split(/[,\s]+/)) {
    if ((DEBUG_CHANNELS as readonly string[]).includes(part)) {
      enabled.add(part as DebugChannel);
    }
  }
  return enabled;
}

function readSpec(): string | null {
  try {
    return localStorage.getItem(DEBUG_STORAGE_KEY);
  } catch {
    return null;
  }
}

export function debugEnabled(channel: DebugChannel): boolean {
  return parseDebugSpec(readSpec()).has(channel);
}

export function debugTrace(
  channel: DebugChannel,
  event: string,
  data?: Record<string, unknown>,
): void {
  if (!debugEnabled(channel)) return;
  const prefix = `[litecode:${channel}] ${event}`;
  if (data === undefined) console.info(prefix);
  else console.info(prefix, data);
}

export function setDebugSpec(spec: string | null): void {
  try {
    if (!spec) localStorage.removeItem(DEBUG_STORAGE_KEY);
    else localStorage.setItem(DEBUG_STORAGE_KEY, spec);
  } catch {
    /* ignore quota / private mode */
  }
}

export function installDebugConsole(): void {
  if (typeof window === "undefined") return;
  try {
    const fromUrl = new URLSearchParams(window.location.search).get("debug");
    if (fromUrl !== null) setDebugSpec(fromUrl === "0" ? null : fromUrl);
  } catch {
    /* ignore */
  }
  window.litecodeDebug = {
    on(spec = "*") {
      setDebugSpec(spec);
      console.info("[litecode] debug on", spec);
    },
    off() {
      setDebugSpec(null);
      console.info("[litecode] debug off");
    },
    status() {
      return readSpec();
    },
  };
}
