# WP-190 — 拣回原 ops 易用度实现

## 背景
原「WP-189 ops 易用度」曾验收 done（tools_version=24：org 树、charter、request/handle/forward、中文报错），但**未 commit/push**。同步 main 时工作区进了 stash；当前看板 WP-189 已被复用为「切 codec 分析」包。

## 线索（勿 drop stash）
- `stash@{0}` / `b28b6fa`：wip-before-sync-main-2026-10-17（含 registry tools_version=24）
- `102f37f`：相关 untracked/acceptance 侧
- `stash@{1}`：wp189-wip-board

## 要求
1. 从上述对象**拣回**到独立分支（建议 `ops/wp-190-ops-usability`），基于 `main` @ `838b6ed`。
2. **不**改写、不覆盖 `.litecode/ops/artifacts/WP-189/`（分析包）。
3. 恢复后：`tools_version≥24`；`org-tree` / `org-sync` / `role-charter` / `request` 可用；`wp.test` + `validate_all` 绿。
4. 写 `artifacts/WP-190/recovery.md` 说明拣回步骤与差异。
5. 先 fill/submit 短方案；合入 main 等 GM/Andy 点头，本包以「分支可合」为 deliver。

禁止：破坏性 reset、删 objects、drop 他人 stash。
