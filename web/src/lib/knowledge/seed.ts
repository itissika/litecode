import { renderKnowledgeMarkdown, type KnowledgeSourceFile } from "./document";

function ref(key: string): string {
  return `[[node : ${key}]]`;
}

const overview = "knowledge 概念概述";
const syntax = "knowledge 语法示例";
const rules = "knowledge 语法校验规则说明";
const agent = "knowledge agent 入门";

function file(
  path: string,
  key: string,
  summary: string,
  refs: string[],
  body: string,
): KnowledgeSourceFile {
  return {
    path,
    markdown: renderKnowledgeMarkdown({
      key,
      status: "enabled",
      summary,
      refs,
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
    [syntax],
    `知识库是工作区里的一组文件夹和 md。文件夹只表示嵌套，不参与引用。一个 md 是一个节点，身份是文件开头的 \`node :\` 声明，全库唯一。

节点里有一行摘要、若干 \`ref :\` 声明，然后是正文。正文要引用别人，先在本文件写 \`ref :\`，再写成 ${ref(syntax)}。

侧边树里拖动节点或文件夹，会改它们在磁盘上的位置，画布跟着按新的嵌套重排。右键可以新建或删除。

面板底部的 private / public 决定这套文件放在哪。private 在 \`.litecode/knowledge\`，不进 git。public 在工作区根的 \`knowledge/\`，可以被 git 跟踪。`,
  ),
  file(
    `knowledge入门/文件夹关系示例/${syntax}.md`,
    syntax,
    "声明、摘要、引用和正文写在同一个 md 里。",
    [overview],
    `这个节点放在「文件夹关系示例」里，上一级文件夹是「knowledge入门」。文件夹本身不能被 ${ref(overview)} 这样引用。

声明写在文件最前面，摘要和引用跟它在同一块里：

\`\`\`node
node : ${syntax}
status : enabled
summary : 一句人话
ref : ${overview}
\`\`\`

正文里的引用写成 ${ref(overview)}。\`[[seq]]\` 这种不带 \`node :\` 的括号只是普通文字。`,
  ),
  file(
    `${rules}.md`,
    rules,
    "声明要唯一，正文只能引用本文件已经声明的 ref。",
    [syntax],
    `声明 \`node :\` 在全库必须唯一。字母、数字、空格、\`_\` 和 \`-\` 可以用。空声明、重复声明、带斜杠的声明是错误。

\`ref :\` 必须指向另一个节点。指向自己、指向不存在的声明，是错误。正文里的 ${ref(syntax)} 如果没有先写在本文件的 \`ref :\` 里，也是错误。

声明了却没在正文用到，只是警告。启用的节点引用禁用或待审的节点，也是警告。文件名和声明不一致，是警告，不会挡住引用。`,
  ),
  file(
    `${agent}.md`,
    agent,
    "用 read 和 edit 改这些 md，就能和人一起维护知识库。",
    [overview, syntax, rules],
    `Agent 用工作区的 read 和 edit 改这些 md，不需要单独的知识库工具。

先读 ${ref(overview)}，确认文件夹、节点，以及文件在 private 还是 public。语法见 ${ref(syntax)}。改声明、摘要、引用和正文之前，先看 ${ref(rules)}。

改 \`node :\` 的值等于改名，要同时改其他文件里的 \`ref :\` 和 \`[[node : ]]\`。不要另起一个同名节点。删除文件之前，先确认没有别的节点还在引用它。`,
  ),
];
