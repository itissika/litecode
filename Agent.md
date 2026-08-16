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

## Release 时机原则

发版是产品契约的一部分：用户按 **GitHub Release + 版本号** 取安装包，不是按 `main` 最新 commit。Agent 不得把「代码已进 main」当成「已经发版」。

### 1. 应当发新版本（并升版本号）

出现任一情况，应提议 Release 新版本（`vX.Y.Z`），而不是继续往旧 tag 上挂包或默默堆在 `main`：

- **特性升级**：用户可见的能力、默认行为、协议/API、安装包形态（含嵌入的 Linux server bundle）发生变化
- **破坏性变更**：配置、数据目录、鉴权、远程连接方式等与上一正式版不兼容
- **变更已积累成一批**：相对上一正式版已有多条 `feat` / `fix` / `perf`，或 `main` 与已发布 tag 明显偏离，再拖会让安装包与仓库脱节
- **安全或正确性修复**：影响已安装用户，应出补丁版（升 patch），不要只停在 commit 上

### 2. 不必为发版而发版

下列改动默认 **不** 单独发版，除非已与第 1 条叠成一批可交付物：

- 仅 CI / workflow / 脚本权限
- 仅文档、注释、changelog 草稿
- 未改变用户可感知行为的内部重构

此类改动推进 `main` 即可。已发布过的 tag 若需补打安装包（例如当时 CI 失败），用手动 `workflow_dispatch` 挂回该 tag，**不要**为此改版本号。

### 3. 发版时必须同步的版本号

版本权威来源是 `Cargo.toml` 的 `[package].version`。发版前同一数字必须写到：

- `Cargo.toml`
- `desktop/package.json`
- `CHANGELOG.md`（新增对应章节）
- Git tag：`v` + 版本号（如 `v0.1.4`），并 **Publish GitHub Release**

Windows 安装包 CI（`windows-signed-release.yml`）在 Release **published** 时自动跑；未 Publish 则不会出安装包。不要只 `git tag` 或只 push `main` 就当成已发版。

### 4. 版本号怎么升

- **major**：不兼容的重大变更
- **minor**：特性升级、可感知的能力增加（向后兼容）
- **patch**：修复、安全补丁、小改进

禁止把新功能或大批修复继续标成旧版本号对外分发。

### 5. Agent 权限

未经用户明确确认，禁止：改版本号、打 tag、Publish Release、force 移动已有 tag。需要发版时先说明相对上一 tag 的变更摘要与建议版本号，等确认后再做。
