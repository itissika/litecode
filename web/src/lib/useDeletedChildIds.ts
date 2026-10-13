import { useMemo } from "react";
import { useMessageStore } from "../stores/messageStore";
import { useSessionStore } from "../stores/sessionStore";
import { deletedChildIds } from "./subagentUi";

/** WP-086: tombstoned child ids under `parentId` (binding states ∪ deleted events). */
export function useDeletedChildIds(parentId: string | undefined): Set<string> {
  const states = useMessageStore((s) =>
    parentId ? s.bySession.get(parentId)?.subagentBindingStates : undefined,
  );
  const deletedIds = useSessionStore((s) => s.deletedSessionIds);
  return useMemo(
    () => deletedChildIds(states, deletedIds),
    [states, deletedIds],
  );
}
