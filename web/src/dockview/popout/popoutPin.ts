import { dockIdFromPopoutUrl } from "../config/popoutUrl";

/** Pin control sitting in a popout tab-bar void. */
export const POPOUT_PIN_CLASS = "lc-popout-pin";

const HORIZONTAL_VOID =
  ".dv-tabs-and-actions-container:not(.dv-groupview-header-vertical) .dv-void-container";

const PIN_MARKUP =
  '<svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true"><path d="M4.15 1.4h3.7v2.05l1.55 1.55v1.15H2.6V5l1.55-1.55V1.4zM5.4 6.4h1.2v4.2H5.4z" fill="currentColor"/></svg>';

function pinFromEvent(event: Event): Element | null {
  const target = event.target;
  if (!target || typeof target !== "object" || !("closest" in target)) return null;
  return (target as Element).closest(`.${POPOUT_PIN_CLASS}`);
}

function paintButton(button: Element, pinned: boolean): void {
  const label = pinned ? "Unpin" : "Pin on top";
  button.setAttribute("aria-pressed", pinned ? "true" : "false");
  button.setAttribute("aria-label", label);
  button.setAttribute("title", label);
}

function createButton(doc: Document): HTMLButtonElement {
  const button = doc.createElement("button");
  button.type = "button";
  button.className = POPOUT_PIN_CLASS;
  button.draggable = false;
  button.innerHTML = PIN_MARKUP;
  return button;
}

/**
 * Puts a pin button on every horizontal tab-bar void in this popout window.
 * The button toggles always-on-top for that window only. Voids added later
 * (a split inside the same window) pick up the current state.
 */
export function bindPopoutPin(
  win: Window,
  setAlwaysOnTop: (dockId: string, onTop: boolean) => Promise<boolean>,
): () => void {
  const dockId = dockIdFromPopoutUrl(win.location.href);
  if (!dockId) return () => {};

  const doc = win.document;
  let pinned = false;
  let pending = false;

  const paint = () => {
    for (const voidEl of doc.querySelectorAll(HORIZONTAL_VOID)) {
      let button = voidEl.querySelector(`:scope > .${POPOUT_PIN_CLASS}`);
      if (!button) {
        button = createButton(doc);
        voidEl.appendChild(button);
      }
      paintButton(button, pinned);
    }
  };

  const swallowPress = (event: Event) => {
    if (!pinFromEvent(event)) return;
    event.preventDefault();
    event.stopPropagation();
  };

  const onClick = (event: Event) => {
    if (!pinFromEvent(event)) return;
    event.preventDefault();
    event.stopPropagation();
    if (pending) return;
    pending = true;
    const next = !pinned;
    void setAlwaysOnTop(dockId, next)
      .then((applied) => {
        pinned = applied === true;
        paint();
      })
      .catch(() => {})
      .finally(() => {
        pending = false;
      });
  };

  doc.addEventListener("pointerdown", swallowPress, true);
  doc.addEventListener("mousedown", swallowPress, true);
  doc.addEventListener("click", onClick, true);
  paint();

  const observer = new MutationObserver(() => paint());
  observer.observe(doc.body ?? doc.documentElement, { childList: true, subtree: true });

  return () => {
    observer.disconnect();
    doc.removeEventListener("pointerdown", swallowPress, true);
    doc.removeEventListener("mousedown", swallowPress, true);
    doc.removeEventListener("click", onClick, true);
    for (const button of doc.querySelectorAll(`.${POPOUT_PIN_CLASS}`)) button.remove();
  };
}
