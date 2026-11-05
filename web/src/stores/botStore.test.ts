import { beforeEach, describe, expect, it, vi } from "vitest";

import type { BotInfo } from "../api/types";
import { useBotStore } from "./botStore";
import { useConnectionStore } from "./connectionStore";

function bot(id: string, patch: Partial<BotInfo> = {}): BotInfo {
  return {
    id,
    name: id,
    description: "",
    body: { system_prompt: "", tools: {}, allowed_subagents: [] },
    created_at: 1,
    session_id: `s-${id}`,
    folder_id: null,
    sort_order: 1,
    ...patch,
  };
}

beforeEach(() => {
  useBotStore.setState({ bots: [], botsLoading: false, botsError: null });
  useConnectionStore.setState({ state: "connected" } as never);
  vi.restoreAllMocks();
});

describe("botStore", () => {
  it("lists bots in shared folder order and upserts lifecycle events", async () => {
    const sendRpc = vi.fn(async () => ({
      bots: [bot("b", { sort_order: 2 }), bot("a", { sort_order: 1 })],
    }));
    useConnectionStore.setState({ sendRpc } as never);

    useBotStore.getState().listBots();
    await vi.waitFor(() =>
      expect(useBotStore.getState().bots.map((item) => item.id)).toEqual([
        "a",
        "b",
      ]),
    );

    const created = bot("c", { name: "Created", sort_order: 3 });
    useBotStore.getState().applyBotEvent({ event: "created", bot: created });
    expect(useBotStore.getState().bots.map((item) => item.name)).toContain(
      "Created",
    );

    useBotStore.getState().applyBotEvent({
      event: "updated",
      bot: { ...created, name: "Renamed" },
    });
    expect(
      useBotStore.getState().bots.find((item) => item.id === "c")?.name,
    ).toBe("Renamed");

    useBotStore.getState().applyBotEvent({
      event: "deleted",
      bot_id: "c",
    });
    expect(
      useBotStore.getState().bots.find((item) => item.id === "c"),
    ).toBeUndefined();
  });

  it("sends update/delete/move RPCs and updates the directory", async () => {
    const existing = bot("b", { name: "Before" });
    useBotStore.setState({ bots: [existing] });
    const sendRpc = vi.fn(async (method: string, params: unknown) => {
      if (method === "bot/update") {
        const body = params as { name?: string; folder_id?: string };
        return { bot: { ...existing, ...body } };
      }
      if (method === "bot/delete") return { bot: existing };
      return {};
    });
    useConnectionStore.setState({ sendRpc } as never);

    const updated = await useBotStore
      .getState()
      .updateBot("b", { name: "After" });
    expect(updated?.name).toBe("After");
    expect(useBotStore.getState().bots[0]?.name).toBe("After");

    expect(await useBotStore.getState().moveBot("b", "f1")).toBe(true);
    expect(sendRpc).toHaveBeenCalledWith("bot/update", {
      id: "b",
      folder_id: "f1",
    });

    expect(await useBotStore.getState().deleteBot("b")).toBe(true);
    expect(useBotStore.getState().bots).toEqual([]);
  });
});