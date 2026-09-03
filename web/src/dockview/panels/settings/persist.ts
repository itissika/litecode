import { useCallback } from "react";
import { useSettingsStore } from "../../../stores/settingsStore";
import type { PersistDocKey } from "../../../stores/settingsDocuments";
import type { PersistStatus } from "../../../lib/settingsPersist";

export {
  SettingsPersistController,
  flushRegisteredSettings,
  isPersistBusy,
  registerSettingsFlush,
  SETTINGS_PERSIST_ERROR_CHANNEL,
  shouldHydrateDraftFromStore,
  useSettingsPersist,
  type PersistStatus,
  type SerializeResult,
  type SettingsPersistOptions,
} from "../../../lib/settingsPersist";

export function useDocPersist(doc: PersistDocKey): {
  persistStatus: PersistStatus;
  setPersistStatus: (status: PersistStatus) => void;
} {
  const persistStatus = useSettingsStore((s) => s.persistByDoc[doc] ?? "idle");
  const setPersistStatus = useCallback(
    (status: PersistStatus) => {
      useSettingsStore.getState().setPersistStatus(doc, status);
    },
    [doc],
  );
  return { persistStatus, setPersistStatus };
}
