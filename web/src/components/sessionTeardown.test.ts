import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { userTextItem } from "../api/adapter";
import type { HumanRow } from "../api/types";
import { setDockviewApi, useConnectionStore } from "../stores/connectionStore";
import { emptySlice as emptyMessageSlice, useMessageStore } from "../stores/messageStore";
import { useNotificationStore } from "../stores/notificationStore";
import { emptySlice as emptyTurnSlice, useTurnStore } from "../stores/turnStore";
import { clearFoldCardOpen, getFoldCardOpenIntent, setFoldCardOpenIntent } from "./foldCardState";
import { releaseSessionTab, releaseSubagentCard, teardownSession } from "./sessionTeardown";
import { holdSubagentRoster, resetSubagentRosterHolds } from "./subagentRosterHolds";

const CHILD = "child-a";
const CARD_ID = `${CHILD}:bubble:tool:call_1`;

const row = (seq: number, text: string): HumanRow => ({
  seq,
  kind: "item/user",
  body: userTextItem(text),
});

/** Every piece of per-session state a teardown may or may not drop. */
function seedSessionState(): ReturnType<typeof vi.fn> {
  const sendRpc = vi.fn(async () => ({}));
  useConnectionStore.setState({
    state: "connected",
    sendRpc,
    subscribedSessions: new Set([CHILD]),
  } as never);

  useMessageStore.setState((s) => {
    const bySession = new Map(s.bySession);
    bySession.set(CHILD, {
      ...emptyMessageSlice(),
      bySeq: new Map([[0, row(0, "do the thing")]]),
      messages: [row(0, "do the thing")],
      display: [row(0, "do the thing")],
      fromSeq: 0,
      toSeq: 1,
      hydrated: true,
    });
    return { bySession };
  });

  useTurnStore.setState((s) => {
    const byId = new Map(s.byId);
    byId.set(CHILD, { ...emptyTurnSlice(), runState: "running" });
    return { byId };
  });

  useNotificationStore.getState().add(CHILD, "stale toast");
  setFoldCardOpenIntent(CARD_ID, "keepopen");
  return sendRpc;
}

const messageRows = () =>
  useMessageStore.getState().bySession.get(CHILD)?.messages.length ?? 0;
const turnRunState = () =>
  useTurnStore.getState().byId.get(CHILD)?.runState ?? "idle";
const unsubscribed = (sendRpc: ReturnType<typeof vi.fn>) =>
  sendRpc.mock.calls.some(([method]) => method === "session/unsubscribe");

beforeEach(() => {
  setDockviewApi(null);
});

afterEach(() => {
  resetSubagentRosterHolds();
  setDockviewApi(null);
  useConnectionStore.setState({
    state: "disconnected",
    subscribedSessions: new Set(),
  });
  useMessageStore.getState().reset(CHILD);
  useTurnStore.getState().resetTurn(CHILD);
  useNotificationStore.getState().reset(CHILD);
  clearFoldCardOpen(CHILD);
});

describe("sessionTeardown — path 1: tab closes, no roster card holds it", () => {
  it("drops everything: subscription, notification, FoldCard intents, both slices", () => {
    const sendRpc = seedSessionState();

    releaseSessionTab(CHILD);

    expect(unsubscribed(sendRpc)).toBe(true);
    expect(useNotificationStore.getState().bySession.has(CHILD)).toBe(false);
    expect(getFoldCardOpenIntent(CARD_ID)).toBe("none");
    expect(messageRows()).toBe(0);
    expect(turnRunState()).toBe("idle");
  });
});

describe("sessionTeardown — path 2: tab closes while an expanded card holds it", () => {
  it("touches nothing — the card keeps the stream and every slice", () => {
    const sendRpc = seedSessionState();
    holdSubagentRoster(CHILD);

    releaseSessionTab(CHILD);

    expect(unsubscribed(sendRpc)).toBe(false);
    expect(useConnectionStore.getState().subscribedSessions.has(CHILD)).toBe(true);
    expect(useNotificationStore.getState().bySession.has(CHILD)).toBe(true);
    expect(getFoldCardOpenIntent(CARD_ID)).toBe("keepopen");
    expect(messageRows()).toBe(1);
    expect(turnRunState()).toBe("running");
  });
});

describe("sessionTeardown — path 3: roster card collapses with no tab (P6)", () => {
  it("unsubscribes and clears surface state but KEEPS the message/turn slices", () => {
    const sendRpc = seedSessionState();

    releaseSubagentCard(CHILD);

    expect(unsubscribed(sendRpc)).toBe(true);
    expect(useConnectionStore.getState().subscribedSessions.has(CHILD)).toBe(false);
    // The review leak: these two used to be skipped on the card path.
    expect(useNotificationStore.getState().bySession.has(CHILD)).toBe(false);
    expect(getFoldCardOpenIntent(CARD_ID)).toBe("none");
    // P6: the child's context stays resident for an incremental re-expand.
    expect(messageRows()).toBe(1);
    expect(turnRunState()).toBe("running");
  });
});

describe("sessionTeardown — path 4: roster card collapses while the tab owns the session", () => {
  it("touches nothing — the tab is still rendering the child", () => {
    const sendRpc = seedSessionState();
    setDockviewApi({
      getPanel: (id: string) => (id === `agent-${CHILD}` ? {} : undefined),
    } as never);

    releaseSubagentCard(CHILD);

    expect(unsubscribed(sendRpc)).toBe(false);
    expect(useNotificationStore.getState().bySession.has(CHILD)).toBe(true);
    expect(getFoldCardOpenIntent(CARD_ID)).toBe("keepopen");
    expect(messageRows()).toBe(1);
    expect(turnRunState()).toBe("running");
  });
});

describe("sessionTeardown core", () => {
  it("always releases the subscription and surface state, projection optional", () => {
    const sendRpc = seedSessionState();

    teardownSession(CHILD, { dropProjection: false });

    expect(unsubscribed(sendRpc)).toBe(true);
    expect(useNotificationStore.getState().bySession.has(CHILD)).toBe(false);
    expect(getFoldCardOpenIntent(CARD_ID)).toBe("none");
    expect(messageRows()).toBe(1);
    expect(turnRunState()).toBe("running");
  });
});
