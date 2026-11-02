/** The attribute a citation chip carries its shortcode in. */
const CITATION_ATTR = "data-citation";
const CHIP_SELECTOR = `.knowledge-token[${CITATION_ATTR}]`;

/** The editor round-trips mentions as nodes, so its copy is never touched. */
const EDITABLE_SELECTOR =
  "[contenteditable='true'], .ProseMirror, input, textarea";

/** A copy event reduced to what this handler needs. */
interface CitationCopyEvent {
  clipboardData: DataTransfer | null;
  target: EventTarget | null;
  preventDefault(): void;
}

function elementOf(node: Node | null): Element | null {
  if (!node) return null;
  return node instanceof Element ? node : node.parentElement;
}

/**
 * Copy with citations whole. A chip is one atomic reference and is not
 * selectable, so a selection that crosses one would otherwise drop it; instead
 * the selection is copied with every chip replaced by its shortcode, which
 * pastes back into a mention field as the same reference. The markup is copied
 * too, so a rich target keeps the formatting. A selection without chips is left
 * to the browser untouched.
 *
 * Runs on the document, not on the transcript element: the browser aims the
 * copy event at whoever holds focus or where the selection starts, so a
 * selection made after clicking elsewhere, a select-all, and a context-menu
 * copy would all miss a container-scoped handler.
 */
export function onCitationCopy(event: CitationCopyEvent): void {
  const data = event.clipboardData;
  // A popout window has its own document and selection; the realm does not.
  const doc =
    (event.target instanceof Node ? event.target.ownerDocument : null) ??
    document;
  const selection = doc.defaultView?.getSelection?.() ?? null;
  if (!data || !selection || selection.isCollapsed || selection.rangeCount === 0) {
    return;
  }
  // Scoped by chips, not by container: a select-all starts at the document, and
  // a selection may run from one bubble into the next.
  const anchor = elementOf(selection.anchorNode);
  if (anchor?.closest(EDITABLE_SELECTOR)) return;
  const holder = doc.createElement("div");
  holder.appendChild(selection.getRangeAt(0).cloneContents());
  const chips = Array.from(holder.querySelectorAll<HTMLElement>(CHIP_SELECTOR));
  if (chips.length === 0) return;
  for (const chip of chips) {
    chip.replaceWith(
      doc.createTextNode(chip.getAttribute(CITATION_ATTR) ?? ""),
    );
  }
  // Off-screen but still laid out: `innerText` is what keeps the paragraph
  // breaks a person expects from a copied reply.
  holder.style.cssText =
    "position:fixed;top:0;left:-10000px;white-space:pre-wrap";
  doc.body.appendChild(holder);
  const text = holder.innerText ?? holder.textContent ?? "";
  const html = holder.innerHTML;
  holder.remove();
  data.setData("text/plain", text);
  data.setData("text/html", html);
  event.preventDefault();
}

const installed = new WeakSet<Document>();

/**
 * One listener per document — the main window and every popout. Capture phase,
 * so a handler that stops the copy event deeper down cannot hide the chips.
 */
export function installCitationCopy(doc: Document = document): () => void {
  if (installed.has(doc)) return () => {};
  installed.add(doc);
  doc.addEventListener("copy", onCitationCopy, true);
  return () => {
    installed.delete(doc);
    doc.removeEventListener("copy", onCitationCopy, true);
  };
}
