/** Overlay cards (Todo + chat input) sitting on the message list.
 *  One utility per line — edit here, both cards pick it up. */

/** Current drop shadow for overlay cards. Reads --_dk-composer-card-shadow,
 *  which defaults to the base (--_dk-composer-shadow) and is overridden to the
 *  focused variant (--_dk-composer-focus-shadow) by the dock container while
 *  its panel is active — so the shadow follows the panel focus state. */
export const composerShadow = "shadow-(--_dk-composer-card-shadow)";

/** Floating action-button glass: 66% editor fill + backdrop blur. Draft text
 *  scrolling under the buttons gets softly obscured (not hidden) — same glass
 *  language as the composer card itself, just stronger so buttons stay legible. */
export const actionButtonGlass =
  "bg-[color-mix(in_srgb,var(--_dk-editor)_66%,transparent)] backdrop-blur-[12px]";

export const composerCardClass = [
  "rounded-md",
  "border",
  "border-(--_dk-line)",
  // glass fill: 88% editor, rest shows the list through
  "[background:color-mix(in_srgb,var(--_dk-editor)_82%,transparent)]",
  "backdrop-blur-[12px]",
  // quick shadow transition — the dock container flips the card-shadow var on
  // panel focus change, and this eases the box-shadow instead of snapping it.
  "transition-shadow duration-150",
  composerShadow,
].join(" ");
