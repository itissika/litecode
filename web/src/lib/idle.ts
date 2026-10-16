/** Resolve once the browser is idle (requestIdleCallback; setTimeout 0 where
 *  it is missing, e.g. jsdom). Used to push non-critical startup work after
 *  the first frame. */
export function whenIdle(): Promise<void> {
  return new Promise((resolve) => {
    if (typeof window.requestIdleCallback === "function") {
      window.requestIdleCallback(() => resolve());
    } else {
      window.setTimeout(resolve, 0);
    }
  });
}
