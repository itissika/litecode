/**
 * A popout is a second document. Dockview copies the opener's stylesheets into
 * it once, when the popout finishes loading; anything the workbench injects
 * afterwards — Monaco's theme sheet when the first editor mounts, a lazy
 * panel's CSS chunk, a theme switch — never reaches the popout, so a panel
 * that mounts after the copy renders unstyled: no token colors, static
 * view-line boxes, widgets without geometry.
 *
 * Mirror the opener head instead: one clone per source element, kept in sync
 * for the life of the popout. Dockview's rule-by-rule copies are dropped when
 * the mirror starts so every sheet exists once, not twice.
 */

const STYLE_SELECTOR = 'style, link[rel~="stylesheet"]';

function styleElements(doc: Document): HTMLElement[] {
  const head = doc.head;
  if (!head) return [];
  return Array.from(head.querySelectorAll<HTMLElement>(STYLE_SELECTOR));
}

/**
 * Keep the target head in step with the source head, now and as it changes.
 * Returns a stop function that disconnects the observer and drops the clones.
 */
export function mirrorDocumentStyles(
  target: Document,
  source: Document = document,
): () => void {
  if (!target.head) {
    target.documentElement.appendChild(target.createElement("head"));
  }
  if (!source.head) return () => {};
  const targetHead = target.head;

  // Dockview's one-shot copy: each rule in its own element. Start clean so the
  // mirror is the only writer and a sheet is not applied twice.
  for (const stale of styleElements(target)) stale.remove();

  const clones = new Map<HTMLElement, HTMLElement>();

  const sync = () => {
    const live = new Set<HTMLElement>();
    for (const el of styleElements(source)) {
      live.add(el);
      const clone = clones.get(el);
      if (!clone) {
        const next = el.cloneNode(true) as HTMLElement;
        if (next.tagName === "LINK") {
          // Resolve against the opener: the popout lives at another URL.
          next.setAttribute("href", (el as HTMLLinkElement).href);
        }
        clones.set(el, next);
        targetHead.appendChild(next);
        continue;
      }
      if (clone.textContent !== el.textContent) clone.textContent = el.textContent;
    }
    for (const [el, clone] of clones) {
      if (live.has(el)) continue;
      clone.remove();
      clones.delete(el);
    }
  };

  const observer = new MutationObserver(sync);
  observer.observe(source.head, {
    childList: true,
    subtree: true,
    characterData: true,
  });
  sync();

  return () => {
    observer.disconnect();
    for (const clone of clones.values()) clone.remove();
    clones.clear();
  };
}
