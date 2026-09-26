# `.litecode/` 工作区运行时目录

本文件由 LiteCode 在打开工作区时**自动生成并覆盖**，请勿改内容。升级产品后以新版本为准。

`.litecode/` 是本仓库的运行时数据，不是源码。建议写入 `.gitignore`。索引和快照始终跳过本目录。`grep` / `glob` 默认也硬跳过嵌套 `.litecode/`（不在 `excludes.json` 里，改排除列表无效）。

要搜这里的磁盘文件：

- `glob`：把 `path` 指到 `.litecode` 或其子目录，或这次调用 `no_ignore`。
- `grep`：把 `path` 指到**具体文件**（例如 `.litecode/excludes.json`），或 `no_ignore=true`。只指到 `.litecode` 目录会被拒绝。

项目契约在仓库根。打开工作区只种子 `CLAUDE.md`；`AGENTS.md` 有则读，不会自动建。不要把契约写进本目录。

下面分两类：**可改配置**（干什么、怎么改、格式、怎么生效、怎么验证、下一步）和**只读**（能看，不要动手）。未出现的文件是正常的（第一次用到才建），不要提前建空壳。
