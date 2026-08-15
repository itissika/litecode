import {
  useCallback,
  useEffect,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import { WrenchIcon } from "@phosphor-icons/react";
import { useSessionsPanelVisible } from "../stores/sessionsPanelVisibility";

type StepKind = "reasoning" | "toolcall" | "text";

/** Expressive emoji pool for the idle placeholder (pop-in on trigger). */
const EMOJIS = [
  "😀", "😎", "🤔", "😴", "🥳", "😇",
  "🙃", "🤩", "🥰", "😺", "😏", "🤖",
];

/** The three entrance animation sequences; one is matched at random per trigger. */
const ENTRANCE_CLASSES = ["lp-anim-1", "lp-anim-2", "lp-anim-3"];

/** Emoji shrink-out duration. */
const EXIT_MS = 200;
/** Recap shrink-out duration (playing before returning to idle). */
const RECAP_EXIT_MS = 300;
/** "finished" fresh-recap hold before it dims into "waiting". */
const FINISHED_MS = 4000;
/** "waiting" hold before the recap plays its exit and the slot returns to idle. */
const WAIT_MS = 30000;

/**
 * Non-linear trigger probability as a function of idle time (seconds since the
 * last update). Clamped to the [1min, 5min] window:
 *   - below 1 min: 0 (too soon to interrupt)
 *   - above 5 min: 0 (dead window)
 *   - within: convex curve, p = ((5 - m)/4)^3 — high near 1 min, hard drop to 0
 *     at 5 min (large curvature, "弧度大" per spec).
 */
function triggerProbability(secsSinceUpdate: number): number {
  const m = secsSinceUpdate / 60;
  if (m < 1 || m > 5) return 0;
  const x = (5 - m) / 4; // 1 at m=1, 0 at m=5
  return x * x * x;
}

/** ── Debug toggle: enable from the console to loop the entrance animations. ──
 *  window.__livePreviewDebug.enabled = true   // preview effects
 *  window.__livePreviewDebug.enabled = false  // back to normal behavior
 */
let debugEnabled = false;
const debugListeners = new Set<() => void>();
function setDebug(v: boolean): void {
  if (v !== debugEnabled) {
    debugEnabled = v;
    debugListeners.forEach((l) => l());
  }
}
function useDebug(): boolean {
  return useSyncExternalStore(
    (cb) => {
      debugListeners.add(cb);
      return () => debugListeners.delete(cb);
    },
    () => debugEnabled,
    () => debugEnabled,
  );
}
if (typeof window !== "undefined") {
  (window as unknown as { __livePreviewDebug: { enabled: boolean } }).__livePreviewDebug = {
    get enabled() {
      return debugEnabled;
    },
    set enabled(v: boolean) {
      setDebug(v);
    },
  };
}

function pick<T>(arr: T[]): T {
  return arr[Math.floor(Math.random() * arr.length)];
}

interface LivePreviewProps {
  /** Accumulated turn-step kinds for the current turn (client-only, not persisted). */
  stepKinds?: StepKind[] | null;
  /** Whether the session's turn is currently in progress. */
  running: boolean;
  /** `updated_at` (ms) — base for the idle-time probability curve. */
  updatedAt: number;
  /** Shared wall-clock tick (ms) from the parent's 1min timer. */
  now: number;
}

type Phase = "idle" | "running" | "finished" | "waiting" | "exiting";

/**
 * Live-summary slot for a session row. The slot is a FIXED width (see
 * `.lp-slot` in live-preview.css) so it never needs to measure itself or trim
 * icons — that removed the ResizeObserver ↔ flex-allocation feedback loop that
 * caused icon flicker when width was tight.
 *
 * The slot shows exactly two things:
 *   - tool calls: a single wrench glyph + count (the only live step we surface)
 *   - emoji:      an idle placeholder, triggered probabilistically when idle
 *
 * Phase machine (local state, driven by `running` + `toolCount` + timers — NOT
 * by a separate stored flag):
 *
 *   idle ──turn_start──▶ running ──turn_finished──▶ finished
 *     ▲                       │                        │ (FINISHED_MS)
 *     │                       │                        ▼
 *     │                       │                     waiting (WAIT_MS, dimmed)
 *     │                       │                        │
 *     │                       │                        ▼ (RECAP_EXIT_MS)
 *     └────────turn_start───────────────────────── exiting ──▶ idle
 *
 * - idle:        a placeholder emoji, TRIGGERED by a non-linear probability roll
 *                (1–5 min idle) when the sessions panel is visible.
 * - running:     the live tool-call glyph + count.
 * - finished:    recap held at full opacity right after the turn ends.
 * - waiting:     same recap, dimmed, counting down to exit.
 * - exiting:     recap plays its shrink-out, then the slot returns to idle.
 *
 * Emoji and the tool glyph are ABSOLUTELY mutually exclusive: the glyph shows
 * whenever `toolCount > 0` (running/finished/waiting/exiting); emoji only
 * appears once the recap has fully exited back to idle. A new turn_start
 * cancels the pending exit timers and jumps straight to running with no exit
 * animation.
 */
export function LivePreview({ stepKinds, running, updatedAt, now }: LivePreviewProps) {
  const panelVisible = useSessionsPanelVisible();
  const debug = useDebug();

  // ── emoji state ──
  const [render, setRender] = useState(false);
  const [exiting, setExiting] = useState(false);
  const [emoji, setEmoji] = useState(() => pick(EMOJIS));
  const [animClass, setAnimClass] = useState(ENTRANCE_CLASSES[0]);
  const [animKey, setAnimKey] = useState(0);

  // ── phase ──
  const [phase, setPhase] = useState<Phase>("idle");

  const toolCount = (stepKinds ?? []).filter((k) => k === "toolcall").length;
  const active = toolCount > 0;

  // Refs mirror state for use inside effects without re-subscribing.
  const bubbledRef = useRef(false);
  const renderRef = useRef(false);
  const exitingRef = useRef(false);
  const exitTimer = useRef<number | null>(null);
  const waitTimers = useRef<number[]>([]);

  const clearWaitTimers = useCallback(() => {
    waitTimers.current.forEach((t) => window.clearTimeout(t));
    waitTimers.current = [];
  }, []);

  const doTrigger = useCallback(() => {
    if (exitTimer.current !== null) {
      window.clearTimeout(exitTimer.current);
      exitTimer.current = null;
    }
    setEmoji(pick(EMOJIS));
    setAnimClass(pick(ENTRANCE_CLASSES));
    setExiting(false);
    setRender(true);
    setAnimKey((k) => k + 1);
    bubbledRef.current = true;
    renderRef.current = true;
    exitingRef.current = false;
  }, []);

  const startExit = useCallback(() => {
    setExiting(true);
    exitingRef.current = true;
    exitTimer.current = window.setTimeout(() => {
      setRender(false);
      setExiting(false);
      renderRef.current = false;
      exitingRef.current = false;
      bubbledRef.current = false; // re-arm for the next idle stint
      exitTimer.current = null;
    }, EXIT_MS);
  }, []);

  // Phase machine — re-derived whenever running / tool count / debug change.
  useEffect(() => {
    if (debug) return;
    clearWaitTimers();
    // A turn in progress is "running" even before its first step is recorded
    // (step_kinds is wiped on turn_start). This avoids an idle flicker.
    if (running) {
      setPhase("running");
      return;
    }
    if (active) {
      // Turn just ended: hold the recap (finished → waiting → exiting → idle).
      setPhase("finished");
      const t1 = window.setTimeout(() => setPhase("waiting"), FINISHED_MS);
      const t2 = window.setTimeout(() => setPhase("exiting"), FINISHED_MS + WAIT_MS);
      waitTimers.current = [t1, t2];
      return;
    }
    // No tool calls at all → idle.
    setPhase("idle");
  }, [running, toolCount, debug, clearWaitTimers]);

  // Play the recap exit, then return to idle (emoji can take over).
  useEffect(() => {
    if (phase !== "exiting") return;
    const t = window.setTimeout(() => setPhase("idle"), RECAP_EXIT_MS);
    return () => window.clearTimeout(t);
  }, [phase]);

  // When a turn starts (leaving idle), shrink any showing emoji out.
  useEffect(() => {
    if (debug) return;
    if (phase !== "idle" && renderRef.current && !exitingRef.current) {
      startExit();
    }
  }, [phase, debug, startExit]);

  // Normal emoji trigger evaluation — runs every tick (and on phase/step changes).
  useEffect(() => {
    if (debug) return; // debug drives its own continuous loop
    if (active) {
      // A turn is active: ensure any idle emoji has shrunk out.
      if (renderRef.current && !exitingRef.current) startExit();
      return;
    }
    if (phase === "idle" && panelVisible && !bubbledRef.current && !renderRef.current) {
      if (Math.random() < triggerProbability(now - updatedAt)) doTrigger();
    }
  }, [now, panelVisible, active, phase, debug, doTrigger, startExit]);

  // Debug loop — continuously cycle the entrance animations for visual preview.
  useEffect(() => {
    if (!debug) return;
    doTrigger();
    const id = window.setInterval(() => doTrigger(), 1700);
    return () => {
      window.clearInterval(id);
      bubbledRef.current = false;
    };
  }, [debug, doTrigger]);

  // Cleanup any pending timers on unmount.
  useEffect(() => {
    return () => {
      if (exitTimer.current !== null) window.clearTimeout(exitTimer.current);
      clearWaitTimers();
    };
  }, [clearWaitTimers]);

  // Debug bypasses the phase machine and just loops the emoji + waiting dots.
  if (debug) {
    return (
      <span key={animKey} className="lp-slot lp-emoji-group" aria-hidden>
        <span className={`lp-emoji ${animClass}`}>{emoji}</span>
        <span className="lp-dots">
          <span />
          <span />
          <span />
        </span>
      </span>
    );
  }

  // Single tool-call glyph + count. Keyed by count so the icon pop + glow
  // replay together each time the count increments.
  const toolNode = (
    <span key="tool" className="lp-step">
      <span key={`tool-${toolCount}`} className="lp-step-icon-wrap">
        <span className="lp-step-glow" aria-hidden />
        <WrenchIcon size={12} className="lp-step-icon" aria-hidden />
      </span>
      {toolCount > 1 && (
        <span className="lp-step-count">
          ×
          <span className="lp-count-wrap">
            <span key={toolCount} className="lp-count-num">
              {toolCount}
            </span>
          </span>
        </span>
      )}
    </span>
  );

  // Exiting recap: keep it mounted briefly for the shrink-out animation.
  if (phase === "exiting") {
    return (
      <span className="lp-slot lp-steps lp-step-exit" aria-hidden>
        {toolNode}
      </span>
    );
  }

  // Active / finished / waiting: show the tool-call glyph + count.
  if (phase === "running" || phase === "finished" || phase === "waiting") {
    return (
      <span
        className={`lp-slot lp-steps${phase === "waiting" ? " is-stale" : ""}`}
        aria-hidden
      >
        {toolNode}
      </span>
    );
  }

  // Exiting emoji (shrink-out) — independent of step state.
  if (exiting) {
    return (
      <span key={animKey} className="lp-slot lp-emoji lp-exit" aria-hidden>
        {emoji}
      </span>
    );
  }

  // Idle + triggered: show the emoji with its random entrance animation,
  // followed by "..." dots that appear one by one then fade (ride the emoji in).
  if (render) {
    return (
      <span key={animKey} className="lp-slot lp-emoji-group" aria-hidden>
        <span className={`lp-emoji ${animClass}`}>{emoji}</span>
        <span className="lp-dots">
          <span />
          <span />
          <span />
        </span>
      </span>
    );
  }

  // Idle + not yet triggered: empty fixed-width slot (keeps the row stable).
  return <span className="lp-slot" aria-hidden />;
}
