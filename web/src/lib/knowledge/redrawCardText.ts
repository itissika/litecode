export function shouldSharpenZoom(start: number | null, end: number): boolean {
  return start != null && start !== end;
}

export function intersectsViewport(rect: DOMRectReadOnly, view: DOMRectReadOnly): boolean {
  return (
    rect.right >= view.left &&
    rect.left <= view.right &&
    rect.bottom >= view.top &&
    rect.top <= view.bottom
  );
}

export function nextTextRaster(current: string | null): "a" | "b" {
  return current === "a" ? "b" : "a";
}

export function redrawVisibleCardText(root: HTMLElement): void {
  const view = root.getBoundingClientRect();
  root.querySelectorAll<HTMLElement>(".knowledge-flow-card").forEach((card) => {
    if (!intersectsViewport(card.getBoundingClientRect(), view)) return;
    card.setAttribute("data-text-raster", nextTextRaster(card.getAttribute("data-text-raster")));
  });
}
