# WP-190 拣回说明

## 目标

将原「WP-189 ops 易用度」（曾验收 done、`tools_version=24`：org 树 / role-charter / request·handle·forward / 中文报错）从 stash/临时对象拣到独立分支 `ops/wp-190-ops-usability`，恢复到可合入状态。

## 基线与分支

| 项 | 值 |
|---|---|
| 基线 | `main@838b6ed` |
| 分支 | `ops/wp-190-ops-usability` |
| worktree | `/workspace/litecode-wp190` |
| 主仓 | `/workspace/litecode`（继续 main，WP-189 分析包不动） |

## 来源对象（只读，未 drop）

| 对象 | 用途 | 本包是否整包应用 |
|---|---|---|
| `stash@{0}` / `b28b6fa` | 主实现：`wp.mjs` / `_lib.mjs` / `wp.test.mjs` / `registry.json`(tv=24) / `board.md` / `board.schema.json` / `validate.mjs`；board 中 `org_nodes`/`ops_config`/agents 组织字段 | **工具文件整文件拣回**；board **手术合并** |
| `102f37f` | 原 acceptance/demo/plan/brief + `context/constitution.md` | 写入本包 `recovered-*` 与 `constitution.md`（**不**写回 WP-189） |
| `stash@{1}` | board/log 碎片 | **未应用**（易污染现看板） |

## 步骤（已执行）

1. `git worktree add /workspace/litecode-wp190 -b ops/wp-190-ops-usability 838b6ed`
2. `git checkout stash@{0} --` 下列文件（仅工具/校验，不含 board/events/log）：
   - `.litecode/ops/tools/{wp.mjs,_lib.mjs,wp.test.mjs,registry.json,board.md}`
   - `.litecode/ops/board.schema.json`
   - `.litecode/ops/validate.mjs`
3. **board.json 手术合并**：
   - 底：主仓 `/workspace/litecode/.litecode/ops/board.json`（保留 WP-189 分析包条目与 WP-190 状态）
   - 自 stash0 仅并入：`org_nodes`、`ops_config`、各 agent 的 `node` / `superior` / `subordinates`
   - **未** `checkout stash -- board.json`，**未**覆盖 `events/*` / `log.jsonl` / `tool_traces.jsonl`
4. 自 `102f37f` 写入：
   - `artifacts/WP-190/recovered-{acceptance,brief,plan,demo-transcript}.*`
   - `.litecode/ops/context/constitution.md`
5. 跑测：`node --test .litecode/ops/tools/wp.test.mjs`、`node .litecode/ops/tools/validate_all.mjs`；抽查 `org-tree` / `role-charter`
6. 在本分支 commit；**不合 main**、不 push main

## 与 main 差异（本分支相对 `838b6ed`）

- ops 工具集升到 `tools_version=24`（org-tree / org-sync / role-charter / request / handle / forward + 中文 CLI 报错）
- `board.schema.json` / `validate.mjs` 支持 `org_nodes` / `ops_config` / agent 组织字段
- board 含组织树与 `ops_config`（`intake_agent=tools_steward`，handle 默认自留 owner）
- 新增 `context/constitution.md` 与本包 recovered 验收材料

## 未应用内容

- `stash@{1}` 整包 board/log
- stash0 中的 `board.json` 全量、`events/inbox.json`、`events/outbox.jsonl`、`log.jsonl`、`tool_traces.jsonl`
- 任何对主仓 `.litecode/ops/artifacts/WP-189/` 的改写

## handle 口径（拣回保持）

无 flag → 审批人自留；`--owner` 指定；`--intake` 才用 `intake_agent`（仅 ops/工具类）。

## 合入

合 `main` 等 GM/Andy 明示批准；本包 deliver 口径为「分支可合 + 测试绿 + 本文件」。
