import type { ReactElement } from "react";

import type { FunctionCallOutputItem } from "../../api/types";
import type { ToolStatus } from "../ToolIcon";
import { BashToolView } from "./BashToolView";
import { EditToolView } from "./EditToolView";
import { FileParamView } from "./FileParamView";
import { KillShellToolView } from "./KillShellToolView";
import { SubagentLaunchToolView } from "./SubagentLaunchToolView";
import { SubagentListToolView } from "./SubagentListToolView";
import { SubagentSendToolView } from "./SubagentSendToolView";
import { SubagentStopToolView } from "./SubagentStopToolView";
import { SubagentWaitToolView } from "./SubagentWaitToolView";
import { WaitShellToolView } from "./WaitShellToolView";
import { WriteToolView } from "./WriteToolView";

export interface ToolViewProps {
  name: string;
  status: ToolStatus;
  input: unknown;
  output?: FunctionCallOutputItem;
  /** Tool call id — used by views (e.g. subagent) to resolve nested state. */
  call_id?: string;
  /** Owning session id — used by views to look up their bindings. */
  sessionId?: string;
}

/**
 * Per-tool content views, keyed by tool name. The single extension point for
 * covered tools — add a view here plus an entry in `paramMeta.ts`. Tools without
 * an entry fall back to the default input/output rendering in `ToolContentView`.
 */
export const TOOL_VIEWS: Record<
  string,
  (props: ToolViewProps) => ReactElement | null
> = {
    read: FileParamView,
    write: WriteToolView,
    edit: EditToolView,
    bash: BashToolView,
    wait_shell: WaitShellToolView,
    kill_shell: KillShellToolView,
    subagent_launch: SubagentLaunchToolView,
    subagent_wait: SubagentWaitToolView,
    subagent_stop: SubagentStopToolView,
    subagent_send: SubagentSendToolView,
    subagent_list: SubagentListToolView,
  };

/** Returns the dedicated view for a tool, or undefined for fallback rendering. */
export function getToolView(
  name: string,
): ((props: ToolViewProps) => ReactElement | null) | undefined {
  return TOOL_VIEWS[name];
}

/**
 * Tools whose view renders the tool output itself (so the dispatcher must NOT
 * also render the default output block). Everything else — including the
 * file-path views — leaves output to the default renderer.
 */
const OUTPUT_OWNED_BY_VIEW: ReadonlySet<string> = new Set([
  "bash",
  "wait_shell",
  "kill_shell",
  "subagent_launch",
  "subagent_wait",
  "subagent_stop",
  "subagent_send",
  "subagent_list",
  "edit",
  "write",
]);

export function viewOwnsOutput(name: string): boolean {
  return OUTPUT_OWNED_BY_VIEW.has(name);
}
