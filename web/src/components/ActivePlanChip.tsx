import { StrategyIcon } from "@phosphor-icons/react";

import { normalizeToolFilePath } from "../api/adapter";
import { useEditorStore } from "../stores/editorStore";
import { useSessionStore } from "../stores/sessionStore";
import { useTurnStore } from "../stores/turnStore";
import { composerCardClass } from "./composerCard";
import { useChipEntrance } from "./useChipEntrance";

/** Compact session-plan affordance. It exists only while the plan is active.
 *  Shared dock-chip animation (fade + expand) keeps it consistent with the
 *  terminal status chip. */
export function ActivePlanChip({ sessionId }: { sessionId: string }) {
  const activePlanPath = useTurnStore(
    (s) => s.byId.get(sessionId)?.activePlanPath ?? null,
  );
  const projectRoot = useSessionStore((s) => s.project);
  const openFile = useEditorStore((s) => s.openFile);
  const { mounted, open } = useChipEntrance(activePlanPath != null);

  if (!mounted || !activePlanPath) return null;

  return (
    <button
      type="button"
      className={`dock-chip ${composerCardClass} flex h-[30px] w-[30px] shrink-0 cursor-pointer items-center justify-center overflow-hidden text-(--_dk-text-secondary) hover:scale-105 active:scale-90 active:brightness-90 ${open ? "is-open" : ""}`}
      aria-label="Open active plan"
      title="Open active plan"
      onClick={() => {
        const path = normalizeToolFilePath(activePlanPath, projectRoot);
        if (path) void openFile(path);
      }}
    >
      <StrategyIcon size={14} weight="fill" aria-hidden />
    </button>
  );
}
