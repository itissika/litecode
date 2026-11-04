import { create } from "zustand";

import type { BotChanged, BotInfo } from "../api/types";
import type { AgentProfile } from "../api/settings";
import { attachSiblingStores, useConnectionStore } from "./connectionStore";
import { useToastStore } from "./toastStore";

export interface BotUpdateInput {
  name?: string;
  description?: string;
  profile?: AgentProfile;
  folder_id?: string;
  clear_folder?: boolean;
  sort_order?: number;
}

interface BotState {
  bots: BotInfo[];
  botsLoading: boolean;
  botsError: string | null;
  listBots: () => void;
  createBot: (
    name: string,
    description: string,
    templateAgentId: string,
  ) => Promise<BotInfo | null>;
  updateBot: (id: string, update: BotUpdateInput) => Promise<BotInfo | null>;
  deleteBot: (id: string) => Promise<boolean>;
  moveBot: (id: string, folderId: string | null) => Promise<boolean>;
  openBot: (id: string) => Promise<string | null>;
  applyBotEvent: (event: BotChanged) => void;
}

function sortBots(bots: BotInfo[]): BotInfo[] {
  return [...bots].sort(
    (left, right) =>
      (left.sort_order ?? 0) - (right.sort_order ?? 0) ||
      left.created_at - right.created_at ||
      left.name.localeCompare(right.name),
  );
}

function upsert(bots: BotInfo[], bot: BotInfo): BotInfo[] {
  return sortBots([...bots.filter((item) => item.id !== bot.id), bot]);
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/** Invalidates in-flight directory reads after a local mutation. */
let botListGeneration = 0;

export const useBotStore = create<BotState>((set, get) => ({
  bots: [],
  botsLoading: false,
  botsError: null,

  listBots: () => {
    const generation = ++botListGeneration;
    set({ botsLoading: true });
    useConnectionStore
      .getState()
      .sendRpc<{ bots?: BotInfo[] }>("bot/list")
      .then((result) => {
        if (generation !== botListGeneration) return;
        set({
          bots: sortBots(result.bots ?? []),
          botsLoading: false,
          botsError: null,
        });
      })
      .catch((error: unknown) => {
        if (generation !== botListGeneration) return;
        set({ botsLoading: false, botsError: errorMessage(error) });
      });
  },

  createBot: async (name, description, templateAgentId) => {
    try {
      const result = await useConnectionStore
        .getState()
        .sendRpc<{ bot: BotInfo }>("bot/create", {
          name,
          description,
          agent_id: templateAgentId,
        });
      botListGeneration += 1;
      set((state) => ({
        bots: upsert(state.bots, result.bot),
        botsLoading: false,
        botsError: null,
      }));
      return result.bot;
    } catch (error) {
      useToastStore.getState().showToast(errorMessage(error), "error");
      return null;
    }
  },

  updateBot: async (id, update) => {
    try {
      const result = await useConnectionStore
        .getState()
        .sendRpc<{ bot: BotInfo }>("bot/update", { id, ...update });
      botListGeneration += 1;
      set((state) => ({
        bots: upsert(state.bots, result.bot),
        botsLoading: false,
        botsError: null,
      }));
      return result.bot;
    } catch (error) {
      useToastStore.getState().showToast(errorMessage(error), "error");
      return null;
    }
  },

  deleteBot: async (id) => {
    try {
      await useConnectionStore
        .getState()
        .sendRpc<{ bot: BotInfo }>("bot/delete", { id });
      botListGeneration += 1;
      set((state) => ({
        bots: state.bots.filter((bot) => bot.id !== id),
        botsLoading: false,
        botsError: null,
      }));
      return true;
    } catch (error) {
      useToastStore.getState().showToast(errorMessage(error), "error");
      return false;
    }
  },

  moveBot: async (id, folderId) => {
    const bot = get().bots.find((item) => item.id === id);
    if (!bot) return false;
    const updated = await get().updateBot(
      id,
      folderId ? { folder_id: folderId } : { clear_folder: true },
    );
    return updated !== null;
  },

  openBot: async (id) => {
    try {
      const result = await useConnectionStore
        .getState()
        .sendRpc<{ session_id: string; bot: BotInfo }>("bot/open", { id });
      botListGeneration += 1;
      set((state) => ({
        bots: upsert(state.bots, result.bot),
        botsLoading: false,
      }));
      return result.session_id;
    } catch (error) {
      useToastStore.getState().showToast(errorMessage(error), "error");
      return null;
    }
  },

  applyBotEvent: (event) => {
    if (event.event === "deleted") {
      if (!event.bot_id) return;
      set((state) => ({
        bots: state.bots.filter((bot) => bot.id !== event.bot_id),
      }));
      return;
    }
    if (!event.bot) return;
    set((state) => ({ bots: upsert(state.bots, event.bot!) }));
  },
}));

attachSiblingStores({ bot: useBotStore });
