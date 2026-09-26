import { functionCallOutputText } from "../../api/adapter";
import { AgentMarkdown } from "../AgentMarkdown";
import type { ToolViewProps } from "./registry";

/**
 * Expanded body of `litecode_workspace`. The tool already speaks Markdown
 * (headings, lists, the button table). Render it with the same component as
 * assistant text, at the compact size tool cards already define.
 */
export function WorkspaceToolView({ status, output }: ToolViewProps) {
  const raw = output ? functionCallOutputText(output) : "";
  if (!raw) return null;
  const failed = status === "failed";
  return (
    <div
      className={`tool-card-markdown ${
        failed ? "text-(--_dk-red-500)" : "text-(--_dk-text-primary)"
      }`}
      data-testid="workspace-view"
    >
      <AgentMarkdown text={raw} />
    </div>
  );
}
