/**
 * Explicit writes to the knowledge directory. Reading the corpus, existence,
 * and every verdict come from the backend snapshot (`snapshot.ts`); nothing
 * here reads the tree, and nothing rewrites a file as a side effect of reading.
 */
import {
  createFile,
  deletePath,
  mkdir,
  renamePath,
  writeFile,
} from "../../api/workspace";
import { isKnowledgeKey, normalizeKey } from "./markers";
import { KNOWLEDGE_ROOT, renderKnowledgeMarkdown } from "./document";
import type { KnowledgeNode } from "./types";

/** Gitignored copy under `.litecode/knowledge`. */
export const KNOWLEDGE_PRIVATE_ROOT = KNOWLEDGE_ROOT;

/** Workspace-root copy. Git can track it. */
export const KNOWLEDGE_PUBLIC_ROOT = "knowledge";

export type { KnowledgeVisibility } from "./snapshot";

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
