import type { IDockviewPanelProps } from "dockview-react";
import { useCallback, useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";

import {
  onTerminalData,
  onTerminalExit,
  registerTerminal,
  terminalClose,
  terminalCreate,
  terminalResize,
  terminalWrite,
  unregisterTerminal,
} from "../../lib/litecodeTerminal";
import { useConnectionStore } from "../../stores/connectionStore";
import { THEME_CHANGE_EVENT } from "../../lib/theme";

// Above this buffer length a column change becomes an expensive reflow, so the
// horizontal axis gets debounced. Below it both axes resize atomically on every
// frame. Mirrors VS Code TerminalResizeDebouncer.StartDebouncingThreshold.
const START_DEBOUNCING_THRESHOLD = 200;
// VS Code TerminalResizeDebouncer.DebounceResizeXDelay.
const DEBOUNCE_RESIZE_X_DELAY = 100;
// Trailing delay for PTY resize signals. Unlike Zed — whose set_size is a
// synchronous in-process call on the same tick as the xterm resize — ours
// crosses a WebSocket RPC. Signalling every drag frame means a
// SIGWINCH storm whose re-render output (PSReadLine redraws the whole prompt
// on every resize) comes back asynchronously and lands at whatever geometry
// xterm has moved on to — duplicated prompt lines piling into scrollback.
// Coalescing to the settled geometry gives one SIGWINCH and one in-place
// redraw per drag burst.
const DEBOUNCE_PTY_RESIZE_DELAY = 100;
// Minimum geometry accepted for spawning the PTY. FitAddon clamps proposals to
// >= 2 cols (Math.max(2, ...)), so a degenerate host — mid-layout, or a
// collapsed edge group whose content is display:none — reports exactly 2 and
// would pass a `cols <= 1` guard. Spawning there produced the "terminal is two
// characters wide" bug; compare against a real floor instead. Any genuinely
// visible panel clears it by a wide margin (bottom edge min height is 100px).
const MIN_SPAWN_COLS = 5;
const MIN_SPAWN_ROWS = 2;

/** Resolve a theme token to its concrete color, falling back if unresolved. */
function readTokenColor(name: string, fallback: string): string {
  const v = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  return v || fallback;
}

/** Build the xterm theme from project tokens so it tracks the active theme. */
function terminalTheme(): { background: string; foreground: string } {
  return {
    background: readTokenColor("--_dk-editor", "#1c1c1c"),
    foreground: readTokenColor("--_dk-text-secondary", "#bcbcbc"),
  };
}

export function TerminalPanel(props: IDockviewPanelProps<{ cwd?: string }>) {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const idRef = useRef<string | null>(null);
  const collapsedRef = useRef(false);
  // Early output (e.g. the shell prompt) that arrives before the terminal id is
  // bound. Buffered so the initial screen isn't lost.
  const pendingRef = useRef("");
  // Generation token: bumped on kill/unmount to invalidate any in-flight create
  // so a terminal spawned just before a collapse never becomes an orphan.
  const reqRef = useRef(0);
  // In-flight create guard: prevents duplicate createTerminal() calls from the
  // several triggers (connect / expand / dimensions) racing into two PTYs.
  const creatingRef = useRef(false);
  // rAF token for the debounced create (see scheduleCreate).
  const rafRef = useRef<number | null>(null);
  // Last geometry actually pushed to the backend PTY; used to skip redundant
  // resizes, which would otherwise make the shell reprint its prompt. This is
  // Zed's set_size contract: only signal the PTY when the GRID size changed,
  // not on every pixel-level wiggle (avoids SIGWINCH spam while dragging).
  const lastColsRef = useRef(-1);
  const lastRowsRef = useRef(-1);
  // Debounced horizontal-resize state, only used once the buffer exceeds
  // START_DEBOUNCING_THRESHOLD (see applyFit): pendingColsRef holds the latest
  // proposed width and colsTimerRef is the trailing timer.
  const pendingColsRef = useRef(-1);
  const colsTimerRef = useRef<number | null>(null);
  // Trailing PTY-resize state (see DEBOUNCE_PTY_RESIZE_DELAY): the latest
  // geometry xterm was resized to, signalled to the backend once it settles.
  const ptyColsRef = useRef(-1);
  const ptyRowsRef = useRef(-1);
  const ptyTimerRef = useRef<number | null>(null);
  const connection = useConnectionStore((s) => s.state);
  // Mirror connection into a ref so the collapse handler reads the live value.
  const connectionRef = useRef(connection);
  connectionRef.current = connection;

  const killTerminal = useCallback(() => {
    reqRef.current++; // supersede any in-flight create
    pendingRef.current = "";
    creatingRef.current = false;
    lastColsRef.current = -1;
    lastRowsRef.current = -1;
    pendingColsRef.current = -1;
    if (colsTimerRef.current != null) {
      clearTimeout(colsTimerRef.current);
      colsTimerRef.current = null;
    }
    ptyColsRef.current = -1;
    ptyRowsRef.current = -1;
    if (ptyTimerRef.current != null) {
      clearTimeout(ptyTimerRef.current);
      ptyTimerRef.current = null;
    }
    const id = idRef.current;
    idRef.current = null;
    if (id) {
      unregisterTerminal(id);
      void terminalClose(id).catch(() => {});
    }
  }, []);

  // Push a geometry change to the backend PTY, but only when it actually
  // changed (grid-level dedupe). A redundant resize makes the shell reprint
  // its prompt — exactly the double-prompt bug we eliminated.
  const pushResize = useCallback((cols: number, rows: number) => {
    if (cols === lastColsRef.current && rows === lastRowsRef.current) return;
    lastColsRef.current = cols;
    lastRowsRef.current = rows;
    const id = idRef.current;
    if (id) void terminalResize(id, cols, rows);
  }, []);

  // The proposed grid size for the container's current pixel size, or null
  // when the host has no usable layout. Three measurement traps handled here:
  //  1. display:none container (collapsed edge group): computed width/height
  //     are 'auto', parseInt makes them NaN, and NaN passes every `<` guard —
  //     so finiteness is checked explicitly.
  //  2. xterm opened while hidden never measured its font (cell size 0x0) and
  //     proposeDimensions() returns undefined forever after; a same-size
  //     resize() re-triggers char measurement (xterm's #785 path), so retry
  //     once when the host is visibly laid out.
  //  3. proposeDimensions() reads getComputedStyle on the xterm parent, which
  //     under Tailwind's global border-box INCLUDES the parent's padding — so
  //     the host must stay padding-free (the p-1 lives on the outer wrapper),
  //     otherwise rows/cols are computed against a box 8px larger than the
  //     area xterm actually fills and the bottom row gets clipped.
  const measureGeometry = useCallback((): { cols: number; rows: number } | null => {
    const term = termRef.current;
    const fit = fitRef.current;
    const host = hostRef.current;
    if (!term || !fit || !host) return null;
    let dims: { cols: number; rows: number } | undefined;
    try {
      dims = fit.proposeDimensions();
    } catch {
      return null;
    }
    if (!dims) {
      if (host.offsetWidth === 0 || host.offsetHeight === 0) return null;
      try {
        term.resize(term.cols, term.rows); // no-op size; forces char re-measure
        dims = fit.proposeDimensions();
      } catch {
        return null;
      }
      if (!dims) return null;
    }
    if (!Number.isFinite(dims.cols) || !Number.isFinite(dims.rows)) return null;
    if (dims.cols < 2 || dims.rows < 1) return null;
    return dims;
  }, []);

  // Apply a grid size to xterm only (no PTY signal). Purely local: trimming or
  // adding blank lines on a rows change is exact, so the canvas can track the
  // panel live on every frame without involving the shell at all.
  const resizeXterm = useCallback((cols: number, rows: number) => {
    const term = termRef.current;
    if (!term) return;
    if (cols === term.cols && rows === term.rows) return;
    try {
      term.resize(cols, rows);
    } catch {
      return;
    }
    term.refresh(0, term.rows - 1);
  }, []);

  // Schedule a TRAILING PTY resize for the latest geometry. The shell only
  // learns the settled size, so a drag produces a single SIGWINCH/redraw at
  // the end instead of a storm of stale-geometry redraws (see
  // DEBOUNCE_PTY_RESIZE_DELAY).
  const schedulePtyResize = useCallback(
    (cols: number, rows: number) => {
      ptyColsRef.current = cols;
      ptyRowsRef.current = rows;
      if (ptyTimerRef.current != null) clearTimeout(ptyTimerRef.current);
      ptyTimerRef.current = window.setTimeout(() => {
        ptyTimerRef.current = null;
        pushResize(ptyColsRef.current, ptyRowsRef.current);
      }, DEBOUNCE_PTY_RESIZE_DELAY);
    },
    [pushResize],
  );

  // Apply a grid size to xterm and schedule the PTY to follow. Desktop shells
  // pair these synchronously on the same tick; our PTY leg is coalesced because
  // the transport is async (see schedulePtyResize).
  const resizeTo = useCallback(
    (cols: number, rows: number) => {
      const term = termRef.current;
      if (!term) return;
      resizeXterm(cols, rows);
      schedulePtyResize(term.cols, term.rows);
    },
    [resizeXterm, schedulePtyResize],
  );

  // Fit the xterm to its container, following the debounce policy above
  // (VS Code TerminalResizeDebouncer, mirrored in the constants):
  //   - small buffer (< 200 lines) or explicit immediate request: resize BOTH
  //     axes in ONE call, atomically, on every frame. Splitting the axes here
  //     is what made dragging repaint garbage.
  //   - large buffer: rows apply immediately (cheap, no reflow), cols are
  //     debounced 100ms (a column change reflows the whole scrollback).
  // In both cases the PTY is signalled separately on a trailing debounce
  // (schedulePtyResize): xterm tracks the panel live while the shell only
  // learns the settled geometry — one SIGWINCH and one in-place prompt
  // redraw per drag burst, instead of a redraw per frame landing at stale
  // coordinates (the duplicated-prompt-lines bug).
  // Triggers: dockview's layout engine (onDidDimensionsChange, pixel-accurate
  // and synchronous with layout) plus a ResizeObserver backstop on the host.
  // The observer is safe here because the host's box is owned by the flex
  // layout (min-h-0 + overflow-hidden), not by xterm's content, so it tracks
  // both grow and shrink and an xterm resize never feeds back into it; it
  // catches changes dockview doesn't signal (window resize, display restore).
  const applyFit = useCallback(
    (immediate = false) => {
      const term = termRef.current;
      if (!term) return;
      const dims = measureGeometry();
      if (!dims) return;

      if (immediate || term.buffer.normal.length < START_DEBOUNCING_THRESHOLD) {
        if (colsTimerRef.current != null) {
          clearTimeout(colsTimerRef.current);
          colsTimerRef.current = null;
        }
        resizeTo(dims.cols, dims.rows);
        return;
      }

      // Large buffer: vertical now (cheap), horizontal debounced (reflow).
      if (dims.rows !== term.rows) {
        if (dims.cols === term.cols) {
          // Pure vertical change: resize xterm and schedule the PTY follow.
          resizeTo(term.cols, dims.rows);
        } else {
          // Corner drag: apply rows locally only — the debounced cols timer
          // below applies both axes and signals the PTY with the final
          // geometry, so the shell never sees an intermediate size the canvas
          // never displayed.
          resizeXterm(term.cols, dims.rows);
        }
      }
      pendingColsRef.current = dims.cols;
      if (colsTimerRef.current != null) clearTimeout(colsTimerRef.current);
      colsTimerRef.current = window.setTimeout(() => {
        colsTimerRef.current = null;
        const t = termRef.current;
        if (!t) return;
        resizeTo(pendingColsRef.current, t.rows);
      }, DEBOUNCE_RESIZE_X_DELAY);
    },
    [measureGeometry, resizeTo, resizeXterm],
  );

  const createTerminal = useCallback(async () => {
    const term = termRef.current;
    if (!term) return;
    if (creatingRef.current || idRef.current) return;
    // The host must have a real, settled size before we spawn the PTY (see
    // MIN_SPAWN_COLS). Bail and let the next dimensions/observer event retry
    // once layout has settled.
    const dims = measureGeometry();
    if (!dims || dims.cols < MIN_SPAWN_COLS || dims.rows < MIN_SPAWN_ROWS) return;
    creatingRef.current = true;
    const myReq = ++reqRef.current;
    try {
      // Size the canvas BEFORE spawning/writing any output. Writing the
      // buffered prompt while the xterm still had a stale width wraps it at
      // that width and reflows it into a doubled/mangled prompt.
      resizeTo(dims.cols, dims.rows);
      const id = await terminalCreate({
        cols: dims.cols,
        rows: dims.rows,
        cwd: props.params?.cwd,
      });
      // Superseded by a collapse / unmount / newer create while awaiting.
      if (myReq !== reqRef.current) {
        await terminalClose(id).catch(() => {});
        return;
      }
      idRef.current = id;
      registerTerminal(id);
      // The container may have moved while the create RPC was in flight; catch
      // up to the settled size before flushing buffered output. We clear (not
      // reset) to drop the previous session's content without desyncing
      // xterm's cursor from the shell's.
      const settled = measureGeometry();
      if (settled && (settled.cols !== term.cols || settled.rows !== term.rows)) {
        resizeTo(settled.cols, settled.rows);
      }
      term.clear();
      // Flush any early output captured before the id was bound. This is the
      // only place the initial prompt is written.
      if (pendingRef.current) {
        term.write(pendingRef.current);
        pendingRef.current = "";
      }
      // Push the final geometry to the backend NOW (not debounced — the shell
      // must know its real size before it draws the first prompt), and cancel
      // any trailing push scheduled while the id was still unbound.
      if (ptyTimerRef.current != null) {
        clearTimeout(ptyTimerRef.current);
        ptyTimerRef.current = null;
      }
      pushResize(term.cols, term.rows);
      term.refresh(0, term.rows - 1);
      term.focus();
    } catch (e) {
      if (myReq === reqRef.current && !collapsedRef.current) {
        term.writeln(
          `\r\n[terminal unavailable] ${e instanceof Error ? e.message : String(e)}`,
        );
      }
    } finally {
      creatingRef.current = false;
    }
  }, [measureGeometry, resizeTo, pushResize, props.params?.cwd]);

  // Create the terminal once the container actually has a real size. The real
  // check is the measured grid (see measureGeometry), not the panel's logical
  // size — the latter is >0 on the first animation frame while the host is
  // still tiny. createTerminal() itself re-checks and bails if not laid out
  // yet, so callers can safely invoke this repeatedly.
  const ensureTerminal = useCallback(() => {
    if (collapsedRef.current) return;
    if (idRef.current || creatingRef.current) return;
    if (connectionRef.current !== "connected") return;
    void createTerminal();
  }, [createTerminal]);

  // Debounce creation to the settled size. During a collapse/expand animation
  // or a drag, dimensions change on every frame; we only want to spawn the PTY
  // once the size has stopped moving, so only the final frame's rAF survives
  // (earlier ones are cancelled).
  const scheduleCreate = useCallback(() => {
    if (rafRef.current != null) cancelAnimationFrame(rafRef.current);
    rafRef.current = requestAnimationFrame(() => {
      rafRef.current = null;
      ensureTerminal();
    });
  }, [ensureTerminal]);

  // xterm setup + data/exit subscriptions — stable for the panel's lifetime.
  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;

    const term = new Terminal({
      cursorBlink: true,
      fontFamily: "JetBrains Mono, ui-monospace, monospace",
      fontSize: 13,
      theme: terminalTheme(),
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(host);
    termRef.current = term;
    fitRef.current = fit;
    // Initial geometry if the panel is already laid out; a no-op when hidden.
    applyFit(true);

    const onData = term.onData((data) => {
      const id = idRef.current;
      if (!id) return;
      void terminalWrite(id, data).catch((e) => {
        term.writeln(`\r\n[write error] ${e instanceof Error ? e.message : String(e)}`);
      });
    });

    const unsubData = onTerminalData((id, data) => {
      if (idRef.current && id === idRef.current) {
        term.write(data);
      } else if (!idRef.current) {
        // Not yet bound (create RPC in flight): stash early output.
        pendingRef.current += data;
        if (pendingRef.current.length > 65536) {
          pendingRef.current = pendingRef.current.slice(-65536);
        }
      }
    });
    const unsubExit = onTerminalExit((id, code) => {
      if (id !== idRef.current) return;
      unregisterTerminal(id);
      idRef.current = null;
      term.writeln(
        `\r\n[process exited${code !== null ? ` with code ${code}` : ""}]`,
      );
    });

    // Primary sizing signal: dockview's layout engine fires this synchronously
    // with every layout tick, grow and shrink, with pixel-accurate values.
    const dDims = props.api.onDidDimensionsChange(() => {
      applyFit();
      scheduleCreate();
    });

    // Backstop: ResizeObserver on the host catches size changes the layout
    // engine doesn't signal (window resize, display:none restore, zoom). The
    // host box is layout-owned (flex + min-h-0 + overflow-hidden), so it
    // reports both directions and never feeds back from xterm resizes.
    const observer = new ResizeObserver(() => {
      applyFit();
      scheduleCreate();
    });
    observer.observe(host);

    // Track live theme switches (the app toggles data-dv-theme + fires this
    // event). Re-read the token colors and push them into xterm so the
    // terminal background/foreground follow the active theme.
    const onThemeChange = () => {
      term.options.theme = terminalTheme();
    };
    window.addEventListener(THEME_CHANGE_EVENT, onThemeChange);

    return () => {
      dDims.dispose();
      observer.disconnect();
      window.removeEventListener(THEME_CHANGE_EVENT, onThemeChange);
      if (rafRef.current != null) cancelAnimationFrame(rafRef.current);
      if (colsTimerRef.current != null) clearTimeout(colsTimerRef.current);
      if (ptyTimerRef.current != null) clearTimeout(ptyTimerRef.current);
      unsubData();
      unsubExit();
      onData.dispose();
      killTerminal();
      term.dispose();
      termRef.current = null;
      fitRef.current = null;
    };
  }, [killTerminal, applyFit, scheduleCreate, props.api]);

  // Collapse / expand (dockview edge-group event) drives the lifecycle:
  // collapsed → kill the current process; expanded → resize immediately to the
  // settled geometry (VS Code flushes pending resizes + re-evaluates on
  // becoming visible) and create a fresh terminal (scheduleCreate debounces to
  // the final animation frame).
  useEffect(() => {
    const groupApi = props.api.group.api;
    collapsedRef.current = groupApi.isCollapsed();
    const d = groupApi.onDidCollapsedChange((e) => {
      collapsedRef.current = e.isCollapsed;
      if (e.isCollapsed) {
        killTerminal();
      } else {
        applyFit(true);
        scheduleCreate();
      }
    });
    return () => d.dispose();
  }, [props.api, killTerminal, applyFit, scheduleCreate]);

  // Connection stability: (re)create only when expanded, connected, and the
  // container is laid out. A WS drop does NOT kill the terminal — the backend
  // PTY stays alive and output resumes on reconnect.
  useEffect(() => {
    if (connection !== "connected") return;
    scheduleCreate();
  }, [connection, scheduleCreate]);

  return (
    // The p-1 lives on the OUTER wrapper: FitAddon measures the host via
    // getComputedStyle, which under border-box includes an element's own
    // padding — padding on the measured host would inflate every proposal.
    <div className="flex h-full min-h-0 w-full flex-col bg-(--_dk-editor) p-1">
      <div
        ref={hostRef}
        className="min-h-0 flex-1 overflow-hidden"
        onDragOver={(e) => {
          e.preventDefault();
          e.dataTransfer.dropEffect = "copy";
        }}
        onDrop={(e) => {
          e.preventDefault();
          const text = e.dataTransfer.getData("text/plain");
          const id = idRef.current;
          if (text && id) void terminalWrite(id, text);
        }}
      />
    </div>
  );
}
