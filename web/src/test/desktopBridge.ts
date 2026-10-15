import type { LitecodeDesktopBridge } from "../api/auth";

/** Tests stub only the bridge methods they exercise; the contract type is complete. */
export function stubDesktopBridge(
  methods: Partial<LitecodeDesktopBridge>,
): LitecodeDesktopBridge {
  return methods as LitecodeDesktopBridge;
}
