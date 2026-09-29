import { fetchTree } from "../../api/workspace";
import { isWorkspaceFileRef } from "./markers";

function parentAndName(path: string): { parent: string; name: string } {
  const slash = path.replaceAll("\\", "/");
  const cut = slash.lastIndexOf("/");
  if (cut === -1) return { parent: "", name: slash };
  return { parent: slash.slice(0, cut), name: slash.slice(cut + 1) };
}

function sameName(left: string, right: string): boolean {
  return left.localeCompare(right, undefined, { sensitivity: "accent" }) === 0;
}

/**
 * Whether each workspace-relative path is a file or a directory.
 * One directory listing covers every citation in that directory.
 * A path that fails `isWorkspaceFileRef` is absent.
 */
export async function workspacePathsExist(
  paths: readonly string[],
): Promise<Record<string, boolean>> {
  const result: Record<string, boolean> = {};
  const grouped = new Map<string, string[]>();
  for (const path of paths) {
    if (!isWorkspaceFileRef(path)) {
      result[path] = false;
      continue;
    }
    const { parent } = parentAndName(path);
    const list = grouped.get(parent);
    if (list) list.push(path);
    else grouped.set(parent, [path]);
  }
  await Promise.all(
    [...grouped.entries()].map(async ([parent, group]) => {
      let names: string[] = [];
      try {
        const entries = await fetchTree(parent, 1);
        names = entries.map((entry) => entry.name);
      } catch {
        names = [];
      }
      for (const path of group) {
        const { name } = parentAndName(path);
        result[path] = names.some((entry) => sameName(entry, name));
      }
    }),
  );
  return result;
}
