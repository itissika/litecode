### `provider-catalog.toml`

本文件在全局数据目录，和全局数据库放在一起，不在 `.litecode/`。LiteCode 只在第一次种下，之后不改写。你可以自己改。进程启动时读一次，改完要重启当前 LiteCode 才生效。

**怎么看。** `litecode_workspace` 面板的 `provider config`，以及 `refresh` / `refresh provider`，读的是同一份结论：

- `seed: configured`：本进程已加载的 catalog 含有本构建 seed 里的 provider 和 model。
- `seed: outdated`：文件能通过校验，但仍缺条目。列出的是文件里缺的（provider 带 endpoint、endpoint_type、auth；model 写 `provider/model`）。
- `seed: need restart`：文件已经有这些条目，当前进程还没读。没有缺失列表。`next` 是让你告诉人类重启当前 LiteCode，重启后才生效。不要再改文件，也不要再去读 guide。
- `rejected`：文件校验失败。后面是校验错误原文。按 `next` 修文件，再跑 `refresh provider`。

用户多写的 provider、改过的 endpoint 不会变成 outdated。

**怎么更新。** 不要覆盖整份文件。先问人要不要补。同意后只把面板上缺的条目补进现有文件，保留原有修改。补齐后 `status` 和 `refresh` 都从 `outdated` 变成 `need restart`。这个工具不写该文件，也没有自动合并。

**怎么验证。** `litecode_workspace refresh provider`（`refresh` 和 `refresh all` 也会检查这一节）。校验的是 TOML 和 catalog 规则（版本、provider、model、重复 id）。失败时输出 `rejected` 和错误原文，不会把还没读入进程的缺失列表再列一遍。

**下一步。** `outdated` 时先问人，再补缺的条目。`need restart` 时只告诉人重启当前 LiteCode。`rejected` 时修文件后再 `refresh provider`。
