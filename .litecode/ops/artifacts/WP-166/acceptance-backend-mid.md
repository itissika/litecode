# WP-166 后端中层验收笔记（2026-10-15）

## 编译门闩（IDE 交叉评审 F1）
- `mark_startup_settled` / `is_startup_settled` / `wait_startup_settled` 已不在 `cfg(test)` 下；`ephemeral_registry` 仍 `cfg(test)`。
- Session 重跑：`CARGO_BUILD_JOBS=1 cargo_locked.sh -t 3000 check --locked --bins` → EXIT=0（约 2m21s）。系统 `/usr/bin/g++` 可用。

## 行为不变测（验收项）
- 计入 WP-171 run7 全量 `cargo_test` 全绿（含 `client_protocol::ws::wp166_tests::*`：settled / gc deferred 等）。未要求本包再开一次全量。

## F2 取舍（后端中层认可）
- **不**加 settled 兜底超时。
- 明确语义：无任何客户端信号（list/new/agent/run / 约定的 HTTP 检索等）时，P2 session-index 维护与空会话 GC **不跑**；有首个信号后立即开始。
- 依据：无人使用时维护无可见收益；与 plan「先回 list 再回放 / 打开后延后」一致。代码注释已写 No timeout。

## 产物
- `impl.diff`、`fix_e0618.diff` 已在 artifacts/WP-166。

## 分支卫生（非阻断）
- 箱上当前检出 `ops/wp-172-actions-ci`（非 `ops/team-framework`）；合入时由执行方切回目标分支，不阻塞本包验收。
