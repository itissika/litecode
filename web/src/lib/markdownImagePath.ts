import { fileKindFromPath } from "./fileKind";
import { isWorkspaceFileRef } from "./knowledge/markers";

/** Where a Markdown image in a file preview should load from. */
export type MarkdownImageTarget =
  | { kind: "remote"; url: string }
  | { kind: "workspace"; path: string };

const HTTP_URL = /^https?:\/\//i;
const HAS_SCHEME = /^[a-zA-Z][a-zA-Z0-9+.-]*:/;

/**
 * Resolve `![](src)` against the markdown file that contains it.
 * `https://` stays a remote image. A relative path is joined to that file's
 * directory; `..` that leaves the workspace, any other scheme, and a non-image
 * extension do not load.
 */
export function resolveMarkdownImage(
  markdownFile: string,
  src: string | undefined,
): MarkdownImageTarget | null {
  const trimmed = src?.trim() ?? "";
  if (!trimmed || trimmed.startsWith("//")) return null;
  if (HTTP_URL.test(trimmed)) return { kind: "remote", url: trimmed };
  if (HAS_SCHEME.test(trimmed)) return null;

  const hashAt = trimmed.indexOf("#");
  const beforeHash = hashAt >= 0 ? trimmed.slice(0, hashAt) : trimmed;
  const queryAt = beforeHash.indexOf("?");
  const rawPath = queryAt >= 0 ? beforeHash.slice(0, queryAt) : beforeHash;
  let decoded: string;
  try {
    decoded = decodeURIComponent(rawPath);
  } catch {
    return null;
  }

  const joined = joinRelative(parentDir(markdownFile), decoded.replaceAll("\\", "/"));
  if (!joined || !isWorkspaceFileRef(joined)) return null;
  if (fileKindFromPath(joined) !== "image") return null;
  return { kind: "workspace", path: joined };
}

function parentDir(file: string): string {
  const slash = file.replaceAll("\\", "/");
  const cut = slash.lastIndexOf("/");
  return cut < 0 ? "" : slash.slice(0, cut);
}

function joinRelative(dir: string, rel: string): string | null {
  if (rel.startsWith("/") || /^[a-zA-Z]:/.test(rel)) return null;
  const parts = dir ? dir.split("/").filter((part) => part.length > 0) : [];
  for (const segment of rel.split("/")) {
    if (segment === "" || segment === ".") continue;
    if (segment === "..") {
      if (parts.length === 0) return null;
      parts.pop();
      continue;
    }
    parts.push(segment);
  }
  if (parts.length === 0) return null;
  return parts.join("/");
}
