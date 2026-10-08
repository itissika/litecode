import type { IDockviewHeaderActionsProps } from "dockview-react";

import { addBrowserPanel, showBrowserAddButton } from "../../lib/browserPanel";
import { groupRole } from "../workbench";
import { readGroup } from "../workbench/readGroup";
import { getPanelIcon } from "../tabs/icons";

export function BrowserHeaderAction(props: IDockviewHeaderActionsProps) {
  const BrowserIcon = getPanelIcon("browser");
  const hasBridge = typeof window.litecode?.browserCreate === "function";
  const role = groupRole(readGroup(props.group));
  if (!showBrowserAddButton(role, hasBridge)) return null;

  return (
    <div className="flex h-full items-center px-1">
      <button
        type="button"
        className="flex h-5 w-5 items-center justify-center rounded text-sm leading-none text-(--_dk-text-muted) transition-colors hover:bg-(--_dk-ix-bg-hover) hover:text-(--_dk-text-secondary)"
        aria-label="New Browser"
        title="New Browser"
        onPointerDown={(event) => event.stopPropagation()}
        onClick={() => addBrowserPanel(props.group.api.id)}
      >
        <BrowserIcon size={14} weight="regular" aria-hidden />
      </button>
    </div>
  );
}
