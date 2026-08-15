/** Overlay cards (Todo + chat input) sitting on the message list.
 *  One utility per line — edit here, both cards pick it up. */
export const composerCardClass = [
  "rounded-md",
  "border",
  "border-(--_dk-line)",
  // glass fill: 88% editor, rest shows the list through
  "[background:color-mix(in_srgb,var(--_dk-editor)_88%,transparent)]",
  "backdrop-blur-[18px]",
  // box-shadow: x y blur spread color  — x/y stay 0 (no offset)
  "shadow-[0_2px_24px_6px_color-mix(in_srgb,var(--_dk-sidepanel)_80%,transparent)]",
].join(" ");
