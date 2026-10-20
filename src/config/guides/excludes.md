### `excludes.json`

打开工作区时若不存在会按内置默认种子。

**干什么。** 工作区排除，外加代码语义检索的收录后缀。三套 glob **不要并成一套**（对齐 VS Code：树 / 检索 / 监视器）。语义索引用另一份后缀白名单，不跟随 `search_exclude`。

- `files_exclude`：资源管理器当文件不存在；检索默认也不碰。
- `search_exclude`：资源管理器还能看见，但人的搜索和文本检索默认不扫。生成物（`Cargo.lock`、`package-lock.json`、`*.min.js` 等）不想被这些检索看见时写在这里。改 `search_exclude` 不会改变代码语义索引。
- `code_index_extensions`：代码语义检索要收录的后缀白名单，不是排除名单。漏写一个后缀，只是这类文件不进语义索引。空数组表示不收任何源文件。缺这个字段时用内置默认后缀，不会变成什么都索引。按文件名最后一段匹配（`Button.tsx` → `tsx`，`types.d.ts` → `ts`）；无后缀文件（`Makefile`、`Dockerfile`、`LICENSE`）不收。`node_modules` 与 `bower_components` 目录硬跳，即使白名单里有 `js`。`json`、`md`、`txt`、`lock`、`csv` 不在默认名单里。
- `watcher_exclude`：监视器**源上硬切**，命中的路径不上变化总线（引擎和界面都收不到增量）。默认含 `**/.litecode/**`。**例外**（硬编码放行，否则设置无法热加载）：`excludes.json`、`mcp.json`、`custom_tools.json`；以及用于计划面板刷新的 `.litecode/plan/*.md`（索引仍硬跳过 `.litecode`）。不放行 `index/` 等产物，也不放行 `engines.json`。

另两个开关：`git_ignore`（人的搜索、文本检索和代码语义检索是否尊重 `.gitignore`，默认 `true`）；`explorer_git_ignore`（资源管理器是否尊重 `.gitignore`，默认 `false`）。检索只认排除名单 + 是否用 ignore 文件；不另藏 hidden。本目录自身的硬跳不走这份列表。

**怎么改。** 设置页；或先读本文件再改。`files_exclude`、`search_exclude`、`watcher_exclude`、`code_index_extensions` 都是**整表替换**，不是和默认合并——不要用下面的形状当整文件覆盖，否则会丢掉 `**/.litecode/**` 等内置项，也会把语义收录后缀收成示例里的短名单。目录写 `dir` 或 `**/dir`，不要写 `dir/`（`.gitignore` 的尾斜杠语义这里没有；保存时会去掉尾 `/`）。后缀不要写成 glob：写 `rs`，不要写 `*.rs`（保存时会收成最后一段）。空行、`#` 行、重复项会被丢掉。删掉本文件会在下次打开时重新种子。

**格式。** `version` 必须为 `1`（缺了整份不生效，保持上一份）。

```json
{
  "version": 1,
  "files_exclude": ["**/.git", "**/.DS_Store"],
  "search_exclude": ["**/node_modules"],
  "watcher_exclude": [".git/objects/**", "*.litecode-tmp*", "**/.litecode/**"],
  "code_index_extensions": ["rs", "ts", "tsx", "py"],
  "git_ignore": true,
  "explorer_git_ignore": false
}
```

**怎么生效。** 监视器放行本文件，重载进程内列表（JSON 无效则保持上一份）。文本索引按 `search_exclude` 调和。已打开代码语义检索时，改 `code_index_extensions` 会按新名单调和：多出来的旧块删掉，新后缀的文件入队，不必整库重建。对齐完成前 `code_search` 可能提示稍后再试。改 `search_exclude` 不会改变代码语义索引。改仓库里的 `.gitignore` 同样会调和索引，不经本文件。

**怎么验证。** `litecode_workspace refresh excludes` 报告已经生效的计数，其中 `code_index_extensions` 是后缀个数。文件坏了会让你去修。语义语料只随收录后缀和 `git_ignore` 变。

**下一步。** 不需要找人：本文件立即生效（监视器也会自动重载）。语义检索 / LSP 是重型引擎，归人类在设置页 **Engines** 开关，本文件与 `litecode_workspace` 都不碰。
