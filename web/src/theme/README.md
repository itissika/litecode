# Theme Tokens — Design Specification

## Principle: one source of truth

All visual values live **once** in `tokens.css` as CSS custom properties under the
`--_dk-*` namespace (`:root` = dark; `:root[data-dv-theme="light"]` = light
overrides). Everything else derives from it:

- **Components** consume tokens through Tailwind utilities exposed by the
  `@theme` block in `index.css`, or directly via `var(--_dk-*)`.
- **JS consumers** (Monaco, Mermaid) read the live computed values at runtime
  through `theme/tokens.ts` — there is **no hand-maintained copy** to keep in
  sync.

If you change a value in `tokens.css`, it updates globally: utilities, components,
and JS-rendered surfaces (Monaco / Mermaid) all follow automatically.

## Text style framework

Four dimensions, all tokenized:

| Dimension | Tokens (`:root`) | Tailwind exposure |
|-----------|------------------|-------------------|
| Color | `--_dk-text-primary/body/secondary/muted/disabled` | `text-(--_dk-text-*)` |
| Weight | `--_dk-text-weight-regular/medium/semibold/bold` | overrides `--font-weight-*` → `font-normal/medium/semibold/bold` |
| Font | `--_dk-font-ui` (`--font-sans`), `--_dk-font-code` (`--font-mono`) | `font-ui`, `font-code` |
| Size | `--_dk-text-3xs/2xs/xs/sm/md` (9/10/11/12/13px) | `text-dk-3xs/2xs/xs/sm/md` |

Notes:
- Weight utilities (`font-medium`, `font-semibold`, …) are overridden in
  `@theme` to read from the weight tokens. Values match Tailwind defaults, so
  there is no visual change — only centralized control.
- The `text-dk-*` size scale uses an isolated `dk-` namespace so the default
  `text-*` utilities are untouched. **Prefer `text-dk-*` over arbitrary
  `text-[Npx]`** in reading components.
- Raw `font-weight: 600` in CSS must use `var(--_dk-text-weight-semibold)`.

### Readability conventions (conversation bubbles)

The message bubble area is the primary reading surface. These conventions keep
it legible; all values come from the tokens above.

- **Reading size floor**: prose and tool content render at ≥ 12px
  (`text-dk-sm` / `text-sm`). The smallest `text-dk-xs` (11px) is reserved for
  *labels and one-line summaries* (tool name, file path, "Copy", counts) — never
  for body content. Reasoning text uses `text-sm` (14px), matching the assistant
  body, not `text-xs`.
- **Contrast floor**: chat prose uses `text-body` (a touch below `text-primary`,
  brighter than `text-secondary`) for stronger contrast; other informational text
  uses `text-secondary` or `text-muted`. `text-disabled` (#646463, ~2.5:1) is
  reserved for *decorative* text only (e.g. a session id) — never for counts,
  language labels, or content.
- **Prose measure**: markdown prose (`p`, `ul`, `ol`, `blockquote`, `h1-3`) is
  capped at `--_dk-prose-measure` (72ch) via `content.css` so long lines don't
  stretch across ultra-wide panels. Code blocks, tables, and mermaid diagrams are
  direct children and stay full width.
- **Code blocks**: 13px (`--_dk-text-md`), `line-height: 1.6`.
- To change the reading width app-wide, edit `--_dk-prose-measure` in
  `tokens.css` — no component edits needed.

## Adding a token

1. Define it in `tokens.css` under `:root` (and a light override in
   `:root[data-dv-theme="light"]` if it is theme-dependent).
2. Expose it in `index.css` `@theme` using the correct namespace
   (`--color-*`, `--font-weight-*`, `--font-*`, `--text-*`).
3. Consume via the generated utility or `var(--_dk-*)`. Never hardcode the
   literal elsewhere.

## JS consumers (no dual source)

`theme/tokens.ts` exposes `readDkTokens()` (current theme) and
`readDkTokensForTheme("dark" | "light")` (specific theme, used to define both
Monaco themes up front). Both read `getComputedStyle` — no second copy.

- **Monaco** (`theme/monaco.ts`): both editor themes are derived from the live
  tokens via `readDkTokensForTheme`.
- **Mermaid** (`theme/mermaid.ts` + `lib/mermaid.ts` + `components/MermaidBlock.tsx`):
  - Renders on mermaid's `base` theme and overrides only the three visual
    dimensions — fills, strokes/lines, text — via
    `buildMermaidThemeVariables(tokens, darkMode)`.
  - **Every color comes from a project token** (or a `color-mix` of one) through
    a small set of roles (`overlay`/`sidepanel` fills, `line-visible`/`line`
    borders, `text-primary`/`text-muted` text, `accent` highlights). No
    hand-picked hex palettes, so diagrams stay in sync with the app theme.
  - The categorical scale (pie slices, mindmap branches) is tinted from
    `accent` so it stays inside the project palette.
  - `MermaidBlock` re-initializes Mermaid with the current theme on
    `THEME_CHANGE_EVENT` and re-renders, so diagrams follow light/dark.

## Adapter pattern for external components

External libraries that cannot use Tailwind utilities are wired through an
**adapter layer** that maps their variables onto `--_dk-*`:

- **dockview** (`theme/dockview/adapter.css`): maps `--dv-*` → `var(--_dk-*)`.
  The tab-group chip palette is sourced from the `--_dk-cat-*` categorical
  tokens (defined once in `tokens.css`, themed per surface).
- **mermaid**: see above — `themeVariables` are built from tokens, not literals.

### Shiki (code highlighting) — intentionally OUT of the token system

Shiki uses its built-in `dark-plus` / `min-light` dual themes, toggled via the
`--shiki-dark` / `--shiki-dark-bg` CSS variables in `index.css`. This is a
**deliberate decision**: syntax-highlight color schemes are a domain of their
own (language-grammar-driven palettes), not UI design tokens. Pulling them into
`--_dk-*` would couple editor syntax colors to the app theme and add no
consistency benefit. Shiki therefore does **not** read `--_dk-*`. If this ever
changes, do it behind a dedicated `theme/shiki.ts` adapter, not by editing
token values.

## Orphan / dead references (resolved)

- `--_dk-border-subtle` was referenced but never defined; usages now point to
  `--_dk-line-visible`.
- `DK_TOKENS_LIGHT` / `DK_TOKENS_DARK` static copies in `tokens.ts` were removed
  in favor of runtime derivation.
