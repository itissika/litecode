import {
  useEffect,
  useRef,
  useState,
  type CSSProperties,
  type ReactElement,
} from "react";
import { UsersThree } from "@phosphor-icons/react";

import type { AgentRunState } from "../api/types";
import { agentColor } from "./agentIdentity";
import type { ToolStatus } from "./ToolIcon";

/**
 * Subagent presence icon for the foldcard header of a `subagent_launch` card.
 *
 * The glyph stands in for the agent itself: its base colour is the agent's
 * accent (`--_sa-color`), held across pre-run / running / ok-settle — unlike
 * the generic ToolIcon, whose colour follows the tool's ok/warn/fail status.
 *
 * Phase machine (local state driven by prop EDGES — never by mount, so a
 * virtual-list remount replays none of the one-shots):
 *   - pre-run (live, child turn not running) : static agent colour, no glow
 *   - idle → running                          : ok-style pop burst once, then
 *                                              breathing glow + gentle bob
 *   - running → ok (sealed with output)       : ok-style pop once more, then
 *                                              settle back to static agent
 *   - running → failed / never ran → failed   : generic red failure (shrink +
 *                                              shockwave), then static red
 *
 * The DOM reuses the `.tool-icon` machinery (pop/fail/shockwave keyframes and
 * their tuned timings), with `--_sa-color` substituted for the emerald default
 * via `.sa-presence`. Breathing classes live in the same chat.css block.
 *
 * Warning reuses the generic amber variant; subagent results never produce one
 * (only a free-text "Warning:" prefix would), so it exists purely defensively.
 */

/** Pop burst length matches tool-pop/tool-glow (700ms). */
const POP_MS = 700;
/** Failure animation length matches tool-fail + shockwave tail (900ms). */
const FAIL_MS = 900;

type OneShot = "enter" | "ok" | "fail" | null;

interface SubagentStatusIconProps {
  /** Agent id — source of the accent colour. Undefined (still streaming args
   *  or an invalid launch) renders the generic glyph in muted grey. */
  agent?: string;
  /** Tool-card work-live flag: false once the call is sealed (output or
   *  terminal failure). Mirrors ToolIcon's `streaming` semantics. */
  live: boolean;
  /** Sealed tool status, used to pick the settle colour/animation. */
  status: ToolStatus;
  /** Child session run state — "running"/"cancelling" drives the live phase. */
  runState: AgentRunState;
}

export function SubagentStatusIcon({
  agent,
  live,
  status,
  runState,
}: SubagentStatusIconProps): ReactElement {
  const running = runState === "running" || runState === "cancelling";
  const color = agent ? agentColor(agent) : undefined;

  // One-shot animation currently playing ("enter" = just started running,
  // "ok"/"fail" = just sealed). Null while static or breathing.
  const [shot, setShot] = useState<OneShot>(null);
  const clearTimer = useRef<number | null>(null);
  const prevLive = useRef(live);
  const prevRunning = useRef(running);

  // Fire one-shots on transitions only: a remount (virtualizer recycle, page
  // reload replay) starts with prev == current and stays static/breathing.
  useEffect(() => {
    const wasLive = prevLive.current;
    prevLive.current = live;
    const wasRunning = prevRunning.current;
    prevRunning.current = running;

    let next: OneShot = null;
    if (wasLive && !live) {
      // Sealed: pick the settle feedback from the derived tool status.
      // Warning reuses the ok pop but flashes amber via `.sa-warn` vars.
      next =
        status === "failed"
          ? "fail"
          : status === "ok" || status === "warning"
            ? "ok"
            : null;
    } else if (!wasRunning && running && live) {
      next = "enter";
    }
    if (!next) return;

    if (clearTimer.current !== null) {
      window.clearTimeout(clearTimer.current);
    }
    setShot(next);
    clearTimer.current = window.setTimeout(
      () => setShot(null),
      next === "fail" ? FAIL_MS : POP_MS,
    );
    return () => {
      if (clearTimer.current !== null) {
        window.clearTimeout(clearTimer.current);
        clearTimer.current = null;
      }
    };
  }, [live, running, status]);

  const sealed = !live;
  // One-shots take over the glyph/glow for their duration; breathing only
  // resumes once the entering pop has finished ("跳完开始呼吸").
  const breathing = !sealed && running && shot === null;
  const popping = shot === "enter" || shot === "ok";
  const failing = shot === "fail";

  const colorClass =
    sealed && status === "failed"
      ? "sa-failed"
      : sealed && status === "warning"
        ? "sa-warn"
        : "";
  const cls = [
    "tool-icon",
    "sa-presence",
    breathing ? "sa-running" : "",
    popping ? "tool-icon--pop" : "",
    failing ? "tool-icon--fail-anim" : "",
    colorClass,
  ]
    .filter(Boolean)
    .join(" ");

  const title = sealed
    ? status === "failed"
      ? "Failed"
      : status === "ok" || status === "warning"
        ? "Completed"
        : "Finished"
    : running
      ? "Running"
      : "Launching";

  return (
    <span
      className={cls}
      style={color ? ({ "--_sa-color": color } as CSSProperties) : undefined}
      title={title}
      aria-hidden
    >
      <UsersThree size={12} weight="fill" className="tool-icon-glyph" />
      {popping && <span className="tool-icon-glow" />}
      {failing && <span className="tool-icon-shockwave" />}
    </span>
  );
}
