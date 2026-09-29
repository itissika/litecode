import { renderKnowledgeMarkdown, type KnowledgeSourceFile } from "./document";
import { knowledgePreview, mentionSource } from "./markers";
import type { KnowledgeStatus } from "./types";

function ref(key: string): string {
  return mentionSource(key);
}

function file(
  path: string,
  key: string,
  status: KnowledgeStatus,
  body: string,
): KnowledgeSourceFile {
  return {
    path,
    markdown: renderKnowledgeMarkdown({
      key,
      status,
      body,
      summary: knowledgePreview(body, 1),
    }),
  };
}

/** Fixture corpus for tests. */
export const knowledgeCorpusFiles: KnowledgeSourceFile[] = [
  file(
    "内核/session.md",
    "session",
    "enabled",
    `以 ${ref("seq")} 单调递增为核心权威不变量的会话。已经落库的记录，只有 ${ref("revert")} 能改。`,
  ),
  file(
    "内核/seq.md",
    "seq",
    "enabled",
    `会话日志的序号，只增不改。它属于一条 ${ref("session")}，改动只能来自 ${ref("revert")}。`,
  ),
  file(
    "内核/revert.md",
    "revert",
    "enabled",
    `对已经落库的 ${ref("seq")} 做删改的唯一入口。`,
  ),
  file(
    "内核/workspace.md",
    "workspace",
    "enabled",
    `一个工作区目录。里面可以同时有多条 ${ref("session")}。`,
  ),
  file(
    "内核/上下文/item.md",
    "item",
    "enabled",
    `内核只有一套真相：OpenAI Responses 的 Item。一条 ${ref("session")} 的上下文由它组成。`,
  ),
  file(
    "内核/上下文/compact.md",
    "compact",
    "enabled",
    `把一条 ${ref("session")} 的早期上下文压成摘要。摘要本身仍是一条 ${ref("item")}。`,
  ),
  file(
    "内核/上下文/reminder.md",
    "reminder",
    "enabled",
    `请求缝上追加的提醒，挂在当前 ${ref("seq")}。${ref("compact")} 之后按种类决定要不要恢复。`,
  ),
  file(
    "内核/上下文/tool.md",
    "tool",
    "enabled",
    `Agent 可调用的能力。调用和结果都作为 ${ref("item")} 回到上下文。`,
  ),
  file(
    "内核/上下文/context.md",
    "context",
    "enabled",
    `发给模型前的视图：已压缩的 ${ref("compact")}、缝上的 ${ref("reminder")}，以及尚未封口的 ${ref("item")}。`,
  ),
  file(
    "供应商/provider.md",
    "provider",
    "enabled",
    `供应商目录。真正的协议差异由 ${ref("codec")} 在边缘消化。`,
  ),
  file(
    "供应商/codec.md",
    "codec",
    "enabled",
    `把 ${ref("item")} 编成 Responses 或 Chat Completions 的请求体。`,
  ),
  file(
    "供应商/wire.md",
    "wire",
    "enabled",
    `每次模型请求的原文。按 ${ref("session")} 分目录，格式由 ${ref("codec")} 决定。`,
  ),
  file(
    "供应商/采样/temperature.md",
    "temperature",
    "disabled",
    "采样温度。请求不再发送这个参数，节点保留为禁用。",
  ),
  file(
    "供应商/采样/sampling.md",
    "sampling",
    "enabled",
    `旧请求会带上 ${ref("temperature")}。目标已经禁用，启用节点引用它只给警告。`,
  ),
  file(
    "问题示例/broken-marker.md",
    "broken-marker",
    "enabled",
    `正文里有一个没有定义过的标识 ${ref("not-a-node")}，校验应该挡住它。\n\n\`\`\`\n${ref("seq")}\n\`\`\``,
  ),
  file(
    "问题示例/loopback.md",
    "loopback",
    "enabled",
    `正文引用了自己 ${ref("loopback")}。`,
  ),
  file(
    "知识库/knowledge.md",
    "knowledge",
    "pending",
    `人类主责、agent 协助的工作区知识库。节点之间是有向引用，稳定不变量的例子是 ${ref("session")}。`,
  ),
  file(
    "dockview.md",
    "dockview",
    "enabled",
    "工作台的面板布局。侧栏、编辑器和会话各自是一块面板。",
  ),
];
