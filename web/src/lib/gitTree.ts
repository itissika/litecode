import type { GitFile } from "../api/workspace";

export type GitTreeDir = {
  kind: "dir";
  name: string;
  path: string;
  children: GitTreeNode[];
};

export type GitTreeFile = {
  kind: "file";
  name: string;
  file: GitFile;
};

export type GitTreeNode = GitTreeDir | GitTreeFile;

function insert(nodes: GitTreeNode[], parts: string[], file: GitFile, prefix: string) {
  if (parts.length === 1) {
    nodes.push({ kind: "file", name: parts[0] ?? file.path, file });
    return;
  }
  const name = parts[0] ?? "";
  const dirPath = prefix ? `${prefix}/${name}` : name;
  let dir = nodes.find((n): n is GitTreeDir => n.kind === "dir" && n.name === name);
  if (!dir) {
    dir = { kind: "dir", name, path: dirPath, children: [] };
    nodes.push(dir);
  }
  insert(dir.children, parts.slice(1), file, dirPath);
}

function sortNodes(nodes: GitTreeNode[]): GitTreeNode[] {
  nodes.sort((a, b) => {
    if (a.kind !== b.kind) return a.kind === "dir" ? -1 : 1;
    return a.name.localeCompare(b.name);
  });
  for (const n of nodes) {
    if (n.kind === "dir") sortNodes(n.children);
  }
  return nodes;
}

/** Nested folder tree from git paths (VS Code "View as Tree"). */
export function buildGitTree(files: GitFile[]): GitTreeNode[] {
  const roots: GitTreeNode[] = [];
  for (const file of files) {
    const parts = file.path.replace(/\\/g, "/").split("/").filter(Boolean);
    if (parts.length === 0) continue;
    insert(roots, parts, file, "");
  }
  return sortNodes(roots);
}

export function descendantFiles(node: GitTreeNode): GitFile[] {
  if (node.kind === "file") return [node.file];
  return node.children.flatMap(descendantFiles);
}

/** File row ids in on-screen order, skipping collapsed directories. */
export function visibleFileIds(
  nodes: GitTreeNode[],
  section: "staged" | "changes",
  collapsed: Set<string>,
  rowId: (section: "staged" | "changes", path: string) => string,
): string[] {
  const out: string[] = [];
  const walk = (list: GitTreeNode[]) => {
    for (const node of list) {
      if (node.kind === "file") {
        out.push(rowId(section, node.file.path));
        continue;
      }
      if (collapsed.has(node.path)) continue;
      walk(node.children);
    }
  };
  walk(nodes);
  return out;
}
