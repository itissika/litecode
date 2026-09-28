### `provider-catalog.toml`

本文件在全局数据目录，和全局数据库放在一起，不在 `.litecode/`。LiteCode 第一次启动时种下。能通过当前版本校验的文件不会被改写。校验失败时，启动会升级这份文件：不认识的字段和枚举值丢掉；一条 provider 或 model 丢掉这些之后仍不合法，就用内置默认里同 id 的条目盖掉；整份仍然读不进去，就整份换成内置默认。改写前把原文存成旁边的 `provider-catalog.toml.bak`。你可以自己改。进程启动时读一次，改完要重启当前 LiteCode 才生效。

**怎么看。** `litecode_workspace` 面板的 `provider config`，以及 `refresh` / `refresh provider`，读的是同一份结论：

- `seed: configured`：本进程已加载的 catalog 含有本构建 seed 里的 provider 和 model。
- `seed: outdated`：文件能通过校验，但仍缺条目。列出的是文件里缺的（provider 带 endpoint、endpoint_type、auth；model 写 `provider/model`）。`seed` 打印本构建 seed 里、文件还没有的 `[[providers]]` 和 `[[models]]` 原文，块上方的注释一并带上。
- `seed: need restart`：文件已经有这些条目，当前进程还没读。没有缺失列表，这时也不打原文。`next` 是让你告诉人类重启当前 LiteCode，重启后才生效。不要再改文件，也不要再去读 guide。
- `rejected`：文件校验失败。后面是校验错误原文。按 `next` 修文件，再跑 `refresh provider`。下次启动时打不开的 catalog 会按上面的规则升级并留下 `.bak`。

用户多写的 provider、改过的 endpoint 不会变成 outdated。

**怎么更新。** 不要覆盖整份文件。先跑 `seed`，先问人要不要补。同意后只把打出来的这些块贴进现有文件，保留原有修改。补齐后 `status` 和 `refresh` 都从 `outdated` 变成 `need restart`。这个工具不写该文件，也不会把它们再打一遍。

**怎么验证。** `litecode_workspace refresh provider`（`refresh` 和 `refresh all` 也会检查这一节）。校验的是 TOML 和 catalog 规则（版本、provider、model、重复 id）。失败时输出 `rejected` 和错误原文，不会把还没读入进程的缺失列表再列一遍。

**下一步。** `outdated` 时先跑 `seed`。`need restart` 时只告诉人重启当前 LiteCode。`rejected` 时修文件后再 `refresh provider`。
