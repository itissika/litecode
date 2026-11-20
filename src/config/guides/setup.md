## 首次上手

刚打开一个工作区，或人对这里还不熟时，按这四步走一遍；做完就停，不要扩成审计。

1. **看**：读根目录 `CLAUDE.md` / `AGENTS.md`，列一层顶层目录，看构建与依赖入口（如 `Cargo.toml`、`package.json`）。只看这些，不要全库遍历。
2. **配排除**：补这个工作区特有、默认没覆盖的依赖 / 产物 / 生成物目录，按 `workspace.help({ topic: "excludes" })` 写进 `.litecode/excludes.json`（整表替换，别丢默认项），再调 `workspace.refresh()` 确认。
3. **荐 App**：用 `workspace.status()` 与 `workspace.help({ topic: "apps" })` 看已配置应用与注册表规范，只挑 1–2 个真贴合的建议，安装、信任、启动留给人。
4. **给下一步**：按看到的东西列 1–3 件值得做的事（跑一次测试、读某个目录、开一个计划），一句话一件。

边界：只读探索，加上改 `excludes.json`；不建空壳文件，不动 MCP、自定义工具和 Apps 注册表，不替人开引擎。
