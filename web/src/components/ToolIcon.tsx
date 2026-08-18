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
  PlugsIcon,
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

// Glyph per built-in tool name. `mcp_*` tools share Plugs; unknown tools fall
// back to a generic wrench.
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
  if (name.startsWith("mcp_")) return PlugsIcon;
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
 * The animation fires when status is ok/failed *and* `live` is true (the
 * owning process group is still the streaming phase). A completed call often
 * mounts directly at `ok` after buffer seal rather than transitioning out of
 * `running`; `live` is what distinguishes that from a remount of a sealed card
 * (FoldCard expand / virtualizer recycle), which must stay static.
 */
export function ToolIcon({
  name,
  status,
  live = false,
}: {
  name: string;
  status: ToolStatus;
  live?: boolean;
}) {
  const Glyph = glyphFor(name);
  const [anim, setAnim] = useState<ToolAnim>(null);
  const clearTimer = useRef<number | null>(null);

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

    if (!live) {
      setAnim(null);
      return;
    }

    if (status === "ok") {
      setAnim("pop");
      clearTimer.current = window.setTimeout(() => setAnim(null), 700);
    } else if (status === "warning") {
      setAnim(null);
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
  }, [status, live]);

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
