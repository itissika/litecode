import {
  BracketsCurlyIcon,
  CodeIcon,
  FilePlusIcon,
  FileTextIcon,
  FilesIcon,
  GlobeIcon,
  HourglassIcon,
  ListChecksIcon,
  MagnifyingGlassIcon,
  PencilIcon,
  PlugsConnectedIcon,
  PuzzlePieceIcon,
  StopIcon,
  StrategyIcon,
  TerminalIcon,
  UsersIcon,
  WrenchIcon,
} from "@phosphor-icons/react";
import { useEffect, useRef, useState } from "react";

export type ToolStatus = "running" | "ok" | "warning" | "failed" | "unknown";

type Glyph = typeof WrenchIcon;

// Glyph per built-in tool name. `mcp_*` tools share PlugsConnected; unknown
// tools fall back to a generic wrench.
const NAME_GLYPH: Record<string, Glyph> = {
  bash: TerminalIcon,
  wait_shell: HourglassIcon,
  kill_shell: StopIcon,
  read: FileTextIcon,
  write: FilePlusIcon,
  edit: PencilIcon,
  grep: MagnifyingGlassIcon,
  glob: FilesIcon,
  todo: ListChecksIcon,
  plan: StrategyIcon,
  code_search: CodeIcon,
  lsp: BracketsCurlyIcon,
  subagent: UsersIcon,
  subagent_launch: UsersIcon,
  webfetch: GlobeIcon,
  custom: PuzzlePieceIcon,
};

function glyphFor(name: string): Glyph {
  if (name in NAME_GLYPH) return NAME_GLYPH[name];
  if (name.startsWith("mcp_")) return PlugsConnectedIcon;
  return WrenchIcon;
}

/**
 * Debug toggle: when true, the icon randomly replays the transition animations
 * on a timer so the effect can be tuned without waiting for a real
 * running→ok / running→fail change. Set back to `false` for production.
 */
const DEBUG_RANDOM_ANIM = false;

type ToolAnim = "pop" | "fail" | null;

/**
 * Tool glyph with two static states and one-shot feedback:
 *   - ok / unknown / running → static theme-foreground colour (no animation)
 *   - failed                → static failure red
 *   - settling to ok        → bounce + green flash once, then settle to static
 *   - settling to failed    → shrink + red flash + shockwave once, then red
 *
 * The animation fires exactly once: when `streaming` (tool-level work-live)
 * transitions from true → false, meaning the tool item has sealed. The
 * animation class is derived from `status` at that moment. After the
 * transition, any remount (FoldCard expand / virtualizer recycle) stays
 * static because `streaming` is already false and no transition occurs.
 */
export function ToolIcon({
  name,
  status,
  streaming = false,
}: {
  name: string;
  status: ToolStatus;
  streaming?: boolean;
}) {
  const Glyph = glyphFor(name);
  const [anim, setAnim] = useState<ToolAnim>(null);
  const clearTimer = useRef<number | null>(null);
  const prevStreaming = useRef(streaming);

  useEffect(() => {
    if (clearTimer.current !== null) {
      window.clearTimeout(clearTimer.current);
      clearTimer.current = null;
    }

    if (DEBUG_RANDOM_ANIM) {
      const tick = () => setAnim(Math.random() < 0.5 ? "pop" : "fail");
      tick();
      const id = window.setInterval(tick, 1800);
      return () => window.clearInterval(id);
    }

    const wasStreaming = prevStreaming.current;
    prevStreaming.current = streaming;

    // Only fire on the true→false transition (item sealed).
    if (!wasStreaming || streaming) {
      return;
    }

    if (status === "ok") {
      setAnim("pop");
      clearTimer.current = window.setTimeout(() => setAnim(null), 700);
    } else if (status === "failed") {
      setAnim("fail");
      clearTimer.current = window.setTimeout(() => setAnim(null), 900);
    } else {
      setAnim(null);
    }
    return () => {
      if (clearTimer.current !== null) {
        window.clearTimeout(clearTimer.current);
        clearTimer.current = null;
      }
    };
  }, [streaming, status]);

  const colorClass =
    status === "failed"
      ? "tool-icon--fail"
      : status === "warning"
        ? "tool-icon--warn"
        : "tool-icon--ok";
  const animClass =
    anim === "pop" ? "tool-icon--pop" : anim === "fail" ? "tool-icon--fail-anim" : "";

  return (
    <span className={`tool-icon ${colorClass} ${animClass}`} aria-hidden>
      <Glyph size={12} weight="fill" className="tool-icon-glyph" />
      {anim === "pop" && <span className="tool-icon-glow" />}
      {anim === "fail" && <span className="tool-icon-shockwave" />}
    </span>
  );
}
