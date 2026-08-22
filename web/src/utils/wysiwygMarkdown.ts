export type MdEditorView = "wysiwyg" | "source";

/** Crepe/ProseMirror struggles past this; keep Monaco as the editor. */
export const WYSIWYG_MARKDOWN_MAX_CHARS = 200_000;

/** Ordinary Markdown files. `.mdx` stays in the source editor. */
export function isWysiwygMarkdownPath(path: string): boolean {
  const lower = path.toLowerCase();
  return lower.endsWith(".md") && !lower.endsWith(".mdx");
}

export function resolveMdEditorView(
  path: string,
  contentLength: number,
  override: MdEditorView | undefined,
): MdEditorView {
  if (!isWysiwygMarkdownPath(path)) return "source";
  if (contentLength > WYSIWYG_MARKDOWN_MAX_CHARS) return "source";
  return override ?? "wysiwyg";
}
