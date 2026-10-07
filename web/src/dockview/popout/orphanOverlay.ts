/**
 * `defaultRenderer="always"` keeps panel content in a `.dv-render-overlay`.
 * Moving a panel into a popout window reparents that content into the popout
 * document and leaves the old overlay behind. The panel is still visible, so
 * the empty shell stays `pointer-events: auto` and covers the main grid.
 * Drop shells that no longer hold a content element.
 */
export function releaseOrphanRenderOverlays(
  docs: Iterable<Document | null | undefined>,
): void {
  for (const doc of docs) {
    if (!doc) continue;
    for (const el of doc.querySelectorAll(".dv-render-overlay")) {
      if (el.childElementCount === 0) el.remove();
    }
  }
}
