import { renderKnowledgeMarkdown, type KnowledgeSourceFile } from "./document";
import { mentionSource } from "./markers";

function ref(key: string): string {
  return mentionSource(key);
}

const overview = "knowledge 概念概述";
const syntax = "knowledge 语法示例";
const rules = "knowledge 语法校验规则说明";
const agent = "knowledge agent 入门";

function file(
  path: string,
  key: string,
  summary: string,
  body: string,
): KnowledgeSourceFile {
  return {
    path,
    markdown: renderKnowledgeMarkdown({
      key,
      status: "enabled",
      summary,
      body,
    }),
  };
}

/**
 * Written when a workspace has no knowledge directory yet.
 * The nodes are the human guide and the agent guide: read and edit these files
 * to collaborate.
 */
export const knowledgeSeedFiles: KnowledgeSourceFile[] = [
  file(
    `knowledge入门/文件夹关系/${overview}.md`,
    overview,
    "文件夹只负责嵌套，md 才是节点。",
    `知识库是工作区里的一组文件夹和 md。文件夹只表示嵌套。一个 md 是一个节点，身份是文件开头的 \`node :\` 声明，全库唯一。

正文引用写成 ${ref(syntax)}。

侧边树里拖动会改磁盘位置，画布按新的嵌套重排。右键可以新建或删除。private 在 \`.litecode/knowledge\`，不进 git。public 在工作区根的 \`knowledge/\`，可以被 git 跟踪。`,
  ),
  file(
    `knowledge入门/文件夹关系示例/${syntax}.md`,
    syntax,
    "声明和正文写在同一个 md 里。",
    `这个节点在「文件夹关系示例」里，上一级是「knowledge入门」。文件夹不能被 ${ref(overview)} 引用。

\`\`\`node
node : ${syntax}
status : enabled
summary : 一句人话
\`\`\`

正文里的引用是 ${ref(overview)}。\`@seq\` 不是引用。`,
  ),
  file(
    `${rules}.md`,
    rules,
    "A key is unique. A citation id must point at another node.",
    `\`node :\` is unique in the library. Letters, digits, spaces, \`_\`, and \`-\` are allowed. Consecutive spaces are not. An empty declaration, a duplicate, or a slash is an error.

The only citation is ${ref(syntax)}: \`[@ id="declaration" label="declaration"]\`. Lookup uses \`id\` only. A citation that points at itself or at a missing declaration is an error. Citing a disabled or pending node, or a filename that does not match the declaration, is a warning.

Status is only \`enabled\`, \`disabled\`, or \`pending\`.

Fenced code, inline code, and \`@seq\` are not citations.`,
  ),
  file(
    `${agent}.md`,
    agent,
    "Edit the body with read and edit. Create, rename, and check go through the knowledge tool.",
    `Edit the body with read and edit. Create, rename, and check go through the knowledge tool. Quote a key that contains spaces.

Read ${ref(overview)} first. Syntax is in ${ref(syntax)}. Rules are in ${ref(rules)}.

Deleting a node and enabling it are human actions in the knowledge panel. Do not delete the files yourself.`,
  ),
];
