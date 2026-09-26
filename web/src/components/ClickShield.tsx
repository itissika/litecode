import { useSyncExternalStore } from "react";

import { areClicksLocked, subscribeClickLock } from "../lib/firstRun";

/**
 * Swallows pointer input for the one-second pause between the key-saved
 * celebration and the move to Agents. Keyboard input is left alone.
 * Sits above the settings dialog (`z-[9999]`) and menus (`z-[10000]`).
 */
export function ClickShield() {
  const locked = useSyncExternalStore(
    subscribeClickLock,
    areClicksLocked,
    () => false,
  );
  if (!locked) return null;
  return (
    <div
      className="fixed inset-0 z-[10001] cursor-default"
      aria-hidden
    />
  );
}
