import { create } from "zustand";

import { nativeViewKey, type NativeViewKind } from "../lib/overlayGeometry";

export type NativeViewPlaceholder = {
  /** Empty when the page could not be captured; the host still gets a blocker. */
  dataUrl: string;
};

interface NativeViewOcclusionStore {
  placeholders: Record<string, NativeViewPlaceholder>;
  setPlaceholder: (kind: NativeViewKind, id: string, dataUrl: string) => void;
  clearPlaceholder: (kind: NativeViewKind, id: string) => void;
  clearAll: () => void;
}

export const useNativeViewOcclusionStore = create<NativeViewOcclusionStore>((set) => ({
  placeholders: {},

  setPlaceholder: (kind, id, dataUrl) =>
    set((state) => {
      const key = nativeViewKey(kind, id);
      if (state.placeholders[key]?.dataUrl === dataUrl) return state;
      return { placeholders: { ...state.placeholders, [key]: { dataUrl } } };
    }),

  clearPlaceholder: (kind, id) =>
    set((state) => {
      const key = nativeViewKey(kind, id);
      if (!(key in state.placeholders)) return state;
      const next = { ...state.placeholders };
      delete next[key];
      return { placeholders: next };
    }),

  clearAll: () => set({ placeholders: {} }),
}));

export function resetNativeViewOcclusionStoreForTests(): void {
  useNativeViewOcclusionStore.getState().clearAll();
}
