import { dockIdFromPopoutUrl } from "../config/popoutUrl";
import { closePanel } from "../workbench/commands";
import { getWindows } from "../workbench/windows";

/** The window-controls row that sits at the right end of a popout tab bar. */
export const POPOUT_ACTIONS_CLASS = "lc-popout-actions";
/** One button in that row; `data-lc-action` names what it does. */
export const POPOUT_ACTION_CLASS = "lc-popout-action";

const HORIZONTAL_VOID =
  ".dv-tabs-and-actions-container:not(.dv-groupview-header-vertical) .dv-void-container";

const svg = (body: string) =>
  `<svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true">${body}</svg>`;

const ICONS = {
  pin: svg(
    '<path d="M4.15 1.4h3.7v2.05l1.55 1.55v1.15H2.6V5l1.55-1.55V1.4zM5.4 6.4h1.2v4.2H5.4z" fill="currentColor"/>',
  ),
  minimize: svg('<rect x="2.2" y="5.4" width="7.6" height="1.2" rx="0.6" fill="currentColor"/>'),
  maximize: svg(
    '<rect x="2.7" y="2.7" width="6.6" height="6.6" rx="1.1" fill="none" stroke="currentColor" stroke-width="1.2"/>',
  ),
  restore: svg(
    '<path d="M4.6 3.3h2.6a2.1 2.1 0 0 1 2.1 2.1v2.6" fill="none" stroke="currentColor" stroke-width="1.2"/><rect x="2.4" y="4.7" width="4.9" height="4.9" rx="1" fill="none" stroke="currentColor" stroke-width="1.2"/>',
  ),
  closeAll: svg(
    '<path d="M3.2 3.2l5.6 5.6M8.8 3.2L3.2 8.8" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/>',
  ),
} as const;

/** OS window control for one popout; the Electron host answers it. */
export interface PopoutWindowControl {
  minimize(dockId: string): Promise<void>;
  /** Returns whether the window is maximized after the toggle. */
  toggleMaximize(dockId: string): Promise<boolean>;
  isMaximized(dockId: string): Promise<boolean>;
}

export interface PopoutActionsHost {
  setAlwaysOnTop(dockId: string, onTop: boolean): Promise<boolean>;
  /** Absent in a plain browser: the OS window is not ours to control. */
  windowControl?: PopoutWindowControl;
}

function elementFromEvent(event: Event, selector: string): Element | null {
  const target = event.target;
  if (!target || typeof target !== "object" || !("closest" in target)) return null;
  return (target as Element).closest(selector);
}

function actionName(button: Element): string | null {
  return button.getAttribute("data-lc-action");
}

function createButton(doc: Document, action: string, markup: string, label: string): HTMLButtonElement {
  const button = doc.createElement("button");
  button.type = "button";
  button.className = POPOUT_ACTION_CLASS;
  button.dataset.lcAction = action;
  button.draggable = false;
  button.innerHTML = markup;
  button.setAttribute("aria-label", label);
  button.setAttribute("title", label);
  return button;
}

/**
 * Window controls on every horizontal tab-bar void of this popout: pin,
 * minimize, maximize/restore, and close-all. The void keeps the drag strip;
 * the row is `no-drag` and swallows the press that would start a tab drag.
 *
 * Minimize and maximize are OS window operations: dockview holds the window
 * handle but exposes no such API, and a popout has no preload, so the host
 * call goes back through the opener with this window's dock id.
 */
export function bindPopoutActions(win: Window, host: PopoutActionsHost): () => void {
  const dockId = dockIdFromPopoutUrl(win.location.href);
  if (!dockId) return () => {};

  const doc = win.document;
  const control = host.windowControl;
  let pinned = false;
  let maximized = false;
  let frame = 0;
  const pending = new Set<string>();

  const paintRow = (row: Element) => {
    const pin = row.querySelector(`[data-lc-action="pin"]`);
    if (pin) {
      const label = pinned ? "Unpin" : "Pin on top";
      pin.setAttribute("aria-pressed", pinned ? "true" : "false");
      pin.setAttribute("aria-label", label);
      pin.setAttribute("title", label);
    }
    const maximize = row.querySelector(`[data-lc-action="maximize"]`);
    if (maximize) {
      const label = maximized ? "Restore" : "Maximize";
      maximize.setAttribute("aria-label", label);
      maximize.setAttribute("title", label);
      // Swap the glyph only when the state flips: writing innerHTML on every
      // paint would feed the observer that scheduled the paint.
      const state = maximized ? "1" : "0";
      if (row.getAttribute("data-maximized") !== state) {
        row.setAttribute("data-maximized", state);
        maximize.innerHTML = maximized ? ICONS.restore : ICONS.maximize;
      }
    }
  };

  const createRow = (): Element => {
    const row = doc.createElement("div");
    row.className = POPOUT_ACTIONS_CLASS;
    row.appendChild(createButton(doc, "pin", ICONS.pin, "Pin on top"));
    if (control) {
      row.appendChild(createButton(doc, "minimize", ICONS.minimize, "Minimize"));
      row.appendChild(createButton(doc, "maximize", ICONS.maximize, "Maximize"));
    }
    row.appendChild(createButton(doc, "close-all", ICONS.closeAll, "Close all tabs"));
    row.setAttribute("data-maximized", "0");
    return row;
  };

  const paint = () => {
    for (const voidEl of doc.querySelectorAll(HORIZONTAL_VOID)) {
      let row = voidEl.querySelector(`:scope > .${POPOUT_ACTIONS_CLASS}`);
      if (!row) {
        row = createRow();
        voidEl.appendChild(row);
      }
      paintRow(row);
    }
  };

  const run = (name: string, task: () => Promise<unknown>) => {
    if (pending.has(name)) return;
    pending.add(name);
    void task()
      .catch(() => {})
      .finally(() => pending.delete(name));
  };

  const syncMaximized = () => {
    frame = 0;
    if (!control) return;
    void control
      .isMaximized(dockId)
      .then((value) => {
        if (value === maximized) return;
        maximized = value;
        paint();
      })
      .catch(() => {});
  };

  const scheduleMaximized = () => {
    if (!control || frame !== 0) return;
    frame = win.requestAnimationFrame(syncMaximized);
  };

  const toggleMaximize = () => {
    if (!control || pending.has("maximize")) return;
    pending.add("maximize");
    void control
      .toggleMaximize(dockId)
      .then((value) => {
        maximized = value === true;
        paint();
      })
      .catch(() => {})
      .finally(() => pending.delete("maximize"));
  };

  const closeAll = () => {
    const group = getWindows().find((entry) => entry.dockId === dockId)?.group;
    for (const panel of group?.panels ?? []) closePanel(panel.id);
  };

  const togglePin = () => {
    if (pending.has("pin")) return;
    pending.add("pin");
    void host
      .setAlwaysOnTop(dockId, !pinned)
      .then((applied) => {
        pinned = applied === true;
        paint();
      })
      .catch(() => {})
      .finally(() => pending.delete("pin"));
  };

  const swallowPress = (event: Event) => {
    if (!elementFromEvent(event, `.${POPOUT_ACTIONS_CLASS}`)) return;
    event.preventDefault();
    event.stopPropagation();
  };

  const onClick = (event: Event) => {
    const button = elementFromEvent(event, `.${POPOUT_ACTION_CLASS}`);
    if (!button) return;
    event.preventDefault();
    event.stopPropagation();
    switch (actionName(button)) {
      case "pin":
        togglePin();
        break;
      case "minimize":
        if (control) run("minimize", () => control.minimize(dockId));
        break;
      case "maximize":
        toggleMaximize();
        break;
      case "close-all":
        closeAll();
        break;
    }
  };

  doc.addEventListener("pointerdown", swallowPress, true);
  doc.addEventListener("mousedown", swallowPress, true);
  doc.addEventListener("click", onClick, true);
  win.addEventListener("resize", scheduleMaximized);
  paint();
  scheduleMaximized();

  const observer = new MutationObserver(() => paint());
  observer.observe(doc.body ?? doc.documentElement, { childList: true, subtree: true });

  return () => {
    observer.disconnect();
    if (frame !== 0) win.cancelAnimationFrame(frame);
    win.removeEventListener("resize", scheduleMaximized);
    doc.removeEventListener("pointerdown", swallowPress, true);
    doc.removeEventListener("mousedown", swallowPress, true);
    doc.removeEventListener("click", onClick, true);
    for (const row of doc.querySelectorAll(`.${POPOUT_ACTIONS_CLASS}`)) row.remove();
  };
}
