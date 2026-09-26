## Agent 速查

1. 项目规则 → 仓库根 `CLAUDE.md` / `AGENTS.md`（后者不会自动建）。
2. 排除 / MCP / 自定义工具 → 「可改的配置」或设置页。MCP 与自定义工具写好定义后，等人在 Agents 里打开。
3. 待办 → `todo` 工具（在会话里，不在本目录）。
4. 旧会话 → `session_search`，再 `read` `.litecode/sessions/<id>.md`。
5. 工作区状态 / 配置校验 → `litecode_workspace`：空调用打开面板，`guide <topic>` 看单节，`refresh` 校验 excludes、MCP、自定义工具和 provider catalog。排除改对了就生效。MCP 与自定义工具在工作区没有 running session 时自动生效；列成 off for you 时，只有这次真的需要才去找人，否则不要提。provider 文件缺条目时是 outdated；补齐后是 need restart，告诉人重启当前 LiteCode，不要再改文件。校验失败时按 `rejected` 后面的错误去修。
6. 不要删整个 `.litecode/`，也不要动「只读」表里的路径。
