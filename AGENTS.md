# LiteCode Agent 契约

LiteCode 把 Tool 和 Context 做到极致。本文件只放：底线、怎么干活、去哪读。细节在 `docs/agents/` 与 `knowledge/`。

## 底线

- 未经确认不 commit / push / tag / Publish，不动版本号。
- 权限与敏感路径只收紧不放宽；不给内核加第二套 Message / Stream。
- 交付前必须过构建门禁，不带类型错误收尾。
- 改契约（本文件、`docs/`、`knowledge/`）要在回复里点名说明。

## 怎么干活

- 找代码第一跳 `code_search`（本仓常开）；符号、定义、引用、诊断用 LSP 工具；有范围了再 `grep` / `glob` / `read`。
- 改代码攒成一批：先把要改的片段连同锚点收齐，一次 `apply_edits` 吃掉多文件多块，再一次看全部结果。
- 拿不准就读代码，别猜。代码和文档打架，以代码为准，并说哪份过期。

## 提交

- `type(scope): 做什么`，一次一事，描述说清做了什么。
- 提交前 `git status` 复查：调试残留、`.tmp/`、`.litecode/`、日志不带进去。
- 不进：密钥、真实邮箱、内部地址、大文件、编译产物。
- 探针、复现脚本、截图放 `.tmp/`，不散在源码目录。

## 日常

- 先读对应契约，再动手；一次改一小块，改完就跑。
- 只做被要求的事，不顺手重构、不扩面。
- 单测不替代构建；构建过了才算能交付。

## 验证

- Rust：`cargo check`；行为有变再 `cargo test`
- `web/`：`npx tsc -b`
- `desktop/`：`npm run build`

## 路由

先看一眼再动手。够不到的地方读代码——路由是提示，不是门槛。

| 要做的事 | 读 | 触发词 |
|---|---|---|
| 开工没头绪 / 定验收 / 判收敛 / 记经验 | `docs/agents/workflow.md` | 新功能、修 bug、验收 |
| 发版 | `docs/agents/release.md` | 版本号、CHANGELOG、tag、Publish |
| 打包 / 跑起来 | `docs/agents/build.md` | dev、nightly、official、安装包 |
| 为什么这么设计 / 不变量 | `knowledge/`（先空调用 `knowledge` 看 board） | 契约、为什么、术语 |

## 边界

- 做：小的、可回退的改动；先读后改；改完自检。
- 问：动契约、动发布、动权限面、加依赖。
- 不做：为追新概念加常驻重量；留一次性产物。
