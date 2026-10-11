import { knowledgeFromFiles } from "./document";
import type { KnowledgeIssue } from "./types";
import { knowledgeCorpusFiles } from "./corpus";

const indexed = knowledgeFromFiles(knowledgeCorpusFiles);

export const knowledgeFixture = indexed.nodes;
export const knowledgeFolderFixture = indexed.folders;

/**
 * What the backend snapshot reports for this corpus: the three planted faults.
 * Verdicts are the backend's (`docs/knowledge-references.md`); tests and
 * previews use this instead of judging locally.
 */
export const knowledgeFixtureIssues: KnowledgeIssue[] = [
  {
    nodeId: "broken-marker",
    severity: "error",
    code: "dangling_relation",
    message: 'Citation "not-a-node" does not exist.',
    ref: "not-a-node",
    refKey: "node:not-a-node",
    args: { key: "not-a-node" },
  },
  {
    nodeId: "loopback",
    severity: "error",
    code: "self_relation",
    message: "Citation points at itself.",
    ref: "loopback",
    refKey: "node:loopback",
    args: { key: "loopback" },
  },
  {
    nodeId: "sampling",
    severity: "warning",
    code: "inactive_target",
    message: 'Cites disabled "temperature".',
    ref: "temperature",
    refKey: "node:temperature",
    args: { key: "temperature", target_status: "disabled" },
  },
];
