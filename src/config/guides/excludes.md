### `excludes.json`

打开工作区时若不存在会按内置默认种子。

**干什么。** 工作区排除。三套 glob **不要并成一套**（对齐 VS Code：树 / 检索 / 监视器）：

- `files_exclude`：资源管理器当文件不存在；检索默认也不碰。
- `search_exclude`：资源管理器还能看见，但人的搜索、`grep` / `glob`、文本索引、语义索引默认不扫。生成物（`Cargo.lock`、`package-lock.json`、`*.min.js` 等）不想被检索时写在这里，不要靠引擎再滤一层语言表。
- `watcher_exclude`：监视器**源上硬切**，命中的路径不上变化总线（引擎和界面都收不到增量）。默认含 `**/.litecode/**`。**例外**（硬编码放行，否则设置无法热加载）：`excludes.json`、`mcp.json`、`custom_tools.json`；以及用于计划面板刷新的 `.litecode/plan/*.md`（索引仍硬跳过 `.litecode`）。不放行 `index/` 等产物，也不放行 `engines.json`。

另两个开关：`git_ignore`（检索是否尊重 `.gitignore`，默认 `true`）；`explorer_git_ignore`（资源管理器是否尊重 `.gitignore`，默认 `false`）。检索只认排除名单 + 是否用 ignore 文件；不另藏 hidden。本目录自身的硬跳不走这份列表。

**怎么改。** 设置页；或先读本文件再改。三个数组是**整表替换**，不是和默认合并——不要用下面的形状当整文件覆盖，否则会丢掉 `**/.litecode/**` 等内置项。目录写 `dir` 或 `**/dir`，不要写 `dir/`（`.gitignore` 的尾斜杠语义这里没有；保存时会去掉尾 `/`）。空行、`#` 行、重复 glob 会被丢掉。删掉本文件会在下次打开时重新种子。

**格式。** `version` 必须为 `1`（缺了整份不生效，保持上一份）。

```json
{
  "version": 1,
  "files_exclude": ["**/.git", "**/.DS_Store"],
  "search_exclude": ["**/node_modules"],
  "watcher_exclude": [".git/objects/**", "*.litecode-tmp*", "**/.litecode/**"],
  "git_ignore": true,
  "explorer_git_ignore": false
}
```

**怎么生效。** 监视器放行本文件，重载进程内列表（JSON 无效则保持上一份）。文本索引按新语料调和；已打开代码语义检索时会同步索引（对齐完成前 `code_search` 可能提示稍后再试）。改仓库里的 `.gitignore` 同样会调和索引，不经本文件。

**怎么验证。** `litecode_workspace refresh excludes` 报告已经生效的计数。文件坏了会让你去修。排除效果可直接用 `grep` / `glob` 验证。

**下一步。** 不需要找人：本文件立即生效（监视器也会自动重载）。语义检索 / LSP 是重型引擎，归人类在设置页 **Engines** 开关，本文件与 `litecode_workspace` 都不碰。
