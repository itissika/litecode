<!-- agent未经允许不得擅自更改 -->

# LiteCode Agent 契约

> 本文件是 LiteCode 项目的工作契约，已合并原 `CLAUDE.md`。凡在本仓库工作的 AI Agent，必须遵守本文件与 `README.md`。

## 项目定位

LiteCode 是一个针对 coding 场景的 Agent 框架，核心信条：**把 Tool 和 Context 两件事做到极致**。项目契约与系统提示词的设计优先于一切。功能、字段、描述、逻辑均需用心设计，不追求最新概念。详见 `README.md`。

## 提交铁律

所有提交无一例外必须遵守以下规则。

### 1. 提交信息规范（Conventional Commits）

格式：`type(scope): 描述`

- **type** 必选，仅限：`feat` / `fix` / `refactor` / `docs` / `chore` / `test` / `style` / `perf` / `build` / `ci`
- **scope** 可选，标注影响模块（如 `runtime`、`web`、`lsp`、`tool`、`context_pipeline`、`session`）
- **描述**：祈使语气，说清"做了什么、为什么"；中英文均可，但单条提交内保持一致

### 2. 原子提交

- 一次提交只做一件事，不混入无关改动
- 不夹带调试代码、临时文件、本地配置

### 3. 提交前自检

- 必须能通过编译与现有测试（`cargo check`、`cargo test`）
- 用 `git status` / `git diff --stat` 复核，只提交与本次改动相关的文件

### 4. 安全红线（绝对禁止）

- 禁止提交任何密钥、token、口令、真实邮箱、内部地址（`.env`、`*.pem`、`*.key` 等）
- 禁止提交大文件或编译产物（模型权重、二进制）；需分发时走外部下载或 Git LFS，且须事先说明

### 5. 分支与推送

- 禁止 force-push 到 `main` / `master`
- 未经确认，禁止推送到共享或公开远程

### 提交信息示例

```
feat(runtime): 添加 bash 后台任务退出时的空闲自动回合机制
fix(web): 修复流式输出时列表跳动与滚动漂移
refactor(lsp): 应用 rustfmt 格式化并启用系统代理
chore: 清理项目并开源准备
```
