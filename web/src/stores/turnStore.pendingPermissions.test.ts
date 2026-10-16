/**
 * WP-184 — pending_permissions hydrate / disconnect / turn boundary (Phase A)
 * and waiting_for_user wire / remount presentation (Phase B).
 * Fixture is a copy of `.litecode/ops/artifacts/WP-181/fixtures/pending_permissions.frozen.json`
 * plus WP-186 shape notes (do not invent another wire shape).
 */
import { beforeEach, describe, expect, it, vi } from "vitest";

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import type {
  PermissionRequest,
  SessionSnapshot,
  TurnFinished,
  TurnPhase,
} from "../api/types";
import {
  isWaitingForUserPhase,
  waitingForUserPayload,
} from "../api/types";
import {
  deriveSessionStatus,
  isAwaitingPermission,
} from "../components/SessionStatusDot";
import { useConnectionStore } from "./connectionStore";
import { useToastStore } from "./toastStore";
import { EMPTY_SLICE, useTurnStore } from "./turnStore";

vi.stubGlobal(
  "window",
  Object.assign(globalThis, {
    requestAnimationFrame: (fn: FrameRequestCallback) => {
      fn(0);
      return 0;
    },
    cancelAnimationFrame: () => {},
  }),
);

/** Vitest-safe path (avoid import.meta.url scheme issues). */
const fixturePath = join(
  typeof __dirname !== "undefined"
    ? __dirname
    : dirname(fileURLToPath(import.meta.url)),
  "../test/fixtures/pending_permissions.frozen.json",
);

/** Copy of WP-181 frozen fixture — do not invent another shape. */
const frozen = JSON.parse(readFileSync(fixturePath, "utf8")) as {
  pending_permissions: PermissionRequest[];
  waiting_for_user: boolean;
  awaiting_permission_derived: string;
};

const fixturePerms = frozen.pending_permissions;

function snapshot(sessionId: string): SessionSnapshot {
  return {
    session_id: sessionId,
    project: "/proj",
    agent_id: "default",
    api_model_id: "m",
    buffer: { last_seq: -1, next_seq: 0, revision: 0 },
    turn: null,
    context_window: 0,
  };
}

describe("WP-184 pending_permissions (Phase A)", () => {
  beforeEach(() => {
    useTurnStore.setState({ byId: new Map() });
    useToastStore.setState({ toasts: [] });
    useConnectionStore.setState({
      sendRpc: vi.fn(async (_method: string) => ({})),
    } as never);
  });

  it("frozen fixture shape: ask_user + permission, kind required, same session/turn", () => {
    expect(fixturePerms).toHaveLength(2);
    expect(fixturePerms[0]!.kind).toBe("ask_user");
    expect(fixturePerms[0]!.questions).toHaveLength(2);
    expect(fixturePerms[1]!.kind).toBe("permission");
    expect(fixturePerms[1]!.questions).toEqual([]);
    expect(fixturePerms[0]!.session_id).toBe(fixturePerms[1]!.session_id);
    expect(fixturePerms[0]!.turn_id).toBe(fixturePerms[1]!.turn_id);
    expect(frozen.waiting_for_user).toBe(true);
  });

  it("applySnapshotMeter rebuilds permissionQueue from pending_permissions", () => {
    const sessionId = fixturePerms[0]!.session_id;
    useTurnStore.getState().applySnapshotMeter(sessionId, {
      ...snapshot(sessionId),
      pending_permissions: fixturePerms,
    });
    const q = useTurnStore.getState().byId.get(sessionId)!.permissionQueue;
    expect(q).toHaveLength(2);
    expect(q[0]!.request_id).toBe("req_ask_user_001");
    expect(q[0]!.kind).toBe("ask_user");
    expect(q[0]!.questions.map((x) => x.id)).toEqual(["q_env", "q_region"]);
    expect(q[1]!.request_id).toBe("req_permission_001");
    expect(q[1]!.kind).toBe("permission");
    expect(q[1]!.tool).toBe("bash");
  });

  it("absent pending_permissions key leaves live queue untouched", () => {
    const sessionId = "sess_absent_key";
    const live = [
      {
        turn_id: "t",
        request_id: "live-card",
        tool: "bash",
        rule_id: "r",
        summary: "from push",
        kind: "permission" as const,
        free_text: false,
        options: [],
        multi_select: false,
        questions: [],
      },
    ];
    useTurnStore.setState({
      byId: new Map([
        [sessionId, { ...EMPTY_SLICE, permissionQueue: live }],
      ]),
    });
    // snapshot() helper omits the key entirely (pre-WP-181).
    useTurnStore.getState().applySnapshotMeter(sessionId, snapshot(sessionId));
    expect(
      useTurnStore.getState().byId.get(sessionId)!.permissionQueue,
    ).toEqual(live);
  });

  it("present pending_permissions: [] clears the live queue", () => {
    const sessionId = "sess_present_empty";
    useTurnStore.setState({
      byId: new Map([
        [
          sessionId,
          {
            ...EMPTY_SLICE,
            permissionQueue: [
              {
                turn_id: "t",
                request_id: "stale",
                tool: "bash",
                rule_id: "r",
                summary: "old",
                kind: "permission",
                free_text: false,
                options: [],
                multi_select: false,
                questions: [],
              },
            ],
          },
        ],
      ]),
    });
    useTurnStore.getState().applySnapshotMeter(sessionId, {
      ...snapshot(sessionId),
      pending_permissions: [],
    });
    expect(
      useTurnStore.getState().byId.get(sessionId)!.permissionQueue,
    ).toEqual([]);
  });

  it("live onPermissionRequest upserts same request_id after snapshot hydrate", () => {
    const sessionId = fixturePerms[0]!.session_id;
    useTurnStore.getState().applySnapshotMeter(sessionId, {
      ...snapshot(sessionId),
      pending_permissions: fixturePerms,
    });
    useTurnStore.getState().onPermissionRequest(sessionId, {
      ...fixturePerms[0]!,
      summary: "Choose deploy target (updated)",
    });
    const q = useTurnStore.getState().byId.get(sessionId)!.permissionQueue;
    expect(q).toHaveLength(2);
    expect(q[0]!.summary).toBe("Choose deploy target (updated)");
    expect(q[1]!.request_id).toBe("req_permission_001");
  });

  it("cards are queue[0] only — ask_user is head of frozen fixture", () => {
    const sessionId = fixturePerms[0]!.session_id;
    useTurnStore.getState().applySnapshotMeter(sessionId, {
      ...snapshot(sessionId),
      pending_permissions: fixturePerms,
    });
    const head =
      useTurnStore.getState().byId.get(sessionId)!.permissionQueue[0]!;
    expect(head.kind).toBe("ask_user");
    expect(head.request_id).toBe("req_ask_user_001");
  });

  it("kind=permission head works when ask_user is not first", () => {
    const sessionId = "sess_perm_head";
    const onlyPerm = [fixturePerms[1]!];
    useTurnStore.getState().applySnapshotMeter(sessionId, {
      ...snapshot(sessionId),
      pending_permissions: onlyPerm,
    });
    const head =
      useTurnStore.getState().byId.get(sessionId)!.permissionQueue[0]!;
    expect(head.kind).toBe("permission");
    expect(head.tool).toBe("bash");
  });

  it("disconnect path never sends agent/permission deny; queue kept until snapshot overwrite", () => {
    const sessionId = fixturePerms[0]!.session_id;
    const sendRpc = vi.fn(async (_method: string) => ({}));
    useConnectionStore.setState({ sendRpc } as never);
    useTurnStore.getState().applySnapshotMeter(sessionId, {
      ...snapshot(sessionId),
      pending_permissions: fixturePerms,
    });
    expect(
      useTurnStore.getState().byId.get(sessionId)!.permissionQueue,
    ).toHaveLength(2);

    // Simulate transport drop: FE must not deny pending asks (WP-176 / WP-184).
    // connectionStore only clears subscribedSessions; it does not touch the queue.
    useConnectionStore.setState({
      state: "reconnecting",
      subscribedSessions: new Set(),
    } as never);
    expect(sendRpc).not.toHaveBeenCalled();
    expect(
      useTurnStore.getState().byId.get(sessionId)!.permissionQueue,
    ).toHaveLength(2);

    // Reconnect hydrate: snapshot overwrites (e.g. only permission remains).
    useTurnStore.getState().applySnapshotMeter(sessionId, {
      ...snapshot(sessionId),
      pending_permissions: [fixturePerms[1]!],
    });
    const q = useTurnStore.getState().byId.get(sessionId)!.permissionQueue;
    expect(q.map((p) => p.request_id)).toEqual(["req_permission_001"]);
    expect(sendRpc).not.toHaveBeenCalledWith(
      "agent/permission",
      expect.anything(),
    );
  });

  it("turn_started clears permissionQueue", () => {
    const sessionId = "sess_started";
    useTurnStore.getState().applySnapshotMeter(sessionId, {
      ...snapshot(sessionId),
      pending_permissions: fixturePerms.map((p) => ({
        ...p,
        session_id: sessionId,
      })),
    });
    useTurnStore.getState().onTurnStarted({
      session_id: sessionId,
      turn_id: "turn_new",
      input: "hi",
      step_max: 8,
    });
    expect(
      useTurnStore.getState().byId.get(sessionId)!.permissionQueue,
    ).toEqual([]);
  });

  it("turn_finished clears permissionQueue (BE guarantees waiting ≠ finished)", () => {
    const sessionId = fixturePerms[0]!.session_id;
    useTurnStore.setState({
      byId: new Map([
        [
          sessionId,
          {
            ...EMPTY_SLICE,
            runState: "running",
            currentTurnId: fixturePerms[0]!.turn_id,
            permissionQueue: [
              {
                turn_id: fixturePerms[0]!.turn_id,
                request_id: "req_ask_user_001",
                tool: "ask_user",
                rule_id: "ask_user",
                summary: "x",
                kind: "ask_user",
                free_text: false,
                options: [],
                multi_select: false,
                questions: [],
              },
            ],
          },
        ],
      ]),
    });
    const tf: TurnFinished = {
      turn_id: fixturePerms[0]!.turn_id,
      reason: "completed",
      final_text: "done",
      error: null,
      snapshot: snapshot(sessionId),
      session_id: sessionId,
    };
    useTurnStore.getState().onTurnFinished(tf);
    const slice = useTurnStore.getState().byId.get(sessionId)!;
    expect(slice.runState).toBe("idle");
    expect(slice.permissionQueue).toEqual([]);
  });

  it("Stop/cancel clears queue and sends agent/cancel only (no permission deny)", async () => {
    const sessionId = "sess_stop";
    const sendRpc = vi.fn(async (_method: string) => ({}));
    useConnectionStore.setState({ sendRpc } as never);
    useTurnStore.setState({
      byId: new Map([
        [
          sessionId,
          {
            ...EMPTY_SLICE,
            runState: "running",
            currentTurnId: "t1",
            permissionQueue: [
              {
                turn_id: "t1",
                request_id: "req_x",
                tool: "bash",
                rule_id: "bash.exec",
                summary: "Run",
                kind: "permission",
                free_text: false,
                options: [],
                multi_select: false,
                questions: [],
              },
            ],
          },
        ],
      ]),
    });
    useTurnStore.getState().cancel(sessionId);
    expect(
      useTurnStore.getState().byId.get(sessionId)!.permissionQueue,
    ).toEqual([]);
    expect(sendRpc).toHaveBeenCalledWith("agent/cancel", {
      session_id: sessionId,
    });
    expect(
      sendRpc.mock.calls.some((c) => c[0] === "agent/permission"),
    ).toBe(false);
  });

  it("only grantPermission (Cancel/Deny/Allow) sends agent/permission", async () => {
    const sessionId = "sess_grant";
    const sendRpc = vi.fn(async (_method: string) => ({}));
    useConnectionStore.setState({ sendRpc } as never);
    useTurnStore.getState().onPermissionRequest(sessionId, {
      ...fixturePerms[1]!,
      session_id: sessionId,
    });
    useTurnStore.getState().grantPermission(sessionId, false, false);
    await vi.waitFor(() => {
      expect(sendRpc).toHaveBeenCalledWith(
        "agent/permission",
        expect.objectContaining({
          request_id: "req_permission_001",
          approved: false,
          always: false,
          session_id: sessionId,
        }),
      );
    });
  });
});

describe("WP-184 list-dot awaiting derivation", () => {
  it("trusts explicit true / false; derives only when undefined", () => {
    expect(isAwaitingPermission(true, 0)).toBe(true);
    expect(isAwaitingPermission(false, 5)).toBe(false);
    expect(isAwaitingPermission(undefined, 0)).toBe(false);
    expect(isAwaitingPermission(undefined, 2)).toBe(true);
  });
});

describe("WP-184 Phase B — waiting_for_user wire + remount (WP-186)", () => {
  beforeEach(() => {
    useTurnStore.setState({ byId: new Map() });
    useToastStore.setState({ toasts: [] });
    useConnectionStore.setState({
      sendRpc: vi.fn(async (_method: string) => ({})),
    } as never);
  });

  const waitingPhase: TurnPhase = {
    waiting_for_user: {
      tool: "ask_user",
      rule_id: "ask_user",
      summary: "Choose deploy target",
    },
  };

  const legacyPhase: TurnPhase = {
    awaiting_permission: {
      tool: "bash",
      rule_id: "bash.exec",
      summary: "Run: grep -R 'TODO' src",
    },
  };

  it("isWaitingForUserPhase accepts waiting_for_user and legacy awaiting_permission", () => {
    expect(isWaitingForUserPhase(waitingPhase)).toBe(true);
    expect(isWaitingForUserPhase(legacyPhase)).toBe(true);
    expect(isWaitingForUserPhase("streaming")).toBe(false);
    expect(isWaitingForUserPhase({ failed: { code: "internal" as never } })).toBe(
      false,
    );
    expect(waitingForUserPayload(waitingPhase)?.tool).toBe("ask_user");
    expect(waitingForUserPayload(legacyPhase)?.tool).toBe("bash");
  });

  it("remount/hydrate with WP-186-shaped snapshot remounts cards + pending status", () => {
    const sessionId = fixturePerms[0]!.session_id;
    const turnId = fixturePerms[0]!.turn_id;
    // Same order as sessionStore hydrate: turn then meter.
    // WP-186: pending_permissions always present; phase key waiting_for_user;
    // bool field name stays awaiting_permission (derived from list length).
    const turn = {
      turn_id: turnId,
      phase: waitingPhase,
      step: 1,
      step_max: 8,
      started_at_ms: 1,
      awaiting_permission: true,
    };
    useTurnStore.getState().applySnapshotTurn(sessionId, turn);
    useTurnStore.getState().applySnapshotMeter(sessionId, {
      ...snapshot(sessionId),
      pending_permissions: fixturePerms,
      turn,
    });
    const slice = useTurnStore.getState().byId.get(sessionId)!;
    expect(slice.permissionQueue).toHaveLength(2);
    expect(slice.permissionQueue[0]!.request_id).toBe("req_ask_user_001");
    expect(slice.permissionQueue[1]!.request_id).toBe("req_permission_001");
    expect(isWaitingForUserPhase(slice.turnPhase)).toBe(true);
    expect(waitingForUserPayload(slice.turnPhase)?.tool).toBe("ask_user");

    const status = deriveSessionStatus(
      { running: true, status: "idle", turn },
      { pendingPermissionCount: slice.permissionQueue.length },
    );
    expect(status).toBe("pending");
  });

  it("explicit pending_permissions: [] clears queue; waiting_for_user gone → not pending", () => {
    const sessionId = fixturePerms[0]!.session_id;
    useTurnStore.getState().applySnapshotMeter(sessionId, {
      ...snapshot(sessionId),
      pending_permissions: fixturePerms,
    });
    expect(
      useTurnStore.getState().byId.get(sessionId)!.permissionQueue,
    ).toHaveLength(2);

    // WP-186 always writes [] when idle — present key clears.
    useTurnStore.getState().applySnapshotMeter(sessionId, {
      ...snapshot(sessionId),
      pending_permissions: [],
      turn: null,
    });
    expect(
      useTurnStore.getState().byId.get(sessionId)!.permissionQueue,
    ).toEqual([]);

    expect(
      deriveSessionStatus(
        {
          running: false,
          turn: {
            turn_id: "t",
            phase: "idle",
            step: 0,
            step_max: 8,
            started_at_ms: 0,
            awaiting_permission: false,
          },
        },
        { pendingPermissionCount: 0 },
      ),
    ).toBe("idle");
  });

  it("deriveSessionStatus: waiting_for_user phase → pending even if bool false", () => {
    expect(
      deriveSessionStatus(
        {
          running: true,
          turn: {
            turn_id: "t",
            phase: waitingPhase,
            step: 1,
            step_max: 8,
            started_at_ms: 1,
            awaiting_permission: false,
          },
        },
        { pendingPermissionCount: 0 },
      ),
    ).toBe("pending");
  });

  it("deriveSessionStatus: legacy awaiting_permission phase object → pending", () => {
    expect(
      deriveSessionStatus(
        {
          running: true,
          turn: {
            turn_id: "t",
            phase: legacyPhase,
            step: 1,
            step_max: 8,
            started_at_ms: 1,
          },
        },
        { pendingPermissionCount: 0 },
      ),
    ).toBe("pending");
  });

  it("frozen fixture still documents waiting_for_user + derived awaiting_permission", () => {
    expect(frozen.waiting_for_user).toBe(true);
    expect(frozen.awaiting_permission_derived).toContain(
      "pending_permissions.length",
    );
    expect(frozen.pending_permissions).toHaveLength(2);
  });
});
