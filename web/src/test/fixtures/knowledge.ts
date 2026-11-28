import { knowledgeFromFiles } from "../../lib/knowledge/document";
import type { KnowledgeIssue } from "../../lib/knowledge/types";
import { knowledgeCorpusFiles } from "../../lib/knowledge/corpus";

const indexed = knowledgeFromFiles(knowledgeCorpusFiles);

export const knowledgeFixture = indexed.nodes;
export const knowledgeFolderFixture = indexed.folders;

/**
 * What the backend snapshot reports for this corpus: the three planted faults.
 * Verdicts are the backend's (`knowledge/contracts/knowledge/knowledge-ref-authority.md`);
 * tests and previews use this instead of judging locally.
 */
export const knowledgeFixtureIssues: KnowledgeIssue[] = [
  {
    nodeId: "broken-marker",
    severity: "error",
    code: "dangling_relation",
    message: 'Citation "knowledge/not-a-node.md" does not exist.',
    ref: "knowledge/not-a-node.md",
    refKey: "path:knowledge/not-a-node.md",
    args: { key: "not-a-node", path: "knowledge/not-a-node.md" },
  },
  {
    nodeId: "loopback",
    severity: "error",
    code: "self_relation",
    message: "Citation points at itself.",
    ref: "knowledge/诊断示例/loopback.md",
    refKey: "path:knowledge/诊断示例/loopback.md",
    args: { key: "loopback", path: "knowledge/诊断示例/loopback.md" },
  },
  {
    nodeId: "sampling",
    severity: "warning",
    code: "inactive_target",
    message: 'Cites disabled "temperature".',
    ref: "knowledge/模型与工具/temperature.md",
    refKey: "path:knowledge/模型与工具/temperature.md",
    args: {
      key: "temperature",
      path: "knowledge/模型与工具/temperature.md",
      target_status: "disabled",
    },
  },
];
