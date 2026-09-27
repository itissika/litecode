import { useCallback, useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebglAddon } from "@xterm/addon-webgl";
import "@xterm/xterm/css/xterm.css";

import {
  bindTerminal,
  discardTerminal,
  terminalClose,
  terminalCreate,
  terminalResize,
  terminalWrite,
  trackCommandLine,
  useTerminalTabs,
} from "../../../lib/litecodeTerminal";
import { useConnectionStore } from "../../../stores/connectionStore";
import { THEME_CHANGE_EVENT } from "../../../lib/theme";

// Above this buffer length a column change becomes an expensive reflow, so the
// horizontal axis gets debounced. Below it both axes resize atomically on every
// frame. Mirrors VS Code TerminalResizeDebouncer.StartDebouncingThreshold.
const START_DEBOUNCING_THRESHOLD = 200;
// VS Code TerminalResizeDebouncer.DebounceResizeXDelay.
const DEBOUNCE_RESIZE_X_DELAY = 100;
// Trailing delay for PTY resize signals. The shell only learns the settled
// geometry, so a drag produces one SIGWINCH instead of a redraw per frame.
const DEBOUNCE_PTY_RESIZE_DELAY = 100;
// Minimum geometry accepted for spawning the PTY. FitAddon clamps proposals to
// >= 2 cols, so a collapsed host would otherwise pass a `cols <= 1` guard.
const MIN_SPAWN_COLS = 5;
const MIN_SPAWN_ROWS = 2;

/** Resolve a theme token to its concrete color, falling back if unresolved. */
function readTokenColor(name: string, fallback: string): string {
  const v = getComputedStyle(document.documentElement)
    .getPropertyValue(name)
    .trim();
  return v || fallback;
}

/** Build the xterm theme from project tokens so it tracks the active theme. */
function terminalTheme(): { background: string; foreground: string } {
  return {
    background: readTokenColor("--_dk-editor", "#1c1c1c"),
    foreground: readTokenColor("--_dk-text-secondary", "#bcbcbc"),
  };
}

export function TerminalInstance({
  tabKey,
  cwd,
  active,
  expanded,
  onExited,
}: {
  tabKey: string;
  cwd?: string;
  active: boolean;
  expanded: boolean;
  onExited: () => void;
}) {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const idRef = useRef<string | null>(null);
  const unbindRef = useRef<(() => void) | null>(null);
  // True once a pty has been bound. A later reconnect reprints [reconnected]
  // and starts a fresh shell in the same scrollback. The server kills the
  // pty when the socket drops, so there is nothing to reattach.
  const hadSessionRef = useRef(false);
  const expandedRef = useRef(expanded);
  const activeRef = useRef(active);
  const cwdRef = useRef(cwd);
  const onExitedRef = useRef(onExited);
  const tabKeyRef = useRef(tabKey);
  expandedRef.current = expanded;
  activeRef.current = active;
  cwdRef.current = cwd;
  onExitedRef.current = onExited;
  tabKeyRef.current = tabKey;

  // Generation token: bumped when a create is abandoned so the late RPC
  // cannot bind an orphan pty.
  const reqRef = useRef(0);
  // Interactive line the tab's label summarizes: keystrokes (not shell output)
  // are the only command source a raw pty offers.
  const commandLineRef = useRef("");
  const creatingRef = useRef(false);
  const fitRafRef = useRef<number | null>(null);
  const lastColsRef = useRef(-1);
  const lastRowsRef = useRef(-1);
  const pendingColsRef = useRef(-1);
  const colsTimerRef = useRef<number | null>(null);
  const ptyColsRef = useRef(-1);
  const ptyRowsRef = useRef(-1);
  const ptyTimerRef = useRef<number | null>(null);
  const connection = useConnectionStore((s) => s.state);
  const connectionRef = useRef(connection);
  const prevConnectionRef = useRef(connection);
  connectionRef.current = connection;

  const clearResizeTimers = useCallback(() => {
    if (colsTimerRef.current != null) {
      clearTimeout(colsTimerRef.current);
      colsTimerRef.current = null;
    }
    if (ptyTimerRef.current != null) {
      clearTimeout(ptyTimerRef.current);
      ptyTimerRef.current = null;
    }
    if (fitRafRef.current != null) {
      cancelAnimationFrame(fitRafRef.current);
      fitRafRef.current = null;
    }
  }, []);

  const detachLocal = useCallback(() => {
    reqRef.current++;
    creatingRef.current = false;
    clearResizeTimers();
    lastColsRef.current = -1;
    lastRowsRef.current = -1;
    pendingColsRef.current = -1;
    ptyColsRef.current = -1;
    ptyRowsRef.current = -1;
    unbindRef.current?.();
    unbindRef.current = null;
    idRef.current = null;
  }, [clearResizeTimers]);

  const killTerminal = useCallback(() => {
    const id = idRef.current;
    detachLocal();
    hadSessionRef.current = false;
    if (id) {
      discardTerminal(id);
      void terminalClose(id).catch(() => {});
    }
  }, [detachLocal]);

  // Push a geometry change to the backend PTY only when the grid changed.
  const pushResize = useCallback((cols: number, rows: number) => {
    if (cols === lastColsRef.current && rows === lastRowsRef.current) return;
    lastColsRef.current = cols;
    lastRowsRef.current = rows;
    const id = idRef.current;
    if (id) void terminalResize(id, cols, rows);
  }, []);

  // Proposed grid size, or null when the host has no usable layout.
  // 1. display:none (collapsed edge) reports non-finite computed sizes.
  // 2. An xterm opened while hidden never measured its font; a same-size
  //    resize re-triggers measurement.
  // 3. The measured host must stay padding-free: FitAddon reads
  //    getComputedStyle, which under border-box includes the element's padding.
  const measureGeometry = useCallback((): {
    cols: number;
    rows: number;
  } | null => {
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
        term.resize(term.cols, term.rows);
        dims = fit.proposeDimensions();
      } catch {
        return null;
      }
      if (!dims) return null;
    }
    if (!Number.isFinite(dims.cols) || !Number.isFinite(dims.rows)) return null;
    // FitAddon clamps an empty host to 2×1. A collapsed edge reports that,
    // and pushing it to the pty reflows the screen. Same floor as spawning.
    if (dims.cols < MIN_SPAWN_COLS || dims.rows < MIN_SPAWN_ROWS) return null;
    return dims;
  }, []);

  const resizeXterm = useCallback((cols: number, rows: number) => {
    const term = termRef.current;
    if (!term) return;
    if (cols === term.cols && rows === term.rows) return;
    try {
      term.resize(cols, rows);
    } catch {
      return;
    }
  }, []);

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

  const resizeTo = useCallback(
    (cols: number, rows: number) => {
      const term = termRef.current;
      if (!term) return;
      resizeXterm(cols, rows);
      schedulePtyResize(term.cols, term.rows);
    },
    [resizeXterm, schedulePtyResize],
  );

  // Small buffers resize both axes together. Large buffers apply rows now and
  // debounce columns, because a column change reflows the scrollback.
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

      if (dims.rows !== term.rows) {
        if (dims.cols === term.cols) {
          resizeTo(term.cols, dims.rows);
        } else {
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
    if (!expandedRef.current) return;
    if (connectionRef.current !== "connected") return;
    const dims = measureGeometry();
    if (!dims || dims.cols < MIN_SPAWN_COLS || dims.rows < MIN_SPAWN_ROWS)
      return;
    creatingRef.current = true;
    const myReq = ++reqRef.current;
    try {
      resizeTo(dims.cols, dims.rows);
      const created = await terminalCreate({
        cols: dims.cols,
        rows: dims.rows,
        cwd: cwdRef.current,
      });
      const id = created.id;
      if (myReq !== reqRef.current) {
        discardTerminal(id);
        await terminalClose(id).catch(() => {});
        return;
      }
      idRef.current = id;
      hadSessionRef.current = true;
      if (created.shell) {
        useTerminalTabs.getState().noteShell(tabKeyRef.current, created.shell);
      }
      unbindRef.current?.();
      // ConPTY consumes the shell's `\x1b[?2004h`, so xterm never learns that
      // readline already has bracketed paste on. Arm it once after the first
      // parsed chunk; a later real report from the shell still wins.
      let armPaste = true;
      unbindRef.current = bindTerminal(id, {
        onData: (data) => {
          term.write(data, () => {
            if (!armPaste) return;
            armPaste = false;
            if (!term.modes.bracketedPasteMode) term.write("\x1b[?2004h");
          });
        },
        onExit: () => {
          if (idRef.current !== id) return;
          detachLocal();
          onExitedRef.current();
        },
      });
      const settled = measureGeometry();
      if (
        settled &&
        (settled.cols !== term.cols || settled.rows !== term.rows)
      ) {
        resizeTo(settled.cols, settled.rows);
      }
      if (ptyTimerRef.current != null) {
        clearTimeout(ptyTimerRef.current);
        ptyTimerRef.current = null;
      }
      pushResize(term.cols, term.rows);
      if (activeRef.current) term.focus();
    } catch (e) {
      if (
        myReq === reqRef.current &&
        expandedRef.current &&
        connectionRef.current === "connected"
      ) {
        term.writeln(
          `\r\n[terminal unavailable] ${e instanceof Error ? e.message : String(e)}`,
        );
      }
    } finally {
      if (myReq === reqRef.current) creatingRef.current = false;
    }
  }, [detachLocal, measureGeometry, pushResize, resizeTo]);

  const ensureTerminal = useCallback(() => {
    if (!expandedRef.current) return;
    if (idRef.current || creatingRef.current) return;
    if (connectionRef.current !== "connected") return;
    void createTerminal();
  }, [createTerminal]);

  // One measurement per frame. ResizeObserver callbacks run before rAF, so
  // the frame reads the settled box. Zed does the same in prepaint.
  const scheduleFrame = useCallback(() => {
    if (fitRafRef.current != null) return;
    fitRafRef.current = requestAnimationFrame(() => {
      fitRafRef.current = null;
      applyFit();
      ensureTerminal();
    });
  }, [applyFit, ensureTerminal]);

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
    try {
      const webgl = new WebglAddon();
      term.loadAddon(webgl);
      webgl.onContextLoss(() => {
        webgl.dispose();
      });
    } catch {
      // DOM renderer stays in place when WebGL cannot start.
    }
    termRef.current = term;
    fitRef.current = fit;

    const onData = term.onData((data) => {
      // Full-screen apps (vim, less, …) own the alternate buffer: their keys are
      // not shell commands, so the tracker neither adopts nor accumulates them —
      // otherwise the first command typed after quitting the app would carry
      // whatever was typed inside it.
      if (term.buffer.active.type === "alternate") {
        commandLineRef.current = "";
      } else {
        const tracked = trackCommandLine(commandLineRef.current, data);
        commandLineRef.current = tracked.line;
        for (const command of tracked.commands) {
          useTerminalTabs.getState().noteCommand(tabKeyRef.current, command);
        }
      }
      const id = idRef.current;
      if (!id) return;
      void terminalWrite(id, data).catch((e) => {
        term.writeln(
          `\r\n[write error] ${e instanceof Error ? e.message : String(e)}`,
        );
      });
    });

    const observer = new ResizeObserver(() => {
      scheduleFrame();
    });
    observer.observe(host);

    const onThemeChange = () => {
      term.options.theme = terminalTheme();
    };
    window.addEventListener(THEME_CHANGE_EVENT, onThemeChange);

    scheduleFrame();

    return () => {
      observer.disconnect();
      window.removeEventListener(THEME_CHANGE_EVENT, onThemeChange);
      onData.dispose();
      killTerminal();
      term.dispose();
      termRef.current = null;
      fitRef.current = null;
    };
  }, [killTerminal, scheduleFrame]);

  // Expand refits immediately. Collapse leaves the pty running.
  useEffect(() => {
    if (!expanded) return;
    applyFit(true);
    ensureTerminal();
    if (active) termRef.current?.focus();
  }, [expanded, active, applyFit, ensureTerminal]);

  // Socket drop kills the pty on the server. Reconnect starts a new shell
  // in the same xterm and keeps the scrollback.
  useEffect(() => {
    const prev = prevConnectionRef.current;
    prevConnectionRef.current = connection;
    if (prev === "connected" && connection !== "connected") {
      if (idRef.current || creatingRef.current) detachLocal();
      return;
    }
    if (connection === "connected" && prev !== "connected") {
      if (hadSessionRef.current) {
        termRef.current?.write("\r\n[reconnected]\r\n");
      }
      ensureTerminal();
    }
  }, [connection, detachLocal, ensureTerminal]);

  return (
    <div
      ref={hostRef}
      className="absolute inset-0 overflow-hidden"
      onMouseUp={(e) => {
        if (e.button !== 0) return;
        const term = termRef.current;
        if (!term?.hasSelection()) return;
        const text = term.getSelection();
        if (!text) return;
        void navigator.clipboard.writeText(text).catch(() => {});
      }}
      onContextMenu={(e) => {
        e.preventDefault();
        const term = termRef.current;
        if (!term) return;
        void navigator.clipboard
          .readText()
          .then((text) => {
            if (!text) return;
            term.paste(text);
            term.focus();
          })
          .catch(() => {});
      }}
      onDragOver={(e) => {
        e.preventDefault();
        e.dataTransfer.dropEffect = "copy";
      }}
      onDrop={(e) => {
        e.preventDefault();
        const text = e.dataTransfer.getData("text/plain");
        if (text) termRef.current?.paste(text);
      }}
    />
  );
}
