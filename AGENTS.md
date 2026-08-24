<!-- 未经允许不得改本文件 -->

# LiteCode Agent 契约

凡在本仓库工作的 Agent，须遵守本文件与 `README.md`。

## 定位

LiteCode 把 Tool 和 Context 做到极致。契约与系统提示词优先于新概念。

## 工程心智

内核只有一套真相：OpenAI Responses `Item`。适配器在边缘消化供应商差异。默认极轻，能力按需叠加；Engine 管生命周期，Tool 只消费。权限与敏感路径只收紧不放宽。取消是封口，不是丢弃。

- 做：改 tool / context 时用心设计字段与边界；IDE 能力优先供给 Agent
- 不做：给内核加第二套 Message / Stream 类型；让 Tool 拥有 LSP / 检索生命周期；为追新概念放宽权限或加常驻重量

## 提交

未经确认，不得提交、不得推送远端。提交须清洁聚焦、描述精简易懂、内容安全。

- 做：提交前检查梳理，一次一事；描述 `type(scope): 做什么`，单条内中英文一致
- 不做：擅自 commit / push / force-push / amend
- 不做：混入调试、测试残留、临时文件、本地配置
- 不做：混入密钥、真实邮箱、内部地址、大文件、编译产物

## 自检

自检服务于构建。提交前、构建前确认「能过构建」，再提交或进入构建。

- 做：过编译与构建门禁（Rust：`cargo check`；改 `web/`：`npx tsc -b`）
- 不做：带着编译/类型错误进入构建或提交
- 不做：用 vitest / 单测替代构建检查

## 发版

只有重要变更才公开新版本：严重缺陷、重大特性、大量体验优化等。用户按 Release 取包，不以 main 当已发版。

### 流程

1. 升并同步版本号（`Cargo.toml` 为准，含 `desktop/package.json` 等）。
2. **合并发版描述**：自上一已发布 tag 至当前，整理 `CHANGELOG.md` 中该版本的条目（见下）。
3. 本地 `package_local.ps1` 验证通过后，打 tag 并 **Publish GitHub Release**（触发 `windows-signed-release`）。
4. CI 只上传安装包并在 Release 正文 **末尾追加** 构件说明，不覆盖已有描述。

### 发版描述（`CHANGELOG.md` + Release 正文）

- **时机**：在最终决定触发云端正式发布时做一次合并更新；升版本号时只记一笔、不得把整版内容写成「最后一次提交」。
- **范围**：覆盖自上一已发布 tag 以来、对该版本用户可见的全部变更，不是单个 commit 的摘要。
- **结构**（顺序固定）：`### 新增` → `### 变更` → `### 修复`；无内容的节可省略。
- **文风**：精简清晰、面向用户；每条一句说清「做了什么 / 用户得到什么」。不写内部 refactor、命名、过渡实现、单测与类型修补除非用户可感知。
- **禁止**：元信息（「累计 N 个提交」）、组件/文件名堆砌、与上一版本重复的修复、发版时才写的打包细节冒充整版变更。
- **Release 正文**：以 `CHANGELOG` 对应版本节为准（可略润色）；Publish 前写入 GitHub Release。CI **append** 安装包与校验和说明，**不得**用 workflow 固定文案覆盖正文。

### 构建编排（均指向当前仓）

| 态 | 入口 | 触发 |
|----|------|------|
| 开发 | `serve.sh` / `serve_win.ps1` / `dev_win.ps1` | 手动 |
| 构建 nightly | `package_local.ps1` | 手动 |
| 构建 official | `windows-signed-release` → `package_win.ps1` | Release `published` 或 `workflow_dispatch` |

- 做：先升版本号并整理好 `CHANGELOG`，再构建，再 Publish
- 不做：仅为 CI、文档、内部重构发版
- 不做：未升版本号就构建发布；擅自改版本号、打 tag、Publish、force 移动已有 tag
