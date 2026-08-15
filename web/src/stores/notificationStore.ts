import { create } from "zustand";

export type NotificationKind = "info" | "error" | "success";

export interface NotificationItem {
  id: string;
  message: string;
  kind: NotificationKind;
  at: number; // timestamp ms
}

interface SessionNotifications {
  items: NotificationItem[];
  lastSeen: number;
}

const EMPTY_ITEMS: NotificationItem[] = [];

function emptySlice(): SessionNotifications {
  return { items: [], lastSeen: 0 };
}

interface NotificationState {
  bySession: Map<string, SessionNotifications>;
}

interface NotificationActions {
  add: (sessionId: string, message: string, kind?: NotificationKind) => void;
  clear: (sessionId: string) => void;
  markSeen: (sessionId: string) => void;
  reset: (sessionId: string) => void;
}

export const useNotificationStore = create<NotificationState & NotificationActions>(
  (set, get) => ({
    bySession: new Map(),

    add: (sessionId, message, kind = "info") => {
      if (!sessionId) return;
      const item: NotificationItem = {
        id: `${Date.now()}-${Math.random().toString(36).slice(2, 7)}`,
        message,
        kind,
        at: Date.now(),
      };
      const bySession = new Map(get().bySession);
      const slice = bySession.get(sessionId) ?? emptySlice();
      bySession.set(sessionId, { ...slice, items: [...slice.items, item] });
      set({ bySession });
    },

    clear: (sessionId) => {
      const bySession = new Map(get().bySession);
      bySession.set(sessionId, emptySlice());
      set({ bySession });
    },

    markSeen: (sessionId) => {
      const prev = get().bySession.get(sessionId);
      if (!prev) return;
      const bySession = new Map(get().bySession);
      bySession.set(sessionId, { ...prev, lastSeen: prev.items.length });
      set({ bySession });
    },

    reset: (sessionId) => {
      if (!get().bySession.has(sessionId)) return;
      const bySession = new Map(get().bySession);
      bySession.delete(sessionId);
      set({ bySession });
    },
  }),
);

export function sessionNotificationItems(
  bySession: Map<string, SessionNotifications>,
  sessionId: string,
): NotificationItem[] {
  return bySession.get(sessionId)?.items ?? EMPTY_ITEMS;
}

export function sessionNotificationLastSeen(
  bySession: Map<string, SessionNotifications>,
  sessionId: string,
): number {
  return bySession.get(sessionId)?.lastSeen ?? 0;
}
