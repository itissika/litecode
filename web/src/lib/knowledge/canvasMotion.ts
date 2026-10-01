/** Quiet period after the last zoom, drag, or resize before cards sharpen. Pan does not. */
export const CANVAS_IDLE_MS = 160;

/** Long enough for the compositor to observe the animation start and then its end. */
const SHARPEN_MS = 48;

const SHARPEN_KEYFRAMES: Keyframe[] = [
  { transform: "translate3d(0, 0, 0)" },
  { transform: "translate3d(0, 0, 1px)" },
];

export function intersectsViewport(rect: DOMRectReadOnly, view: DOMRectReadOnly): boolean {
  return (
    rect.right >= view.left &&
    rect.left <= view.right &&
    rect.bottom >= view.top &&
    rect.top <= view.bottom
  );
}

/** Pan keeps the same zoom. The first sample only records it. */
export function zoomChanged(previous: number | null, next: number): boolean {
  return previous != null && previous !== next;
}

/** Drag and resize only. Measurement updates omit `dragging` / `resizing`. */
export function isCanvasGesture(change: {
  type: string;
  dragging?: boolean;
  resizing?: boolean;
}): boolean {
  if (change.type === "position") return typeof change.dragging === "boolean";
  if (change.type === "dimensions") return typeof change.resizing === "boolean";
  return false;
}

/**
 * Chrome re-rasters a layer when a transform animation ends, at the scale
 * already on screen. Viewport zoom is not an animation, so a settled gesture
 * plays one on the card and lets it finish. `fill: none` drops the transform.
 */
function sharpenVisibleCards(root: HTMLElement): Animation[] {
  const view = root.getBoundingClientRect();
  const started: Animation[] = [];
  for (const card of root.querySelectorAll<HTMLElement>(".knowledge-flow-card")) {
    if (card.matches(".is-scale-in, .is-scale-out, .is-arrive, .is-leave")) continue;
    if (!intersectsViewport(card.getBoundingClientRect(), view)) continue;
    if (typeof card.animate !== "function") continue;
    started.push(
      card.animate(SHARPEN_KEYFRAMES, {
        duration: SHARPEN_MS,
        easing: "linear",
        fill: "none",
      }),
    );
  }
  return started;
}

export function createCanvasMotion() {
  let timer = 0;
  let sharpening: Animation[] = [];

  function cancelSharpen() {
    for (const animation of sharpening) animation.cancel();
    sharpening = [];
  }

  return {
    /** Restart the idle wait. A new gesture cancels a sharpen already in flight. */
    note(host: HTMLElement) {
      cancelSharpen();
      window.clearTimeout(timer);
      timer = window.setTimeout(() => {
        timer = 0;
        sharpening = sharpenVisibleCards(host);
      }, CANVAS_IDLE_MS);
    },
    dispose() {
      window.clearTimeout(timer);
      timer = 0;
      cancelSharpen();
    },
  };
}
