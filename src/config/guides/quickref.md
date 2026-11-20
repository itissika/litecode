## Agent 速查

1. 项目规则 → 仓库根 `CLAUDE.md` / `AGENTS.md`（后者不会自动建）。
2. 排除 / MCP / 自定义工具 → 「可改的配置」或设置页。MCP 与自定义工具写好定义后，等人在 Agents 里打开。
3. 待办 → `todo.write` 工具（在会话里，不在本目录）。
4. 旧会话 → `session_search`，路径是 `.litecode/sessions/<id>.md`。
5. 工作区状态 / 配置校验 → `workspace.help`、`workspace.refresh`、`workspace.status`：调 `workspace.help()` 看指引，`workspace.refresh()` 一次校验 `excludes.json`、`mcp.json`、`custom_tools.json` 与 `apps.json`，`workspace.status()` 看实时状态。MCP 与自定义工具在下一个 turn 开始时生效；列成 off for you 时，只有这次真的需要才去找人，否则不要提。校验失败时按错误去修。新工作区先读 `workspace.help({ topic: "setup" })` 走一遍（看、配排除、荐 App、给下一步）。
6. 不要删整个 `.litecode/`，也不要动「只读」表里的路径。
