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
    "声明要唯一，引用的 id 必须指向另一个节点。",
    `\`node :\` 全库唯一。字母、数字、空格、\`_\`、\`-\` 可以用。空声明、重复、带斜杠是错误。

引用只有 ${ref(syntax)} 这一种：\`[@ id="声明" label="声明"]\`。查找只认 \`id\`。指向自己或不存在的声明是错误。引用禁用或待审节点、文件名和声明不一致，是警告。

代码块、行内代码和 \`@seq\` 不是引用。`,
  ),
  file(
    `${agent}.md`,
    agent,
    "用 read 和 edit 改这些 md，就能和人一起维护知识库。",
    `用 read 和 edit 改这些 md。先读 ${ref(overview)}，语法见 ${ref(syntax)}，规则见 ${ref(rules)}。

改 \`node :\` 等于改名，要同时改其他文件里的 \`id\`，以及与旧声明相同的 \`label\`。删除前先确认没有别的节点还在引用它。`,
  ),
];
