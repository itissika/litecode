# Batch 2 · ide-substrate 目录清单

- 负责人：IDE 基座（ide-substrate）
- 工作区 HEAD：`075308b`（`git rev-parse --short HEAD`）；分支 `ops/team-framework`
- 作用域：`src/workspace` `src/engines` `src/lsp` `src/terminal` `src/ide_base.rs` `src/proc_lifetime.rs`
- 基线哈希（节点正文）：`075308b`（当前 HEAD；admission 文档存量基线仍为 `e583ff2`）
- 本轮：**未 commit / 未 push**

## 1. 拟新增节点

| key | 路径 | 上层 | A# | 一句话承诺 |
|---|---|---|---|---|
| workspace-sandbox | knowledge/contracts/workspace-sandbox.md | workspace-shared-substrate | A6 | 工作区路径唯一安全门：经 `Sandbox::resolve`，越界即 Escape |
| workspace-watcher | knowledge/contracts/workspace-watcher.md | workspace-shared-substrate | A1/A7 | 一工作区唯一 notify 所有者；排除源端硬切，非根热切换 |
| terminal-pty | knowledge/contracts/terminal-pty.md | workspace-shared-substrate | A1/A7 | 只许一套 PTY 基座（TerminalHub + PtySession）；死亡清单禁第二套 |
| lsp-hub-lifecycle | knowledge/contracts/lsp-hub-lifecycle.md | engine-lifecycle | A2/A4 | activate 只登记；按需 spawn；stop 清空并 shutdown 全部 LS |
| process-lifetime | knowledge/contracts/process-lifetime.md | engine-lifecycle | A5/A2 | 引擎子进程绑定宿主寿命；Stopped/崩溃后不得残留 worker/LS |

均为 `status: pending`，`domain: ide-substrate`，符号级证据 + `基线：075308b`。

**刻意不建（避免与主节点主张重复）：**
- path_index / code_search stop 语义 → 引用 [@ key="engine-lifecycle"]（Stopped 即无运行实体 + 执行门）
- 检索词法腿/真源 → 已有 [@ key="retrieval-contract"]
- 工作区根不可热切换 → 已含于 [@ key="workspace-shared-substrate"]「一进程一工作区」；死亡清单误报记 ISSUE-007，不单开节点

## 2. 拟保留 / 轻改的存量节点

**保留（不改主张意思）：**
- `harness/workspace-shared-substrate`（enabled）
- `contracts/engine-lifecycle`（enabled）
- `contracts/retrieval-contract`（enabled）
- `design/engine-design`（enabled）

**本轮轻改（domain / 向下链接 / 措辞打磨；已 append_log）：**
| 节点 | 改动 |
|---|---|
| workspace-shared-substrate | 补 `domain: ide-substrate`；补 `向下展开` → workspace-sandbox / workspace-watcher / terminal-pty；补 `基线：075308b` |
| engine-lifecycle | 补 `domain`；补 `契约展开` → lsp-hub-lifecycle / process-lifetime；补 `基线` |
| retrieval-contract | 补 `domain`；补 `基线` |
| engine-design | 补 `domain`；`关键契约：` → `契约展开：`（消 tag_unknown，链接不变）；补 `基线` |

**已知存量 warning（保持原状，不为消 warning 改双上层）：**
- `parent_multiple`：workspace-shared-substrate、engine-lifecycle、retrieval-contract（上层各 2 个）
- `inactive_target`：enabled 父节点指向本轮 pending 子节点（预期，Batch 3 升降后再消）

## 3. 疑似主节点重复项

| 对 | 说明 | 判定 |
|---|---|---|
| workspace-sandbox ↔ workspace-shared-substrate | 共享 sandbox.rs 文件级/符号证据 | **非重复**。substrate 写清单级主张；sandbox 钉 A6 唯一安全门与 resolve 原语 |
| workspace-watcher ↔ workspace-shared-substrate | 共享 watcher 主题 | **非重复**。substrate 写「唯一监听者」；watcher 钉 sole notify owner + 排除≠热切换 |
| terminal-pty ↔ workspace-shared-substrate | 共享终端主题 | **非重复**。substrate 写「同一 PTY 基座」；terminal-pty 钉实现边界与死亡清单 |
| lsp-hub-lifecycle ↔ engine-lifecycle | LSP 寿命 vs 引擎状态机 | **非重复**。engine-lifecycle 管 desired/Warm/Stopped；lsp-hub 管 activate/lazy-spawn/shutdown |
| process-lifetime ↔ engine-lifecycle | stop 语义 | **非重复**。lifecycle 管状态迁移；process-lifetime 管子进程绑定与残留禁令 |
| 与 retrieval-contract | 无共享符号主张 | 无 dup |

kb_check 对本域新节点无 `dup_primary_suspect`；全库 dup 警告仍主要在 `contracts/tools/*`（他域）。

## 4. 本域 issue 草稿（含种子映射；Batch 3 再落盘）

| id | grade | layer | core | 对应节点 | 摘要 |
|---|---|---|---|---|---|
| ISSUE-002 | drift | gate | 是（retrieval-contract） | retrieval-contract | 词法腿测试仍断言 2 参 `lexical_search_ripgrep(query, preset)`，实现已 4 参；不变量（不接索引加速）仍成立 |
| ISSUE-007 | drift | gate | 否 | workspace-shared-substrate、workspace-watcher | 热切换死亡清单子串 `reload_workspace` 误伤 `reload_workspace_excludes_from_disk` / `plan.reload_workspace` |
| ISSUE-009 | suspect | env | 是（engine-lifecycle） | engine-lifecycle、process-lifetime | Linux 下 code_search stop 后 worker 可能未结束；确认后升 bug，计入 B4 |
| ISSUE-010 | bug | product | 是（workspace-shared-substrate） | workspace-shared-substrate、workspace-sandbox | `copy_path` 先 `resolve` 跟随符号链接，顶层链接被复制为目标；不越界，不触发 B3 |
| ISSUE-004 | suspect | product | 否 | lsp-hub-lifecycle（旁听；工具输出整形或在 tools 域） | optional_lsp findReferences 缺上下文块；Batch 3 再定主责域 |

**新发现（本轮目录阶段，未升阻断）：** 无。未跑会触达 B1–B5 的新确认。

阻断：种子统计仍为 arch 0；核心 product 级 open drift+bug 仍为 ISSUE-003（他域）+ ISSUE-010 = 未达 B4/B5。**当前无阻断上报。**

## 5. kb_check 摘要

- 命令：`node .litecode/ops/tools/kb_check.mjs`（及 `--node <key>` 逐个）
- 全库：errors=**0**，warnings≈88（他域+存量提示；随并行 Batch 2 浮动）
- **本域 5 个新 pending：error=0，warning=0**（`--node` 逐个确认）
- 本域存量相关 warning（仅提示）：`parent_multiple`×3、`inactive_target`×5（enabled→pending 挂树）
- 无 filename_mismatch / missing_symbol / dangling_relation

## 6. 节点计数

| 项 | 数 |
|---|---|
| 本域存量 enabled | 4（均保留并补 domain） |
| 本域新增 pending | **5**（上表 keys） |
| 本域节点合计 | **9** |

## 7. 交付路径

- Artifact：`/workspace/litecode/.litecode/ops/artifacts/BATCH-2-ide-substrate.md`
- 新节点：`/workspace/litecode/knowledge/contracts/{workspace-sandbox,workspace-watcher,terminal-pty,lsp-hub-lifecycle,process-lifetime}.md`
