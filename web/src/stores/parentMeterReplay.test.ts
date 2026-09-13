import { describe, it, expect } from "vitest";
import fs from "node:fs";
import path from "node:path";
import { useConnectionStore } from "./connectionStore";
import { useSessionStore } from "./sessionStore";
import { useTurnStore } from "./turnStore";
import { useMessageStore } from "./messageStore";
import type { WireEnvelope } from "../api/agentWs";

// Replays REAL frames captured from the running backend (web/probe-frames.json,
// written by web/probe.mjs) through the live dispatch chain, then asserts the
// parent session's turn slice — the exact path the browser exercises after a
// mid-turn reload: session/attached + session/snapshot hydrate runState + ring.
const frames: { method: string; params: unknown }[] = JSON.parse(
  fs.readFileSync(path.resolve(__dirname, "../../probe-frames.json"), "utf-8"),
);

describe("replay: real backend frames hydrate parent turn state", () => {
  it("snapshot/attached frames produce running + ring values", () => {
    for (const f of frames) {
      useConnectionStore
        .getState()
        .dispatchEnvelope({ jsonrpc: "2.0", method: f.method, params: f.params } as WireEnvelope);
    }

    const snapFrame = frames.find((f) => f.method === "session/snapshot");
    const sid = (snapFrame?.params as { session_id?: string } | undefined)?.session_id;
    expect(sid).toBeTruthy();

    const slice = useTurnStore.getState().byId.get(sid!);
    expect(slice, "turn slice exists for running parent").toBeTruthy();
    expect(slice!.runState).toBe("running");
    expect(slice!.contextWindow).toBeGreaterThan(0);
    expect(slice!.contextTokensEstimate).toBeGreaterThan(0);
    expect(slice!.lastTurnPromptTokens).toBeGreaterThan(0);

    const sess = useSessionStore.getState().byId.get(sid!);
    expect(sess?.activePrimary).toBeTruthy();
    expect(useMessageStore.getState().bySession.get(sid!)).toBeTruthy();
  });
});
