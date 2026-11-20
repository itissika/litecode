import type { ReactElement } from "react";

import type { FunctionCallOutputItem } from "../../api/types";
import type { ToolStatus } from "../ToolIcon";
import { RunCodeToolView } from "./RunCodeToolView";

export interface ToolViewProps {
  name: string;
  status: ToolStatus;
  input: unknown;
  output?: FunctionCallOutputItem;
  /** Tool call id — optional legacy context. */
  call_id?: string;
  /** Owning session id — optional context. */
  sessionId?: string;
}

/**
 * Per-tool content views, keyed by tool name. Message-list dedicated built-in
 * tool views have been retired in favor of the `run_code` card view plus
 * generic `ToolContentView` fallback for any legacy rows.
 */
const TOOL_VIEWS: Record<
  string,
  (props: ToolViewProps) => ReactElement | null
> = {
  run_code: RunCodeToolView,
};

/** Returns the dedicated view for a tool, or undefined for fallback rendering. */
export function getToolView(
  name: string,
): ((props: ToolViewProps) => ReactElement | null) | undefined {
  return TOOL_VIEWS[name];
}

/**
 * Tools whose view renders the tool output itself (so the dispatcher must NOT
 * also render the default output block).
 */
const OUTPUT_OWNED_BY_VIEW: ReadonlySet<string> = new Set(["run_code"]);

export function viewOwnsOutput(name: string): boolean {
  return OUTPUT_OWNED_BY_VIEW.has(name);
}
