import {
  createFile,
  deletePath,
  fetchTree,
  mkdir,
  readFile,
  renamePath,
  writeFile,
  type TreeEntry,
} from "../../api/workspace";
import { isKnowledgeKey, normalizeKey } from "./markers";
import {
  KNOWLEDGE_ROOT,
  knowledgeFromFiles,
  renderKnowledgeMarkdown,
  upgradeKnowledgeMarkdown,
  type KnowledgeSourceFile,
} from "./document";
import { knowledgeSeedFiles } from "./seed";
import type { KnowledgeFolder, KnowledgeNode } from "./types";

/** Gitignored copy. Created when a workspace has no knowledge directory yet. */
export const KNOWLEDGE_PRIVATE_ROOT = KNOWLEDGE_ROOT;

/** Workspace-root copy. Git can track it. */
export const KNOWLEDGE_PUBLIC_ROOT = "knowledge";

export type KnowledgeVisibility = "private" | "public";

export async function writeKnowledgeNode(
  node: KnowledgeNode,
  root: string = KNOWLEDGE_PRIVATE_ROOT,
): Promise<void> {
  await writeFile(
    knowledgeDiskPath(root, node.path),
    renderKnowledgeMarkdown({
      key: node.key,
      status: node.status,
      invalidStatus: node.invalidStatus,
      summary: node.summary,
      body: node.value,
      x: node.x,
      y: node.y,
      w: node.w,
      h: node.h,
      extras: node.extras,
    }),
  );
}

function normalizePath(path: string): string {
  return path.replaceAll("\\", "/");
}

function relativeToRoot(path: string, root: string): string | null {
  const norm = normalizePath(path);
  const prefix = `${root}/`;
  if (norm === root) return "";
  if (!norm.startsWith(prefix)) return null;
  return norm.slice(prefix.length);
}

async function listChildren(dir: string): Promise<TreeEntry[]> {
  const entries = await fetchTree(dir, 1);
  return entries.map((entry) => ({
    ...entry,
    path: normalizePath(entry.path),
  }));
}

/** Walk one knowledge directory. A missing root throws so the caller can seed it. */
export async function readKnowledgeTree(root: string): Promise<{
  files: KnowledgeSourceFile[];
  directories: string[];
}> {
  const files: KnowledgeSourceFile[] = [];
  const directories: string[] = [];

  async function walk(dir: string): Promise<void> {
    const entries = await listChildren(dir);
    for (const entry of entries) {
      const rel = relativeToRoot(entry.path, root);
      if (rel == null || rel.length === 0) continue;
      if (entry.kind === "dir") {
        directories.push(rel);
        await walk(entry.path);
        continue;
      }
      if (!entry.name.toLowerCase().endsWith(".md")) continue;
      let markdown = await readFile(entry.path);
      const upgraded = upgradeKnowledgeMarkdown(markdown);
      if (upgraded && upgraded !== markdown) {
        markdown = upgraded;
        await writeFile(entry.path, upgraded);
      }
      files.push({ path: rel, markdown });
    }
  }

  await walk(root);
  return { files, directories };
}

/** Create the onboarding tree under `root`. */
export async function seedKnowledgeWorkspace(
  root: string = KNOWLEDGE_PRIVATE_ROOT,
): Promise<void> {
  await mkdir(root);
  for (const file of knowledgeSeedFiles) {
    await createFile(knowledgeDiskPath(root, file.path), file.markdown);
  }
}

export async function knowledgeRootExists(root: string): Promise<boolean> {
  try {
    const entries = await fetchTree(root, 1);
    return entries.every((entry) => entry.path !== root || entry.kind === "dir");
  } catch {
    return false;
  }
}

/**
 * Public `knowledge/` wins when it exists. Otherwise use `.litecode/knowledge`.
 * When neither exists, create the private tree and the onboarding nodes.
 */
export async function locateKnowledgeRoot(): Promise<{
  root: string;
  visibility: KnowledgeVisibility;
}> {
  if (await knowledgeRootExists(KNOWLEDGE_PUBLIC_ROOT)) {
    return { root: KNOWLEDGE_PUBLIC_ROOT, visibility: "public" };
  }
  if (await knowledgeRootExists(KNOWLEDGE_PRIVATE_ROOT)) {
    return { root: KNOWLEDGE_PRIVATE_ROOT, visibility: "private" };
  }
  await seedKnowledgeWorkspace(KNOWLEDGE_PRIVATE_ROOT);
  return { root: KNOWLEDGE_PRIVATE_ROOT, visibility: "private" };
}

/** Read the on-disk corpus, seeding the private tree when both locations are missing. */
export async function loadKnowledgeFromWorkspace(): Promise<{
  nodes: KnowledgeNode[];
  folders: KnowledgeFolder[];
  root: string;
  visibility: KnowledgeVisibility;
}> {
  const located = await locateKnowledgeRoot();
  const loaded = await readKnowledgeSnapshot(located.root);
  return { ...loaded, ...located };
}

/** Re-read an existing corpus. Does not create the sample tree. */
export async function readKnowledgeSnapshot(
  root: string,
): Promise<{
  nodes: KnowledgeNode[];
  folders: KnowledgeFolder[];
}> {
  const tree = await readKnowledgeTree(root);
  return knowledgeFromFiles(tree.files, tree.directories);
}

function joinRel(parentId: string | null, name: string): string {
  return parentId ? `${parentId}/${name}` : name;
}

/** Folder name under `parentId`, or null when the name cannot be a directory. */
export function knowledgeFolderRel(
  parentId: string | null,
  name: string,
): string | null {
  const trimmed = name.trim();
  if (!trimmed || trimmed === "." || trimmed === "..") return null;
  if (/[\\/\0]/.test(trimmed)) return null;
  return joinRel(parentId, trimmed);
}

/** `key.md` under `folderId`, or null when the key is not a declaration. */
export function knowledgeNodeRel(
  folderId: string | null,
  key: string,
): string | null {
  const normalized = normalizeKey(key);
  if (!isKnowledgeKey(normalized)) return null;
  return joinRel(folderId, `${normalized}.md`);
}

export function knowledgeDiskPath(root: string, rel: string): string {
  return rel ? `${root}/${rel}` : root;
}

export function knowledgeMoveDestination(
  fromRel: string,
  parentId: string | null,
): string {
  const slash = fromRel.lastIndexOf("/");
  const name = slash === -1 ? fromRel : fromRel.slice(slash + 1);
  return parentId ? `${parentId}/${name}` : name;
}

export async function createKnowledgeFolder(
  root: string,
  rel: string,
): Promise<void> {
  await mkdir(knowledgeDiskPath(root, rel));
}

export async function createKnowledgeNode(
  root: string,
  rel: string,
  key: string,
): Promise<void> {
  await createFile(
    knowledgeDiskPath(root, rel),
    renderKnowledgeMarkdown({
      key,
      status: "enabled",
      summary: "",
      body: "",
    }),
  );
}

export async function deleteKnowledgeEntry(
  root: string,
  rel: string,
  recursive: boolean,
): Promise<void> {
  await deletePath(knowledgeDiskPath(root, rel), recursive);
}

export async function moveKnowledgeEntry(
  root: string,
  fromRel: string,
  toRel: string,
): Promise<void> {
  await renamePath(
    knowledgeDiskPath(root, fromRel),
    knowledgeDiskPath(root, toRel),
    false,
  );
}

export async function moveKnowledgeRoot(from: string, to: string): Promise<void> {
  await renamePath(from, to, false);
}
