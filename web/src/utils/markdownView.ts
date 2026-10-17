export type MdEditorView = "preview" | "source";

/** Past this size the preview stays closed and Monaco remains the editor. */
export const PREVIEW_MARKDOWN_MAX_CHARS = 200_000;

/** Ordinary Markdown files. `.mdx` stays in the source editor. */
export function isPreviewMarkdownPath(path: string): boolean {
  const lower = path.toLowerCase();
  return lower.endsWith(".md") && !lower.endsWith(".mdx");
}

export function resolveMdEditorView(
  path: string,
  contentLength: number,
  override: MdEditorView | undefined,
): MdEditorView {
  if (!isPreviewMarkdownPath(path)) return "source";
  if (contentLength > PREVIEW_MARKDOWN_MAX_CHARS) return "source";
  return override ?? "preview";
}
