# WP-190 拣回方案（短）

> owner：tools_steward · approver：gm  
> 基线：`main@838b6ed` · 目标分支：`ops/wp-190-ops-usability`  
> 原则：只拣回 ops 易用度实现；**不碰**当前 WP-189（切 codec 分析）；**不 drop** stash；不破坏性 reset / 不删 objects；合 main 等 GM/Andy 点头。

## 1. 目标

把原「WP-189 ops 易用度」（曾 done、tools_version=24：org 树 / charter / request·handle·forward / 中文报错）从 stash/临时对象拣到独立分支，恢复到可合入状态。

## 2. 线索（只读，勿 drop）

| 对象 | 用途 |
|---|---|
| `stash@{0}` / `b28b6fa` | 主实现：`wp.mjs`/`_lib.mjs`/`wp.test.mjs`/`registry.json`(tv=24)、`board.schema.json`、`validate.mjs` 等 |
| `102f37f` | 原验收产物 + `context/constitution.md`（迁入 **WP-190** artifacts，不写回 WP-189） |
| `stash@{1}` | 仅 board/log 碎片；**默认不整包应用**（易污染现看板） |

## 3. 步骤

1. `git fetch` 后从 `838b6ed` 建分支 `ops/wp-190-ops-usability`（不切丢他人工作：若当前脏树，先只读核对；用 `git worktree` 或干净 checkout 该分支，避免覆盖 WP-189 分析产物）。
2. **工具文件整文件拣回**（来自 `stash@{0}` / `b28b6fa`）：  
   `.litecode/ops/tools/{wp.mjs,_lib.mjs,wp.test.mjs,registry.json,board.md}`、`board.schema.json`、`validate.mjs`（及同 diff 内必要配套）。
3. **board.json 手术合并**（禁止 `checkout stash -- board.json`）：  
   - 保留现看板 WP-189 分析包及后续条目；  
   - 仅并入 `org_nodes` / `ops_config` / agents 的 `node|superior|subordinates`；  
   - 不把旧「ops 易用度」伪装成 WP-189；说明写进本包 `recovery.md`。
4. 从 `102f37f` 拷贝原 acceptance/demo/plan 到 `.litecode/ops/artifacts/WP-190/`（可加前缀 `recovered-`）；补 `constitution.md` 若缺。
5. 跑：`node --test .litecode/ops/tools/wp.test.mjs`、`node .litecode/ops/tools/validate_all.mjs`；抽查 `org-tree` / `role-charter` / `request` 帮助输出。
6. 写 `artifacts/WP-190/recovery.md`（命令、来源对象、与 main 差异、未应用的 stash 部分）。
7. 在独立分支上 **commit**（仅本分支；**不** push main、不自动合入）。deliver 口径：分支可合 + 测试绿 + recovery.md。

## 4. handle 口径（拣回时保持）

无 flag → 审批人自留；`--owner` 指定；`--intake` 才用 `intake_agent`（仅 ops/工具类）。

## 5. 验收（done_when）

- 独立分支含易用度实现且 `tools_version≥24`
- `org-tree` / `org-sync` / `role-charter` / `request` 可用
- `wp.test` + `validate_all` 绿
- WP-189 分析产物未被改写
- `recovery.md` 齐；合 main 等明示批准

## 6. 不做

drop stash；破坏性 reset；删 `.git/objects`；覆盖 `.litecode/ops/artifacts/WP-189/`；未批准合 main。
